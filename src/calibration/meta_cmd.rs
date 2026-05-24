use anyhow::Context;
use serde::Serialize;

use super::{
    catalog::{MetaHeuristicInputs, ThresholdStore, META_HEURISTICS},
    eval, plan_parser,
    sidecar::Sidecar,
    Db,
};

#[derive(Debug, Serialize)]
pub struct MetaHeuristicsOutput {
    pub fired: Vec<String>,
    pub not_fired: Vec<String>,
}

pub fn run(
    db: &Db,
    plan_dir: &std::path::Path,
    sidecar_path: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let output = evaluate(db, plan_dir, sidecar_path)?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub fn evaluate(
    db: &Db,
    plan_dir: &std::path::Path,
    sidecar_path: Option<&std::path::Path>,
) -> anyhow::Result<MetaHeuristicsOutput> {
    let plan = plan_parser::parse(plan_dir)?;
    let store = ThresholdStore::load(db)?;
    let trigger_rows = eval::evaluate_triggers(&plan, &store)?;
    let outcomes = trigger_rows
        .iter()
        .map(eval::EvalRow::outcome)
        .collect::<Vec<_>>();
    let sidecar = sidecar_path
        .map(load_sidecar_path)
        .transpose()
        .context("failed to load meta-heuristics sidecar")?;
    let mut inputs = MetaHeuristicInputs::new(&plan, &outcomes);
    inputs.verify = sidecar.as_ref().and_then(|sidecar| sidecar.verify.as_ref());

    let mut fired = Vec::new();
    let mut not_fired = Vec::new();
    for heuristic in META_HEURISTICS {
        if heuristic.fires(&inputs) {
            fired.push(heuristic.name().to_string());
        } else {
            not_fired.push(heuristic.name().to_string());
        }
    }
    Ok(MetaHeuristicsOutput { fired, not_fired })
}

fn load_sidecar_path(path: &std::path::Path) -> anyhow::Result<Sidecar> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read sidecar {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("malformed sidecar {}", path.display()))
}
