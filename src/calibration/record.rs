use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use anyhow::{bail, Context};
use rusqlite::params;

use super::{sidecar::Sidecar, Db};

pub fn run(plan_dir: &Path, db: &mut Db) -> anyhow::Result<()> {
    let sidecar = Sidecar::load(plan_dir)?;
    let plan_id = sidecar.plan.id.clone();
    let tags = derive_tags(&sidecar);
    let trigger_count = sidecar.triggers.len();
    let phase_count = sidecar.phases.len();
    let tag_count = tags.values().map(BTreeSet::len).sum::<usize>();
    let capture_reasons = serde_json::to_string(&sidecar.meta_heuristics_fired)
        .context("failed to serialize capture reasons")?;
    let routing_dist =
        serde_json::to_string(&sidecar.plan.routing_dist).context("failed to serialize routing")?;

    let tx = db
        .connection_mut()
        .transaction()
        .context("failed to start calibration record transaction")?;

    tx.execute(
        "INSERT INTO plans (
            id,
            created_at,
            name,
            path,
            flavor,
            worktype,
            phase_count,
            wave_count,
            max_chain_depth,
            repo_spread,
            routing_dist,
            shape_hash,
            capture_reasons
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ON CONFLICT(id) DO UPDATE SET
            created_at = excluded.created_at,
            name = excluded.name,
            path = excluded.path,
            flavor = excluded.flavor,
            worktype = excluded.worktype,
            phase_count = excluded.phase_count,
            wave_count = excluded.wave_count,
            max_chain_depth = excluded.max_chain_depth,
            repo_spread = excluded.repo_spread,
            routing_dist = excluded.routing_dist,
            shape_hash = excluded.shape_hash,
            capture_reasons = excluded.capture_reasons",
        params![
            sidecar.plan.id,
            sidecar.plan.created_at,
            sidecar.plan.name,
            plan_dir.display().to_string(),
            sidecar.plan.flavor,
            sidecar.plan.worktype,
            i64::from(sidecar.plan.phase_count),
            i64::from(sidecar.plan.wave_count),
            i64::from(sidecar.plan.max_chain_depth),
            i64::from(sidecar.plan.repo_spread),
            routing_dist,
            sidecar.plan.shape_hash,
            capture_reasons,
        ],
    )
    .context("failed to upsert plan")?;

    delete_record_children(&tx, &plan_id)?;

    for trigger in &sidecar.triggers {
        tx.execute(
            "INSERT INTO triggers (
                plan_id,
                name,
                input_value,
                threshold,
                fired,
                section_added
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                plan_id,
                trigger.name,
                trigger.input_value,
                trigger.threshold,
                trigger.fired,
                trigger.section_added,
            ],
        )
        .context("failed to insert trigger")?;
    }

    for phase in &sidecar.phases {
        let files = serde_json::to_string(&phase.files).context("failed to serialize files")?;
        tx.execute(
            "INSERT INTO phases (
                plan_id,
                ordinal,
                slug,
                routing_tier,
                files
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                plan_id,
                i64::from(phase.ordinal),
                phase.slug,
                phase.routing_tier,
                files,
            ],
        )
        .context("failed to insert phase")?;
    }

    insert_tags(&tx, &plan_id, &tags)?;
    tx.commit()
        .context("failed to commit calibration record transaction")?;

    println!(
        "recorded {plan_id} ({trigger_count} triggers, {phase_count} phases, {tag_count} tags)"
    );
    Ok(())
}

