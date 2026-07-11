use std::{
    collections::BTreeMap,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context as AnyhowContext, Result};
use serde::Serialize;

use crate::{
    calibration::{db::DbParam, Db},
    cli::args::{UsageArgs, UsageCommand, UsageFormat},
    commands::Context,
    config::DbTarget,
};

#[derive(Debug, Serialize)]
struct UsageReportRow {
    skill: String,
    uses: i64,
    sessions: i64,
    first_seen: Option<String>,
    last_seen: Option<String>,
    harnesses: BTreeMap<String, i64>,
}

pub fn run(args: UsageArgs, target: DbTarget, context: Option<&Context>) -> Result<()> {
    match args.command {
        UsageCommand::Record {
            skill,
            harness,
            session,
            project,
            event_id,
            outcome,
            adapter_version,
        } => record(
            target,
            UsageEvent {
                skill,
                harness,
                session,
                project,
                event_id,
                outcome,
                adapter_version,
            },
        ),
        UsageCommand::Report { since, format } => {
            let context = context.context("usage report requires a loaded skillnet context")?;
            report(target, context, since.as_deref(), format)
        }
    }
}

struct UsageEvent {
    skill: String,
    harness: String,
    session: String,
    project: Option<String>,
    event_id: String,
    outcome: String,
    adapter_version: String,
}

fn record(target: DbTarget, event: UsageEvent) -> Result<()> {
    for (label, value) in [
        ("skill", event.skill.as_str()),
        ("harness", event.harness.as_str()),
        ("session", event.session.as_str()),
        ("event-id", event.event_id.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("{label} must not be empty");
        }
    }
    let db = open_db(target)?;
    db.execute(
        "INSERT INTO skill_invocations (
            session_id, skill_name, tool_name, project_dir, started_at, ended_at,
            outcome, plan_id, payload, hook_event, harness, source_event_id,
            adapter_version, canonical_skill_name
        ) VALUES ($1, $2, 'skillnet-usage', $3, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP,
                  $4, NULL, $5, 'SkillActivated', $6, $7, $8, $2)
         ON CONFLICT DO NOTHING",
        &[
            DbParam::from(event.session.as_str()),
            DbParam::from(event.skill.as_str()),
            DbParam::nullable_text(event.project.as_deref()),
            DbParam::from(event.outcome.as_str()),
            DbParam::from("{}"),
            DbParam::from(event.harness.as_str()),
            DbParam::from(event.event_id.as_str()),
            DbParam::from(event.adapter_version.as_str()),
        ],
    )?;
    println!("recorded");
    Ok(())
}

fn report(
    target: DbTarget,
    context: &Context,
    since: Option<&str>,
    format: UsageFormat,
) -> Result<()> {
    let since_unix = since.map(parse_duration).transpose()?;
    let db = open_db(target)?;
    let aggregates = db.usage_aggregate(since_unix)?;
    let skills = current_skills(context)?;
    let mut rows: BTreeMap<String, UsageReportRow> = skills
        .into_iter()
        .map(|skill| {
            (
                skill.clone(),
                UsageReportRow {
                    skill,
                    uses: 0,
                    sessions: 0,
                    first_seen: None,
                    last_seen: None,
                    harnesses: BTreeMap::new(),
                },
            )
        })
        .collect();

    for aggregate in aggregates {
        let skill = normalize_skill_name(&aggregate.skill_name, &rows);
        let row = rows.entry(skill.clone()).or_insert_with(|| UsageReportRow {
            skill,
            uses: 0,
            sessions: 0,
            first_seen: None,
            last_seen: None,
            harnesses: BTreeMap::new(),
        });
        row.uses += aggregate.uses;
        row.sessions += aggregate.sessions;
        row.first_seen = min_seen(row.first_seen.take(), Some(aggregate.first_seen));
        row.last_seen = max_seen(row.last_seen.take(), Some(aggregate.last_seen));
        *row.harnesses.entry(aggregate.harness).or_default() += aggregate.uses;
    }

    let rows: Vec<_> = rows.into_values().collect();
    match format {
        UsageFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        UsageFormat::Table => {
            println!("skill\tuses\tsessions\tharnesses\tfirst_seen\tlast_seen");
            for row in rows {
                let harnesses = row
                    .harnesses
                    .into_iter()
                    .map(|(name, count)| format!("{name}:{count}"))
                    .collect::<Vec<_>>()
                    .join(",");
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    row.skill,
                    row.uses,
                    row.sessions,
                    harnesses,
                    row.first_seen.unwrap_or_default(),
                    row.last_seen.unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

fn current_skills(context: &Context) -> Result<Vec<String>> {
    let mut skills = Vec::new();
    for target in context.all_targets()? {
        if !target.canonical_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&target.canonical_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() && path.join("SKILL.md").is_file() {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                let qualified = if target.name == "global" {
                    format!("global/{name}")
                } else {
                    format!("{}/{name}", target.name)
                };
                skills.push(qualified);
            }
        }
    }
    skills.sort();
    skills.dedup();
    Ok(skills)
}

fn normalize_skill_name(name: &str, current: &BTreeMap<String, UsageReportRow>) -> String {
    if current.contains_key(name) {
        return name.to_string();
    }
    let global = format!("global/{name}");
    if current.contains_key(&global) {
        global
    } else {
        name.to_string()
    }
}

fn parse_duration(value: &str) -> Result<i64> {
    let (number, suffix) = value.trim().split_at(value.trim().len().saturating_sub(1));
    let amount: i64 = number
        .parse()
        .with_context(|| format!("invalid duration `{value}`"))?;
    let seconds = match suffix {
        "s" => amount,
        "m" => amount * 60,
        "h" => amount * 3600,
        "d" => amount * 86400,
        "w" => amount * 604800,
        _ => bail!("duration must end in s, m, h, d, or w"),
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    Ok(now - seconds)
}

fn min_seen(current: Option<String>, candidate: Option<String>) -> Option<String> {
    match (current, candidate) {
        (None, value) => value,
        (value, None) => value,
        (Some(a), Some(b)) => Some(a.min(b)),
    }
}
fn max_seen(current: Option<String>, candidate: Option<String>) -> Option<String> {
    match (current, candidate) {
        (None, value) => value,
        (value, None) => value,
        (Some(a), Some(b)) => Some(a.max(b)),
    }
}

fn open_db(target: DbTarget) -> Result<Db> {
    match target {
        DbTarget::Sqlite(path) => Db::open(&path),
        DbTarget::Postgres(url) => Db::open_postgres(&url),
    }
}
