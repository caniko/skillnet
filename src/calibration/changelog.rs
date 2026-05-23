use std::collections::BTreeMap;

use anyhow::{bail, Context};

use super::{analyze::pretty_threshold, db::DbParam as P, Db};

pub fn run(db: &Db, since: Option<&str>) -> anyhow::Result<()> {
    if let Some(since) = since {
        validate_iso_date(since)?;
    }

    let decided_date_expr = db.unix_date_expr("decided_at");
    let mut sql = format!(
        "SELECT
            trigger_name,
            current_threshold,
            proposed_threshold,
            supporting_plan_ids,
            fire_rate,
            signal_rate,
            filter_tags,
            rationale,
            {decided_date_expr} AS decided_date
        FROM calibration_proposals
        WHERE decision = 'accepted'
          AND decided_at IS NOT NULL"
    );
    if since.is_some() {
        sql.push_str(&format!(" AND {decided_date_expr} >= $1"));
    }
    sql.push_str(" ORDER BY decided_at DESC, id DESC");

    let params = if let Some(since) = since {
        vec![P::from(since)]
    } else {
        Vec::new()
    };
    let rows = db.query_all(&sql, &params, |row| {
        Ok(AcceptedProposalRow {
            trigger: row.get_string(0)?,
            old: row.get_f64(1)?,
            new: row.get_f64(2)?,
            supporting_raw: row.get_string(3)?,
            fire_rate: row.get_f64(4)?,
            signal_rate: row.get_f64(5)?,
            filter_raw: row.get_optional_string(6)?,
            rationale: row.get_optional_string(7)?,
            date: row.get_string(8)?,
        })
    })?;

    for row in &rows {
        let supporting: Vec<String> = serde_json::from_str(&row.supporting_raw)
            .with_context(|| format!("failed to parse supporting plan ids for {}", row.trigger))?;
        let filters = row
            .filter_raw
            .as_deref()
            .map(serde_json::from_str::<BTreeMap<String, String>>)
            .transpose()
            .with_context(|| format!("failed to parse filter tags for {}", row.trigger))?;

        println!(
            "### {date} — {trigger}: {} → {}\n",
            pretty_threshold(row.old),
            pretty_threshold(row.new),
            date = row.date.as_str(),
            trigger = row.trigger.as_str(),
        );
        println!(
            "- **Rationale**: {}",
            row.rationale
                .clone()
                .unwrap_or_else(|| "accepted".to_string())
        );
        println!("- **Fire rate at decision**: {:.1}%", row.fire_rate * 100.0);
        println!("- **Signal rate at decision**: {:+.2}", row.signal_rate);
        println!(
            "- **Supporting plans**: {}, ids: {}",
            supporting.len(),
            supporting.join(", ")
        );
        if let Some(filters) = filters {
            if !filters.is_empty() {
                let rendered = filters
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("- **Filter tags**: {rendered}");
            }
        }
        println!();
    }

    if rows.is_empty() {
        println!("no accepted proposals");
    }
    Ok(())
}

struct AcceptedProposalRow {
    trigger: String,
    old: f64,
    new: f64,
    supporting_raw: String,
    fire_rate: f64,
    signal_rate: f64,
    filter_raw: Option<String>,
    rationale: Option<String>,
    date: String,
}

fn validate_iso_date(value: &str) -> anyhow::Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..].iter().all(u8::is_ascii_digit)
    {
        return Ok(());
    }
    bail!("--since must be an ISO date in YYYY-MM-DD form");
}
