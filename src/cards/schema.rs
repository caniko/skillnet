use std::{collections::BTreeMap, fs, path::Component};

use anyhow::{bail, ensure, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use clap::ValueEnum;
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::RegexSet;
use serde::{Deserialize, Serialize};

use super::{
    engine::{MatchInput, Payload},
    expr::Expr,
    RepoFingerprint,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Event {
    SessionStart,
    PostEdit,
    PreTool,
    PrePush,
    Ci,
    Audit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Tier {
    Fix,
    Advise,
    Gate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Cooldown {
    Session,
    #[serde(rename = "repo-30d")]
    Repo30d,
    Always,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pack {
    name: String,
    #[serde(rename = "description")]
    _description: String,
    #[serde(default, rename = "languages")]
    _languages: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub id: String,
    pub pack: String,
    pub tier: Tier,
    pub cooldown: Cooldown,
    #[serde(default)]
    pub events: Vec<Event>,
    pub full_ref: Option<Utf8PathBuf>,
    fixture: Utf8PathBuf,
    negative_fixture: Option<Utf8PathBuf>,
    #[serde(default)]
    pub(crate) on: Triggers,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Triggers {
    pub diagnostic: Vec<String>,
    path: Vec<String>,
    not_path: Vec<String>,
    diff_regex: Vec<String>,
    pub tool: Vec<String>,
    command_regex: Vec<String>,
    not_command_regex: Vec<String>,
    repo_expr: String,
    pub signal: Vec<String>,
}

pub(crate) struct Card {
    pub metadata: Metadata,
    pub body: String,
    pub(super) path: GlobSet,
    pub(super) not_path: GlobSet,
    pub(super) diff: RegexSet,
    pub(super) command: RegexSet,
    pub(super) not_command: RegexSet,
    pub(super) repo_expr: Option<Expr>,
    file: Utf8PathBuf,
    pack_dir: Utf8PathBuf,
    source_dir: Utf8PathBuf,
}

impl Card {
    pub(crate) fn full_path(&self) -> Result<Option<Utf8PathBuf>> {
        // `full_ref` is relative to the card, not the pack directory.
        self.metadata
            .full_ref
            .as_ref()
            .map(|reference| {
                let path = self
                    .file
                    .parent()
                    .unwrap_or(Utf8Path::new(""))
                    .join(reference)
                    .canonicalize_utf8()
                    .with_context(|| format!("resolve full_ref {reference}"))?;
                ensure!(
                    path != self.source_dir && path.starts_with(&self.source_dir) && path.is_file(),
                    "full_ref must resolve to a file inside {}",
                    self.source_dir
                );
                Ok(path)
            })
            .transpose()
    }

    pub(super) fn tokens(&self) -> usize {
        self.body.len().div_ceil(4)
    }

    fn read(
        file: Utf8PathBuf,
        pack_dir: &Utf8Path,
        source_dir: &Utf8Path,
        pack: &str,
    ) -> Result<Self> {
        let text = fs::read_to_string(&file)?;
        let mut lines = text.lines();
        ensure!(
            lines.next() == Some("+++"),
            "missing opening `+++` TOML delimiter"
        );
        let front: Vec<_> = lines.by_ref().take_while(|line| *line != "+++").collect();
        let body = lines.collect::<Vec<_>>().join("\n").trim().to_owned();
        ensure!(
            !body.is_empty(),
            "missing closing `+++` delimiter or compact body"
        );
        let metadata: Metadata =
            toml::from_str(&front.join("\n")).context("invalid card frontmatter")?;
        ensure!(
            metadata.pack == pack,
            "card pack {:?} does not match {pack:?}",
            metadata.pack
        );
        ensure!(
            metadata.id.split_once('/').is_some_and(|(scope, id)| {
                scope == pack && !id.trim().is_empty() && !id.contains('/')
            }),
            "invalid card ID {:?}; expected {pack}/<name>",
            metadata.id
        );
        if let Some(reference) = &metadata.full_ref {
            ensure!(
                !reference.as_str().is_empty() && reference.is_relative(),
                "full_ref must be a relative path"
            );
        }
        let on = &metadata.on;
        for diagnostic in &on.diagnostic {
            let prefix = diagnostic.strip_suffix('*').unwrap_or(diagnostic);
            ensure!(
                !diagnostic.is_empty() && !prefix.contains('*'),
                "invalid diagnostic pattern {diagnostic:?}"
            );
        }
        Ok(Self {
            path: globs(&on.path).context("invalid on.path")?,
            not_path: globs(&on.not_path).context("invalid on.not_path")?,
            diff: RegexSet::new(&on.diff_regex).context("invalid on.diff_regex")?,
            command: RegexSet::new(&on.command_regex).context("invalid on.command_regex")?,
            not_command: RegexSet::new(&on.not_command_regex)
                .context("invalid on.not_command_regex")?,
            repo_expr: (!on.repo_expr.trim().is_empty())
                .then(|| Expr::compile(&on.repo_expr))
                .transpose()
                .context("invalid on.repo_expr")?,
            file,
            pack_dir: pack_dir.to_owned(),
            source_dir: source_dir.to_owned(),
            metadata,
            body,
        })
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.tokens() <= 160,
            "compact body is {} estimated tokens; limit is 160",
            self.tokens()
        );
        self.full_path()?;
        self.check_fixture(&self.metadata.fixture, true)?;
        if let Some(fixture) = &self.metadata.negative_fixture {
            self.check_fixture(fixture, false)?;
        }
        Ok(())
    }

    fn check_fixture(&self, reference: &Utf8Path, positive: bool) -> Result<()> {
        let kind = if positive { "positive" } else { "negative" };
        let check = || -> Result<()> {
            ensure!(
                reference
                    .as_std_path()
                    .components()
                    .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
                "fixture must be relative to the pack directory"
            );
            let path = self.pack_dir.join(reference).canonicalize_utf8()?;
            ensure!(
                path.starts_with(self.pack_dir.canonicalize_utf8()?) && path.is_file(),
                "fixture is not a file inside the pack directory"
            );
            let fixture: Fixture = serde_json::from_slice(&fs::read(path)?)?;
            let payload = fixture.payload.unwrap_or(fixture.flat);
            let fp = payload.fingerprint(RepoFingerprint::default())?;
            let input = MatchInput::new(fixture.event, &payload, &fp.root, &fp)?;
            ensure!(
                input.matches(self) == positive,
                "fixture must {}fire the card",
                if positive { "" } else { "not " }
            );
            Ok(())
        };
        check().with_context(|| format!("{kind} fixture {reference}"))
    }
}

#[derive(Deserialize)]
struct Fixture {
    event: Event,
    payload: Option<Payload>,
    #[serde(flatten)]
    flat: Payload,
}

fn globs(patterns: &[String]) -> Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        ensure!(!pattern.is_empty(), "empty path glob");
        set.add(Glob::new(pattern)?);
    }
    Ok(set.build()?)
}

pub(crate) fn load_cards(root: &Utf8Path) -> Result<Vec<Card>> {
    let absolute = Utf8PathBuf::try_from(std::path::absolute(root)?)?;
    let source_dir = absolute
        .parent()
        .context("packs directory has no parent")?
        .canonicalize_utf8()
        .with_context(|| format!("resolve source directory for {root}"))?;
    let root = root
        .canonicalize_utf8()
        .with_context(|| format!("resolve packs directory {root}"))?;
    let mut cards = BTreeMap::new();
    for pack_dir in entries(&root)? {
        if !pack_dir.is_dir()
            || pack_dir
                .file_name()
                .is_some_and(|name| name.starts_with('.'))
        {
            continue;
        }
        let manifest = pack_dir.join("pack.toml");
        let text = fs::read_to_string(&manifest).with_context(|| format!("read {manifest}"))?;
        let pack: Pack =
            toml::from_str(&text).with_context(|| format!("invalid pack {manifest}"))?;
        ensure!(
            Some(pack.name.as_str()) == pack_dir.file_name(),
            "pack name {:?} does not match its directory {pack_dir}",
            pack.name
        );
        for file in entries(&pack_dir.join("cards"))? {
            if file.extension() != Some("md") || !file.is_file() {
                continue;
            }
            let card = Card::read(file.clone(), &pack_dir, &source_dir, &pack.name)
                .with_context(|| format!("invalid card {file}"))?;
            let id = card.metadata.id.clone();
            if let Some(previous) = cards.insert(id.clone(), card) {
                bail!("duplicate card ID {id} in {} and {file}", previous.file);
            }
        }
    }
    Ok(cards.into_values().collect())
}

pub(crate) fn validate_cards(cards: &[Card]) -> Result<()> {
    let errors: Vec<_> = cards
        .iter()
        .filter_map(|card| {
            card.validate()
                .err()
                .map(|err| format!("{} ({}): {err:#}", card.metadata.id, card.file))
        })
        .collect();
    ensure!(
        errors.is_empty(),
        "card validation failed:\n{}",
        errors.join("\n")
    );
    Ok(())
}

fn entries(path: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let read = || -> Result<Vec<Utf8PathBuf>> {
        let mut entries = fs::read_dir(path)?
            .map(|entry| Ok(Utf8PathBuf::try_from(entry?.path())?))
            .collect::<Result<Vec<_>>>()?;
        entries.sort_unstable();
        Ok(entries)
    };
    read().with_context(|| format!("read directory {path}"))
}
