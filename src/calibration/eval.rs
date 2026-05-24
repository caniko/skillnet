use anyhow::Context;
use serde::Serialize;

use super::{
    catalog::{PlanInputs, ThresholdSource, ThresholdStore, TriggerOutcome, HEURISTICS},
    plan_parser, Db,
};

#[derive(Clone, Copy, Debug)]
pub enum OutputFormat {
    Json,
    Table,
}

#[derive(Debug, Serialize)]
pub struct EvalRow {
    pub name: String,
    pub category: String,
    pub input_value: f64,
    pub threshold: f64,
    pub threshold_source: ThresholdSource,
    pub fired: bool,
    pub section_added: Option<String>,
}

impl EvalRow {
    pub fn outcome(&self) -> TriggerOutcome {
        TriggerOutcome {
            input_value: self.input_value,
            threshold: self.threshold,
            fired: self.fired,
            section_added: self.section_added.clone(),
        }
    }
}

pub fn run(db: &Db, plan_dir: &std::path::Path, format: OutputFormat) -> anyhow::Result<()> {
    let plan = plan_parser::parse(plan_dir)?;
    let store = ThresholdStore::load(db)?;
    let rows = evaluate_triggers(&plan, &store)?;
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        OutputFormat::Table => print_table(&rows),
    }
    Ok(())
}

pub fn evaluate_triggers(
    plan: &PlanInputs,
    store: &ThresholdStore<'_>,
) -> anyhow::Result<Vec<EvalRow>> {
    HEURISTICS
        .iter()
        .copied()
        .map(|heuristic| {
            let name = heuristic.name();
            let threshold = store
                .get_optional(name)
                .with_context(|| format!("missing threshold for heuristic {name}"))?;
            let threshold_source = store.source(name).unwrap_or(ThresholdSource::Default);
            let outcome = heuristic.evaluate(plan, threshold);
            Ok(EvalRow {
                name: name.to_string(),
                category: heuristic.category().as_str().to_string(),
                input_value: outcome.input_value,
                threshold,
                threshold_source,
                fired: outcome.fired,
                section_added: outcome.section_added,
            })
        })
        .collect()
}

fn print_table(rows: &[EvalRow]) {
    println!(
        "{:<32} {:<14} {:>10} {:>10} {:<9} SECTION",
        "NAME", "CATEGORY", "INPUT", "THRESHOLD", "FIRED"
    );
    for row in rows {
        println!(
            "{:<32} {:<14} {:>10.2} {:>10.2} {:<9} {}",
            row.name,
            row.category,
            row.input_value,
            row.threshold,
            row.fired,
            row.section_added.as_deref().unwrap_or("")
        );
    }
}
