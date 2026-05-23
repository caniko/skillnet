use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use anyhow::{bail, Context};

use super::{db::DbParam as P, db::Tx, sidecar::Sidecar, Db};

pub fn run(plan_dir: &Path, db: &mut Db) -> anyhow::Result<()> {
    let sidecar = Sidecar::load(plan_dir)?;
    let plan_id = sidecar.plan.id.clone();
    let tags = derive_tags(&sidecar);
    let trigger_count = sidecar.triggers.len();
    let phase_count = sidecar.phases.len();
    let tag_count = tags.values().map(BTreeSet::len).sum::<usize>();
    let plan_path = plan_dir.display().to_string();
    let capture_reasons = serde_json::to_string(&sidecar.meta_heuristics_fired)
        .context("failed to serialize capture reasons")?;
    let routing_dist =
        serde_json::to_string(&sidecar.plan.routing_dist).context("failed to serialize routing")?;

    db.transaction(|tx| {
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
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
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
            &[
                P::from(&sidecar.plan.id),
                P::from(sidecar.plan.created_at),
                P::from(&sidecar.plan.name),
                P::from(&plan_path),
                P::from(&sidecar.plan.flavor),
                P::nullable_text(sidecar.plan.worktype.as_deref()),
                P::from(i64::from(sidecar.plan.phase_count)),
                P::from(i64::from(sidecar.plan.wave_count)),
                P::from(i64::from(sidecar.plan.max_chain_depth)),
                P::from(i64::from(sidecar.plan.repo_spread)),
                P::from(&routing_dist),
                P::from(&sidecar.plan.shape_hash),
                P::from(&capture_reasons),
            ],
        )
        .context("failed to upsert plan")?;

        delete_record_children(tx, &plan_id)?;

        for trigger in &sidecar.triggers {
            tx.execute(
                "INSERT INTO triggers (
                plan_id,
                name,
                input_value,
                threshold,
                fired,
                section_added
            ) VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    P::from(&plan_id),
                    P::from(&trigger.name),
                    P::from(trigger.input_value),
                    P::from(trigger.threshold),
                    P::from(trigger.fired),
                    P::nullable_text(trigger.section_added.as_deref()),
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
            ) VALUES ($1, $2, $3, $4, $5)",
                &[
                    P::from(&plan_id),
                    P::from(i64::from(phase.ordinal)),
                    P::from(&phase.slug),
                    P::from(&phase.routing_tier),
                    P::from(&files),
                ],
            )
            .context("failed to insert phase")?;
        }

        insert_tags(tx, &plan_id, &tags)?;
        Ok(())
    })
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

    db.transaction(|tx| {
        let plan_exists: bool = tx
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM plans WHERE id = $1)",
                &[P::from(&plan_id)],
                |row| row.get_bool(0),
            )
            .context("failed to check plan existence")?;
        if !plan_exists {
            bail!("plan {plan_id} has not been recorded");
        }

        tx.execute(
            "DELETE FROM verifications WHERE plan_id = $1",
            &[P::from(&plan_id)],
        )
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
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                P::from(&plan_id),
                P::from(verify.verified_at),
                P::nullable_i64(verify.elapsed_seconds),
                P::from(&verify.outcome),
                P::from(&phase_outcomes),
                P::nullable_text(emergency_changes.as_deref()),
                P::nullable_text(verify.surprises.as_deref()),
            ],
        )
        .context("failed to insert verification")?;
        tx.execute(
            "DELETE FROM tags WHERE plan_id = $1 AND key = 'outcome'",
            &[P::from(&plan_id)],
        )
        .context("failed to delete prior outcome tag")?;
        tx.execute(
            "INSERT INTO tags (plan_id, key, value) VALUES ($1, 'outcome', $2)",
            &[P::from(&plan_id), P::from(&verify.outcome)],
        )
        .context("failed to insert outcome tag")?;
        Ok(())
    })
    .context("failed to commit calibration verify transaction")?;

    println!(
        "verified {plan_id}: {} ({passed}/{total} phases passed)",
        verify.outcome
    );
    Ok(())
}

fn delete_record_children(tx: &mut Tx<'_>, plan_id: &str) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM triggers WHERE plan_id = $1",
        &[P::from(plan_id)],
    )
    .context("failed to delete prior triggers")?;
    tx.execute("DELETE FROM phases WHERE plan_id = $1", &[P::from(plan_id)])
        .context("failed to delete prior phases")?;
    tx.execute("DELETE FROM tags WHERE plan_id = $1", &[P::from(plan_id)])
        .context("failed to delete prior tags")?;
    Ok(())
}

fn insert_tags(
    tx: &mut Tx<'_>,
    plan_id: &str,
    tags: &BTreeMap<String, BTreeSet<String>>,
) -> anyhow::Result<()> {
    for (key, values) in tags {
        for value in values {
            tx.execute(
                "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                &[P::from(plan_id), P::from(key), P::from(value)],
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