pub fn run_verify(plan_dir: &Path, db: &mut Db) -> anyhow::Result<()> {
    let sidecar = Sidecar::load(plan_dir)?;
    let verify = sidecar.verify.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "no verify section in sidecar {}",
            plan_dir.join(".calibration.json").display()
        )
    })?;
    let plan_id = sidecar.plan.id.clone();
    let phase_outcomes =
        serde_json::to_string(&verify.phase_outcomes).context("failed to serialize outcomes")?;
    let emergency_changes = verify
        .emergency_changes
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("failed to serialize emergency changes")?;
    let passed = verify
        .phase_outcomes
        .values()
        .filter(|outcome| outcome.as_str() == "passed")
        .count();
    let total = verify.phase_outcomes.len();

    let tx = db
        .connection_mut()
        .transaction()
        .context("failed to start calibration verify transaction")?;

    let plan_exists: bool = tx
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM plans WHERE id = ?1)",
            [&plan_id],
            |row| row.get(0),
        )
        .context("failed to check plan existence")?;
    if !plan_exists {
        bail!("plan {} has not been recorded", plan_id);
    }

    tx.execute("DELETE FROM verifications WHERE plan_id = ?1", [&plan_id])
        .context("failed to delete prior verification")?;
    tx.execute(
        "INSERT INTO verifications (
            plan_id,
            verified_at,
            elapsed_seconds,
            outcome,
            phase_outcomes,
            emergency_changes,
            surprises
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            plan_id,
            verify.verified_at,
            verify.elapsed_seconds,
            verify.outcome,
            phase_outcomes,
            emergency_changes,
            verify.surprises,
        ],
    )
    .context("failed to insert verification")?;
    tx.execute(
        "DELETE FROM tags WHERE plan_id = ?1 AND key = 'outcome'",
        [&plan_id],
    )
    .context("failed to delete prior outcome tag")?;
    tx.execute(
        "INSERT INTO tags (plan_id, key, value) VALUES (?1, 'outcome', ?2)",
        params![plan_id, verify.outcome],
    )
    .context("failed to insert outcome tag")?;
    tx.commit()
        .context("failed to commit calibration verify transaction")?;

    println!(
        "verified {plan_id}: {} ({passed}/{total} phases passed)",
        verify.outcome
    );
    Ok(())
}

fn delete_record_children(tx: &rusqlite::Transaction<'_>, plan_id: &str) -> anyhow::Result<()> {
    tx.execute("DELETE FROM triggers WHERE plan_id = ?1", [plan_id])
        .context("failed to delete prior triggers")?;
    tx.execute("DELETE FROM phases WHERE plan_id = ?1", [plan_id])
        .context("failed to delete prior phases")?;
    tx.execute("DELETE FROM tags WHERE plan_id = ?1", [plan_id])
        .context("failed to delete prior tags")?;
    Ok(())
}

fn insert_tags(
    tx: &rusqlite::Transaction<'_>,
    plan_id: &str,
    tags: &BTreeMap<String, BTreeSet<String>>,
) -> anyhow::Result<()> {
    for (key, values) in tags {
        for value in values {
            tx.execute(
                "INSERT INTO tags (plan_id, key, value) VALUES (?1, ?2, ?3)",
                params![plan_id, key, value],
            )
            .with_context(|| format!("failed to insert tag {key}:{value}"))?;
        }
    }
    Ok(())
}

fn derive_tags(sidecar: &Sidecar) -> BTreeMap<String, BTreeSet<String>> {
    let mut tags = BTreeMap::new();
    insert_tag(&mut tags, "flavor", sidecar.plan.flavor.clone());
    if let Some(worktype) = &sidecar.plan.worktype {
        insert_tag(&mut tags, "worktype", worktype.clone());
    }
    insert_tag(
        &mut tags,
        "scope",
        match sidecar.plan.repo_spread {
            0 | 1 => "single-repo",
            2 => "multi-repo",
            _ => "cross-org",
        }
        .to_string(),
    );
    insert_tag(&mut tags, "risk", derive_risk(sidecar).to_string());

    for signal in &sidecar.meta_heuristics_fired {
        insert_tag(&mut tags, "signal", signal.clone());
    }

    for (key, value) in &sidecar.tags {
        let mut values = BTreeSet::new();
        values.insert(value.clone());
        tags.insert(key.clone(), values);
    }

    tags
}

fn insert_tag(tags: &mut BTreeMap<String, BTreeSet<String>>, key: &str, value: String) {
    tags.entry(key.to_string()).or_default().insert(value);
}

fn derive_risk(sidecar: &Sidecar) -> &'static str {
    if sidecar.plan.routing_dist.get("max").copied().unwrap_or(0) > 0 {
        "high"
    } else if sidecar.plan.routing_dist.get("high").copied().unwrap_or(0) > 0 {
        "mixed"
    } else if sidecar
        .plan
        .routing_dist
        .iter()
        .all(|(tier, count)| *count == 0 || tier == "low" || tier == "medium")
    {
        "low"
    } else {
        "mixed"
    }
}
