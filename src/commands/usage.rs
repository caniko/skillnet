use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context as AnyhowContext, Result};
use serde::Serialize;

use crate::{
    calibration::{db::DbParam, Db},
    cli::args::{UsageArgs, UsageCommand, UsageFormat},
    commands::Context,
    config::DbTarget,
};

const CARD_TELEMETRY_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Serialize)]
struct CardStatsRow {
    card: String,
    injected: i64,
    read: i64,
    read_through_rate: Option<f64>,
}

#[derive(Debug, Serialize)]
struct CardStatsTotals {
    injected: i64,
    read: i64,
    read_through_rate: Option<f64>,
}

#[derive(Debug, Serialize)]
struct CardStatsReport {
    cards: Vec<CardStatsRow>,
    totals: CardStatsTotals,
}

#[derive(Debug, Serialize)]
struct UsageReportRow {
    skill: String,
    uses: i64,
    sessions: i64,
    first_seen: Option<String>,
    last_seen: Option<String>,
    harnesses: BTreeMap<String, i64>,
}

pub(crate) fn run(args: UsageArgs, target: DbTarget, context: Option<&Context>) -> Result<()> {
    match args.command {
        UsageCommand::Record {
            skill,
            harness,
            session,
            project,
            event_id,
            outcome,
            adapter_version,
        } => {
            record(
                target,
                &UsageEvent {
                    skill,
                    harness,
                    session,
                    project,
                    event_id,
                    outcome,
                    adapter_version,
                },
                "SkillActivated",
                "skillnet-usage",
            )?;
            println!("recorded");
            Ok(())
        }
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

pub(super) fn record_card_read(
    target: DbTarget,
    card: &str,
    harness: &str,
    session: Option<&str>,
    root: &str,
) -> Result<()> {
    record(
        target,
        &UsageEvent {
            skill: card.to_owned(),
            harness: harness.to_owned(),
            session: session.map_or_else(|| uuid::Uuid::nil().to_string(), str::to_owned),
            project: Some(root.to_owned()),
            event_id: format!("card-read:{}", uuid::Uuid::new_v4()),
            outcome: "ok".to_owned(),
            adapter_version: "1".to_owned(),
        },
        "CardRead",
        "skillnet-card",
    )
}

pub(super) fn record_card_injected(
    target: DbTarget,
    cards: &[&str],
    harness: &str,
    session: Option<&str>,
    root: &str,
) -> Result<()> {
    if cards.is_empty() {
        return Ok(());
    }
    let db = open_db_with_timeout(target, CARD_TELEMETRY_TIMEOUT)?;
    let session_id = session.map_or_else(|| uuid::Uuid::nil().to_string(), str::to_owned);
    for &card in cards {
        let event = UsageEvent {
            skill: card.to_owned(),
            harness: harness.to_owned(),
            session: session_id.clone(),
            project: Some(root.to_owned()),
            event_id: format!("card-injected:{}", uuid::Uuid::new_v4()),
            outcome: "ok".to_owned(),
            adapter_version: "1".to_owned(),
        };
        insert_event(&db, &event, "CardInjected", "skillnet-trigger")?;
    }
    Ok(())
}

pub(super) fn card_stats(target: DbTarget, since: Option<&str>, format: UsageFormat) -> Result<()> {
    let since_unix = since.map(parse_duration).transpose()?;
    let db = open_db(target)?;
    let stats = db.card_stats(since_unix)?;
    let mut total_injected = 0_i64;
    let mut total_read = 0_i64;
    let cards: Vec<CardStatsRow> = stats
        .into_iter()
        .map(|stat| {
            total_injected += stat.injected;
            total_read += stat.read;
            CardStatsRow {
                card: stat.card,
                injected: stat.injected,
                read: stat.read,
                read_through_rate: read_through_rate(stat.injected, stat.read),
            }
        })
        .collect();
    let totals = CardStatsTotals {
        injected: total_injected,
        read: total_read,
        read_through_rate: read_through_rate(total_injected, total_read),
    };
    let report = CardStatsReport { cards, totals };
    match format {
        UsageFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        UsageFormat::Table => {
            println!("card\tinjected\tread\tread_through_rate");
            for row in &report.cards {
                println!(
                    "{}\t{}\t{}\t{}",
                    row.card,
                    row.injected,
                    row.read,
                    format_rate(row.read_through_rate)
                );
            }
            println!(
                "TOTAL\t{}\t{}\t{}",
                report.totals.injected,
                report.totals.read,
                format_rate(report.totals.read_through_rate)
            );
        }
    }
    Ok(())
}

fn read_through_rate(injected: i64, read: i64) -> Option<f64> {
    (injected > 0).then(|| read as f64 / injected as f64)
}

fn format_rate(rate: Option<f64>) -> String {
    rate.map_or_else(|| "-".to_owned(), |value| format!("{:.1}%", value * 100.0))
}

fn record(target: DbTarget, event: &UsageEvent, hook_event: &str, tool: &str) -> Result<()> {
    validate_event(event)?;
    let db = open_db(target)?;
    insert_event(&db, event, hook_event, tool)
}

fn validate_event(event: &UsageEvent) -> Result<()> {
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
    Ok(())
}

fn insert_event(db: &Db, event: &UsageEvent, hook_event: &str, tool: &str) -> Result<()> {
    validate_event(event)?;
    db.execute(
        "INSERT INTO skill_invocations (
            session_id, skill_name, tool_name, project_dir, started_at, ended_at,
            outcome, plan_id, payload, hook_event, harness, source_event_id,
            adapter_version, canonical_skill_name
         ) VALUES ($1, $2, $9, $3, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP,
                   $4, NULL, $5, $10, $6, $7, $8, $2)
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
            DbParam::from(tool),
            DbParam::from(hook_event),
        ],
    )?;
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
        .iter()
        .cloned()
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
        let skill = normalize_skill_name(&aggregate.skill_name, &skills);
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

fn current_skills(context: &Context) -> Result<BTreeSet<String>> {
    let mut skills = BTreeSet::new();
    for target in context.all_targets()? {
        if !target.canonical_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&target.canonical_path)? {
            let entry = entry?;
            let path = entry.path();
            let template = target.name == "global"
                && camino::Utf8Path::from_path(&path).is_some_and(crate::templates::is_template);
            if path.is_dir() && (path.join("SKILL.md").is_file() || template) {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                let qualified = if target.name == "global" {
                    format!("global/{name}")
                } else {
                    format!("{}/{name}", target.name)
                };
                skills.insert(qualified);
            }
        }
    }
    Ok(skills)
}

