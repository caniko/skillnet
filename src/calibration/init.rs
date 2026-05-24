use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use uuid::Uuid;

use super::{
    catalog::ThresholdStore,
    eval, meta_cmd, plan_parser,
    shape_hash,
    sidecar::{PhaseRecord, PlanRecord, Sidecar, TriggerRecord},
    Db,
};

const SIDECAR_FILE: &str = ".calibration.json";

pub struct InitOptions {
    pub stdout: bool,
    pub force: bool,
}

pub fn run(db: &Db, plan_dir: &Path, options: InitOptions) -> anyhow::Result<()> {
    let sidecar = build_sidecar(db, plan_dir, options.force)?;
    let rendered = serde_json::to_string_pretty(&sidecar).context("failed to serialize sidecar")?;
    if options.stdout {
        println!("{rendered}");
        return Ok(());
    }

    let path = plan_dir.join(SIDECAR_FILE);
    if path.exists() && !options.force {
        bail!(
            "refusing to overwrite existing sidecar {}; pass --force to replace it",
            path.display()
        );
    }
    fs::write(&path, format!("{rendered}\n"))
        .with_context(|| format!("failed to write sidecar {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

pub fn build_sidecar(db: &Db, plan_dir: &Path, force: bool) -> anyhow::Result<Sidecar> {
    let previous = if force {
        load_existing_sidecar(plan_dir)?
    } else {
        None
    };
    let plan = plan_parser::parse(plan_dir)?;
    let store = ThresholdStore::load(db)?;
    let triggers = eval::evaluate_triggers(&plan, &store)?
        .into_iter()
        .map(|row| TriggerRecord {
            name: row.name,
            input_value: row.input_value,
            threshold: row.threshold,
            fired: row.fired,
            section_added: row.section_added,
        })
        .collect::<Vec<_>>();
    let meta = meta_cmd::evaluate(db, plan_dir, None)?;
    let sidecar = Sidecar {
        schema_version: 1,
        plan: PlanRecord {
            id: previous
                .as_ref()
                .map(|sidecar| sidecar.plan.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            name: plan.name.clone(),
            flavor: plan.flavor.clone(),
            worktype: plan.worktype.clone(),
            created_at: previous
                .as_ref()
                .map(|sidecar| sidecar.plan.created_at)
                .transpose()
                .unwrap_or_else(unix_timestamp)?,
            phase_count: plan.phase_count,
            wave_count: plan.wave_count,
            max_chain_depth: plan.max_chain_depth,
            repo_spread: plan.repo_spread,
            routing_dist: plan.routing_dist.clone(),
            shape_hash: shape_hash::compute(&plan),
        },
        triggers,
        phases: plan
            .phases
            .iter()
            .map(|phase| PhaseRecord {
                ordinal: phase.ordinal,
                slug: phase.slug.clone(),
                routing_tier: phase.routing_tier.clone(),
                files: phase.files.clone(),
            })
            .collect(),
        meta_heuristics_fired: meta.fired,
        tags: previous
            .as_ref()
            .map(|sidecar| sidecar.tags.clone())
            .unwrap_or_else(|| auto_tags(&plan)),
        verify: previous.and_then(|sidecar| sidecar.verify),
    };
    Ok(sidecar)
}

fn load_existing_sidecar(plan_dir: &Path) -> anyhow::Result<Option<Sidecar>> {
    let path = plan_dir.join(SIDECAR_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read existing sidecar {}", path.display()))?;
    serde_json::from_str(&raw)
        .map(Some)
        .with_context(|| format!("malformed existing sidecar {}", path.display()))
}

fn auto_tags(plan: &super::catalog::PlanInputs) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::new();
    tags.insert("flavor".to_string(), plan.flavor.clone());
    if let Some(worktype) = &plan.worktype {
        tags.insert("worktype".to_string(), worktype.clone());
    }
    tags.insert(
        "scope".to_string(),
        match plan.repo_spread {
            0 | 1 => "single-repo",
            2 => "multi-repo",
            _ => "cross-org",
        }
        .to_string(),
    );
    tags.insert(
        "risk".to_string(),
        if plan.routing_dist.get("max").copied().unwrap_or(0) > 0 {
            "high"
        } else if plan.routing_dist.get("high").copied().unwrap_or(0) > 0 {
            "mixed"
        } else {
            "low"
        }
        .to_string(),
    );
    tags
}

fn unix_timestamp() -> anyhow::Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    i64::try_from(duration.as_secs()).context("unix timestamp does not fit in i64")
}
