use std::{
    env, fs,
    io::{self, IsTerminal, Read, Write},
};

use anyhow::{bail, ensure, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::{
    cards::{
        findings::{extract, FindingFormat},
        git_diff, load_cards, repo_root, resolve, validate_cards, Event, Limits, MatchInput,
        Payload, RepoFingerprint, Resolution,
    },
    cli::{
        args::{CardArgs, CardCommand, TriggerArgs},
        resolve_mirror_root,
    },
    config::{Config, DbOverrides},
};

pub(crate) fn findings(
    format: FindingFormat,
    input: Option<&Utf8Path>,
    pretty: bool,
) -> Result<()> {
    let text = if let Some(path) = input {
        fs::read_to_string(path).with_context(|| format!("read findings input {path}"))?
    } else {
        io::read_to_string(io::stdin().lock()).context("read findings from stdin")?
    };
    let findings = extract(&text, format)?;
    if pretty {
        let mut out = io::stdout().lock();
        serde_json::to_writer_pretty(&mut out, &findings)?;
        writeln!(out)?;
        Ok(())
    } else {
        print_json(&findings)
    }
}

pub(crate) fn fingerprint(cwd: &Utf8Path, json: bool) -> Result<()> {
    let fingerprint = RepoFingerprint::detect(cwd)?;
    if json {
        print_json(&fingerprint)
    } else {
        println!("{fingerprint:#?}");
        Ok(())
    }
}

pub(crate) fn trigger(
    args: TriggerArgs,
    config: &Utf8Path,
    mirror: Option<&Utf8PathBuf>,
) -> Result<()> {
    let detected = RepoFingerprint::detect(&args.cwd)?;
    let root = detected.root.clone();
    let mut payload = read_payload(args.payload_file.as_deref(), args.no_stdin)?;
    payload.extract_findings(args.event);
    let cards = load_cards(&packs_dir(args.packs_dir, config, mirror)?)?;
    let fp = payload.fingerprint(detected)?;
    let changes = args
        .base
        .as_deref()
        .map(|base| git_diff(&args.cwd, base))
        .transpose()?;
    let input = MatchInput::new(args.event, &payload, &root, &fp)?
        .with_changes(changes.as_deref().unwrap_or_default());
    let session = session(args.session);
    let out = resolve(
        &cards,
        &input,
        args.state_dir.as_deref(),
        &root,
        args.harness.trim(),
        session.as_deref(),
        &Limits {
            max_cards: args
                .max_cards
                .unwrap_or(if args.event == Event::Audit { 20 } else { 2 }),
            max_tokens: args.max_tokens,
        },
    )?;
    if args.json {
        print_json(&out)
    } else {
        print_resolution(out);
        Ok(())
    }
}

pub(crate) fn card(
    args: CardArgs,
    config: &Utf8Path,
    mirror: Option<&Utf8PathBuf>,
    db: &DbOverrides,
) -> Result<()> {
    let cards = load_cards(&packs_dir(args.packs_dir, config, mirror)?)?;
    match args.command {
        CardCommand::List { pack, json } => {
            let list: Vec<_> = cards
                .iter()
                .map(|c| &c.metadata)
                .filter(|c| pack.as_ref().is_none_or(|pack| c.pack == *pack))
                .collect();
            if json {
                print_json(&list)?;
            } else {
                for card in list {
                    println!("{}\t{:?}", card.id, card.tier);
                }
            }
        }
        CardCommand::Show {
            id,
            full,
            harness,
            session: sid,
            no_record,
        } => {
            let card = cards
                .iter()
                .find(|card| card.metadata.id == id)
                .with_context(|| format!("unknown card {id:?}"))?;
            if full {
                let path = card
                    .full_path()?
                    .with_context(|| format!("{id} has no full_ref"))?;
                let contents =
                    fs::read_to_string(&path).with_context(|| format!("read full_ref {path}"))?;
                io::stdout().lock().write_all(contents.as_bytes())?;
            } else {
                println!("{}", card.body);
            }
            if !no_record {
                let record = || -> Result<()> {
                    let root = repo_root(Utf8Path::new("."))?;
                    let db =
                        Config::load_database_or_default(config)?.resolve_db_with_overrides(db)?;
                    super::usage::record_card_read(
                        db,
                        &id,
                        harness.trim(),
                        session(sid).as_deref(),
                        root.as_str(),
                    )
                };
                if let Err(err) = record() {
                    eprintln!("skillnet: warning: failed to record card read: {err}");
                }
            }
        }
        CardCommand::Validate => {
            validate_cards(&cards)?;
            println!("validated {} cards", cards.len());
        }
    }
    Ok(())
}

fn packs_dir(
    explicit: Option<Utf8PathBuf>,
    config: &Utf8Path,
    mirror: Option<&Utf8PathBuf>,
) -> Result<Utf8PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(path) = env::var_os("AI_SKILLS_REPO").filter(|value| !value.is_empty()) {
        return Ok(Utf8PathBuf::try_from(std::path::PathBuf::from(path))?.join("packs"));
    }
    let config = Config::read(config)
        .context("configure a global skills source or set --packs-dir or AI_SKILLS_REPO")?;
    let global = config.global_canonical_path(&resolve_mirror_root(&config, mirror)?)?;
    ensure!(
        global.is_absolute(),
        "no absolute global skills source configured; set --packs-dir or AI_SKILLS_REPO"
    );
    global
        .parent()
        .map(|parent| parent.join("packs"))
        .with_context(|| format!("global skills path {global} has no parent; set --packs-dir"))
}

fn session(explicit: Option<String>) -> Option<String> {
    explicit
        .or_else(|| env::var("CLAUDE_SESSION_ID").ok())
        .and_then(|id| {
            let id = id.trim();
            (!id.is_empty()).then(|| id.to_owned())
        })
}

fn read_payload(file: Option<&Utf8Path>, no_stdin: bool) -> Result<Payload> {
    let text = if let Some(file) = file {
        fs::read_to_string(file).with_context(|| format!("read trigger payload {file}"))?
    } else {
        let stdin = io::stdin();
        if no_stdin || stdin.is_terminal() {
            return Ok(Payload::default());
        }
        let mut text = String::new();
        stdin
            .lock()
            .read_to_string(&mut text)
            .context("read trigger payload from stdin")?;
        if text.trim().is_empty() {
            return Ok(Payload::default());
        }
        text
    };
    let value: serde_json::Value =
        serde_json::from_str(&text).context("invalid trigger payload JSON")?;
    if !value.is_object() {
        bail!("trigger payload must be a JSON object");
    }
    serde_json::from_value(value).context("invalid trigger payload")
}

fn print_json(value: &impl Serialize) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}

fn print_resolution(out: Resolution<'_>) {
    if let Some(deny) = out.deny {
        println!("DENY {}\n{}", deny.card, deny.reason);
    }
    for card in out.inject {
        println!("{}\n{}", card.card, card.text);
    }
    for (card, count) in out.collapsed {
        println!("{card}: {count} repeat(s) suppressed");
    }
    if out.budget_dropped > 0 {
        println!("{} card(s) suppressed by budget", out.budget_dropped);
    }
}
