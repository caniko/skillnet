use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use rusqlite::params;

use super::{
    analyze::{self, AnalyzeOptions},
    Db,
};

pub struct ProposeInput {
    pub trigger: String,
    pub new_threshold: f64,
    pub filter_tags: Vec<(String, String)>,
    pub rationale: String,
    pub supporting_plan_ids: Vec<String>,
}

#[derive(Clone, Copy)]
pub enum ProposalFilter {
    All,
    Pending,
    Accepted,
    Rejected,
}

impl ProposalFilter {
    pub fn from_flags(pending: bool, accepted: bool, rejected: bool) -> Self {
        if pending {
            Self::Pending
        } else if accepted {
            Self::Accepted
        } else if rejected {
            Self::Rejected
        } else {
            Self::All
        }
    }

    fn decision(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Pending => Some("pending"),
            Self::Accepted => Some("accepted"),
            Self::Rejected => Some("rejected"),
        }
    }
}

pub fn run(db: &mut Db, input: ProposeInput) -> anyhow::Result<()> {
    let current_threshold = analyze::latest_threshold(db, &input.trigger, &input.filter_tags)?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no verified calibration rows found for trigger {} with the requested filters",
                input.trigger
            )
        })?;
    let report = analyze::analyze(
        db,
        &AnalyzeOptions {
            filter_tags: input.filter_tags.clone(),
            trigger: Some(input.trigger.clone()),
            min_n: 1,
        },
    )?;
    let trigger = report
        .triggers
        .first()
        .ok_or_else(|| anyhow::anyhow!("no analysis row found for trigger {}", input.trigger))?;
    let signal_rate = trigger.signal_rate.unwrap_or(0.0);
    let filter_tags = filter_tags_json(&input.filter_tags)?;
    let supporting_plan_ids = serde_json::to_string(&input.supporting_plan_ids)
        .context("failed to serialize supporting plan ids")?;

    db.connection_mut().execute(
        "INSERT INTO calibration_proposals (
            proposed_at,
            trigger_name,
            current_threshold,
            proposed_threshold,
            supporting_plan_ids,
            fire_rate,
            signal_rate,
            filter_tags,
            decision,
            rationale
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9)",
        params![
            unix_timestamp()?,
            input.trigger,
            current_threshold,
            input.new_threshold,
            supporting_plan_ids,
            trigger.fire_rate,
            signal_rate,
            filter_tags,
            input.rationale,
        ],
    )?;

    let id = db.connection().last_insert_rowid();
    println!(
        "proposal {id} pending: {} {} -> {}",
        trigger.trigger,
        analyze::pretty_threshold(current_threshold),
        analyze::pretty_threshold(input.new_threshold)
    );
    Ok(())
}

pub fn list(db: &Db, filter: ProposalFilter) -> anyhow::Result<()> {
    let mut sql = "SELECT
            id,
            trigger_name,
            current_threshold,
            proposed_threshold,
            decision,
            fire_rate,
            signal_rate,
            supporting_plan_ids
        FROM calibration_proposals"
        .to_string();
    if filter.decision().is_some() {
        sql.push_str(" WHERE decision = ?1");
    }
    sql.push_str(" ORDER BY id");

    let mut stmt = db.connection().prepare(&sql)?;
    let mut rows = if let Some(decision) = filter.decision() {
        stmt.query([decision])?
    } else {
        stmt.query([])?
    };

    println!(
        "{:<5} {:<28} {:>9} {:>9} {:<9} {:>7} {:>8} SUPPORT",
        "ID", "TRIGGER", "CURRENT", "PROPOSED", "DECISION", "FIRE%", "SIGNAL"
    );
    let mut count = 0;
    while let Some(row) = rows.next()? {
        count += 1;
        let ids_raw: String = row.get(7)?;
        let ids: Vec<String> = serde_json::from_str(&ids_raw).unwrap_or_default();
        println!(
            "{:<5} {:<28} {:>9} {:>9} {:<9} {:>6.1}% {:>+8.2} {}",
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            analyze::pretty_threshold(row.get::<_, f64>(2)?),
            analyze::pretty_threshold(row.get::<_, f64>(3)?),
            row.get::<_, String>(4)?,
            row.get::<_, f64>(5)? * 100.0,
            row.get::<_, f64>(6)?,
            ids.join(","),
        );
    }
    if count == 0 {
        println!("no proposals");
    }
    Ok(())
}

fn filter_tags_json(filter_tags: &[(String, String)]) -> anyhow::Result<Option<String>> {
    if filter_tags.is_empty() {
        return Ok(None);
    }

    let tags = filter_tags
        .iter()
        .cloned()
        .collect::<BTreeMap<String, String>>();
    serde_json::to_string(&tags)
        .map(Some)
        .context("failed to serialize filter tags")
}

fn unix_timestamp() -> anyhow::Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    i64::try_from(duration.as_secs()).context("unix timestamp does not fit in i64")
}
