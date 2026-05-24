use anyhow::Context;
use serde::Serialize;

use super::{catalog::PlanInputs, plan_parser};

#[derive(Debug, Serialize)]
pub struct ShapeHashOutput {
    pub shape_hash: String,
}

pub fn run(plan_dir: &std::path::Path) -> anyhow::Result<()> {
    let plan = plan_parser::parse(plan_dir)?;
    println!("{}", compute(&plan));
    Ok(())
}

pub fn compute(plan: &PlanInputs) -> String {
    let mut hasher = blake3::Hasher::new();
    update_field(&mut hasher, "phase_count", plan.phase_count);
    update_field(&mut hasher, "wave_count", plan.wave_count);
    update_field(&mut hasher, "max_chain_depth", plan.max_chain_depth);
    update_field(&mut hasher, "repo_spread", plan.repo_spread);

    for (tier, count) in &plan.routing_dist {
        update_str(&mut hasher, "routing", tier);
        update_field(&mut hasher, "routing_count", *count);
    }

    let mut phases = plan.phases.iter().collect::<Vec<_>>();
    phases.sort_by_key(|phase| phase.ordinal);
    for phase in phases {
        update_field(&mut hasher, "phase", phase.ordinal);
        update_str(&mut hasher, "slug", &phase.slug);
        update_str(&mut hasher, "tier", &phase.routing_tier);
    }

    let mut files = plan
        .phases
        .iter()
        .flat_map(|phase| phase.files.iter())
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    for file in files {
        update_str(&mut hasher, "file", file);
    }

    hasher.finalize().to_hex().to_string()
}

pub fn json(plan_dir: &std::path::Path) -> anyhow::Result<String> {
    let plan = plan_parser::parse(plan_dir)?;
    serde_json::to_string_pretty(&ShapeHashOutput {
        shape_hash: compute(&plan),
    })
    .context("failed to serialize shape hash")
}

fn update_field(hasher: &mut blake3::Hasher, name: &str, value: u32) {
    update_str(hasher, name, &value.to_string());
}

fn update_str(hasher: &mut blake3::Hasher, name: &str, value: &str) {
    hasher.update(name.as_bytes());
    hasher.update(b"\0");
    hasher.update(value.as_bytes());
    hasher.update(b"\0");
}