fn normalize_skill_name(name: &str, current: &BTreeSet<String>) -> String {
    if name.contains('/') {
        return name.to_string();
    }
    let global = format!("global/{name}");
    if current.contains(&global) {
        return global;
    }
    let mut matches = current
        .iter()
        .filter(|skill| skill.split_once('/').is_some_and(|(_, bare)| bare == name));
    match (matches.next(), matches.next()) {
        (Some(skill), None) => skill.clone(),
        _ => name.to_string(),
    }
}

fn parse_duration(value: &str) -> Result<i64> {
    let trimmed = value.trim();
    let mut chars = trimmed.chars();
    let multiplier = match chars.next_back() {
        Some('s') => 1,
        Some('m') => 60,
        Some('h') => 3_600,
        Some('d') => 86_400,
        Some('w') => 604_800,
        _ => bail!("duration must end in s, m, h, d, or w"),
    };
    let number = chars.as_str();
    let amount: i64 = number
        .parse()
        .with_context(|| format!("invalid duration `{value}`"))?;
    if amount <= 0 {
        bail!("duration must be positive");
    }
    let seconds = amount
        .checked_mul(multiplier)
        .context("duration overflow")?;
    let now = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())?;
    now.checked_sub(seconds).context("duration cutoff overflow")
}

fn min_seen(current: Option<String>, candidate: Option<String>) -> Option<String> {
    match (current, candidate) {
        (None, value) | (value, None) => value,
        (Some(a), Some(b)) => Some(a.min(b)),
    }
}
fn max_seen(current: Option<String>, candidate: Option<String>) -> Option<String> {
    match (current, candidate) {
        (None, value) | (value, None) => value,
        (Some(a), Some(b)) => Some(a.max(b)),
    }
}

fn open_db(target: DbTarget) -> Result<Db> {
    match target {
        DbTarget::Sqlite(path) => Db::open(&path),
        #[cfg(feature = "postgres")]
        DbTarget::Postgres(url) => Db::open_postgres(&url),
        #[cfg(not(feature = "postgres"))]
        DbTarget::Postgres(_) => bail!(
            "Postgres usage database requested, but this skillnet binary was built without the \
             `postgres` feature; rebuild with `--features postgres` or select the sqlite backend"
        ),
    }
}

fn open_db_with_timeout(target: DbTarget, timeout: Duration) -> Result<Db> {
    match target {
        DbTarget::Sqlite(path) => Db::open(&path),
        #[cfg(feature = "postgres")]
        DbTarget::Postgres(url) => Db::open_postgres_with_timeout(&url, timeout),
        #[cfg(not(feature = "postgres"))]
        DbTarget::Postgres(_) => {
            let _ = timeout;
            bail!(
                "Postgres usage database requested, but this skillnet binary was built without the \
                 `postgres` feature; rebuild with `--features postgres` or select the sqlite backend"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_accepts_units_and_rejects_bad_input() {
        let now = || {
            i64::try_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .unwrap()
        };
        for (input, seconds) in [
            (" 2s ", 2),
            ("2m", 120),
            ("2h", 7_200),
            ("2d", 172_800),
            ("2w", 1_209_600),
        ] {
            let before = now();
            let cutoff = parse_duration(input).unwrap();
            assert!((before..=now()).contains(&(cutoff + seconds)));
        }
        for input in [
            "-1m",
            "0s",
            "9223372036854775807w",
            "9223372036854775808s",
            "3日",
            "💥",
            "",
            "1.5h",
            "1",
        ] {
            assert!(parse_duration(input).is_err(), "{input}");
        }
    }

    #[test]
    fn historical_names_match_only_global_or_a_unique_project_skill() {
        let catalog = [
            "canix/update-canix",
            "a/shared",
            "b/shared",
            "global/shared",
            "a/ambiguous",
            "b/ambiguous",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect();
        for (raw, expected) in [
            ("update-canix", "canix/update-canix"),
            ("shared", "global/shared"),
            ("a/shared", "a/shared"),
            ("ambiguous", "ambiguous"),
            ("missing", "missing"),
        ] {
            assert_eq!(normalize_skill_name(raw, &catalog), expected);
        }
    }

    #[test]
    #[cfg(not(feature = "postgres"))]
    fn postgres_usage_without_the_backend_is_explicit_and_redacted() {
        let error = open_db(DbTarget::Postgres(
            "postgres://user:private@invalid/db".into(),
        ))
        .err()
        .unwrap();
        let message = error.to_string();
        assert!(message.contains("--features postgres"));
        assert!(!message.contains("private"));
    }
}
