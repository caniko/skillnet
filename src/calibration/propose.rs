use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;

use super::{
    analyze::{self, AnalyzeOptions},
    db::DbParam as P,
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

    let id = db.execute_returning_id(
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
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9)",
        &[
            P::from(unix_timestamp()?),
            P::from(input.trigger.as_str()),
            P::from(current_threshold),
            P::from(input.new_threshold),
            P::from(&supporting_plan_ids),
            P::from(trigger.fire_rate),
            P::from(signal_rate),
            P::nullable_text(filter_tags.as_deref()),
            P::from(input.rationale.as_str()),
        ],
    )?;
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
        sql.push_str(" WHERE decision = $1");
    }
    sql.push_str(" ORDER BY id");

    let params = if let Some(decision) = filter.decision() {
        vec![P::from(decision)]
    } else {
        Vec::new()
    };
    let rows = db.query_all(&sql, &params, |row| {
        let ids_raw = row.get_string(7)?;
        let ids: Vec<String> = serde_json::from_str(&ids_raw).unwrap_or_default();
        Ok((
            row.get_i64(0)?,
            row.get_string(1)?,
            row.get_f64(2)?,
            row.get_f64(3)?,
            row.get_string(4)?,
            row.get_f64(5)?,
            row.get_f64(6)?,
            ids,
        ))
    })?;

    println!(
        "{:<5} {:<28} {:>9} {:>9} {:<9} {:>7} {:>8} SUPPORT",
        "ID", "TRIGGER", "CURRENT", "PROPOSED", "DECISION", "FIRE%", "SIGNAL"
    );
    for (id, trigger, current, proposed, decision, fire_rate, signal_rate, ids) in &rows {
        println!(
            "{:<5} {:<28} {:>9} {:>9} {:<9} {:>6.1}% {:>+8.2} {}",
            id,
            trigger,
            analyze::pretty_threshold(*current),
            analyze::pretty_threshold(*proposed),
            decision,
            fire_rate * 100.0,
            signal_rate,
            ids.join(","),
        );
    }
    if rows.is_empty() {
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
