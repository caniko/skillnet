use std::collections::BTreeMap;

use anyhow::{bail, Context};

use super::{analyze::pretty_threshold, Db};

pub fn run(db: &Db, since: Option<&str>) -> anyhow::Result<()> {
    if let Some(since) = since {
        validate_iso_date(since)?;
    }

    let mut sql = "SELECT
            trigger_name,
            current_threshold,
            proposed_threshold,
            supporting_plan_ids,
            fire_rate,
            signal_rate,
            filter_tags,
            rationale,
            strftime('%Y-%m-%d', decided_at, 'unixepoch') AS decided_date
        FROM calibration_proposals
        WHERE decision = 'accepted'
          AND decided_at IS NOT NULL"
        .to_string();
    if since.is_some() {
        sql.push_str(" AND decided_date >= ?1");
    }
    sql.push_str(" ORDER BY decided_at DESC, id DESC");

    let mut stmt = db.connection().prepare(&sql)?;
    let mut rows = if let Some(since) = since {
        stmt.query([since])?
    } else {
        stmt.query([])?
    };

    let mut emitted = 0;
    while let Some(row) = rows.next()? {
        emitted += 1;
        let trigger: String = row.get(0)?;
        let old: f64 = row.get(1)?;
        let new: f64 = row.get(2)?;
        let supporting_raw: String = row.get(3)?;
        let supporting: Vec<String> = serde_json::from_str(&supporting_raw)
            .with_context(|| format!("failed to parse supporting plan ids for {trigger}"))?;
        let filter_raw: Option<String> = row.get(6)?;
        let filters = filter_raw
            .as_deref()
            .map(serde_json::from_str::<BTreeMap<String, String>>)
            .transpose()
            .with_context(|| format!("failed to parse filter tags for {trigger}"))?;
        let rationale: Option<String> = row.get(7)?;
        let date: String = row.get(8)?;

        println!(
            "### {date} — {trigger}: {} → {}\n",
            pretty_threshold(old),
            pretty_threshold(new)
        );
        println!(
            "- **Rationale**: {}",
            rationale.unwrap_or_else(|| "accepted".to_string())
        );
        println!(
            "- **Fire rate at decision**: {:.1}%",
            row.get::<_, f64>(4)? * 100.0
        );
        println!(
            "- **Signal rate at decision**: {:+.2}",
            row.get::<_, f64>(5)?
        );
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

    if emitted == 0 {
        println!("no accepted proposals");
    }
    Ok(())
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
