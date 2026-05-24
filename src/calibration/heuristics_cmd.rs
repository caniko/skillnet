use anyhow::{bail, Context};
use serde::Serialize;

use super::{
    catalog::{HeuristicCategory, ThresholdSource, ThresholdStore, HEURISTICS},
    Db,
};

#[derive(Clone, Copy, Debug)]
pub enum OutputFormat {
    Json,
    Table,
}

#[derive(Clone, Copy, Debug)]
pub enum CategoryFilter {
    Coordination,
    Risk,
    PlanShape,
    QualityLint,
}

impl CategoryFilter {
    pub fn category(self) -> HeuristicCategory {
        match self {
            Self::Coordination => HeuristicCategory::Coordination,
            Self::Risk => HeuristicCategory::Risk,
            Self::PlanShape => HeuristicCategory::PlanShape,
            Self::QualityLint => HeuristicCategory::QualityLint,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HeuristicRow {
    pub name: String,
    pub category: String,
    pub default_threshold: f64,
    pub current_threshold: f64,
    pub threshold_source: ThresholdSource,
    pub description: String,
    pub section_added_template: Option<String>,
}

pub fn list(db: &Db, format: OutputFormat, category: Option<CategoryFilter>) -> anyhow::Result<()> {
    let store = ThresholdStore::load(db)?;
    let rows = rows(&store, category.map(CategoryFilter::category))?;
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        OutputFormat::Table => print_table(&rows),
    }
    Ok(())
}

pub fn show(db: &Db, name: &str) -> anyhow::Result<()> {
    let store = ThresholdStore::load(db)?;
    let row = rows(&store, None)?
        .into_iter()
        .find(|row| row.name == name)
        .with_context(|| format!("unknown heuristic {name}"))?;
    println!("{}", serde_json::to_string_pretty(&row)?);
    Ok(())
}

fn rows(
    store: &ThresholdStore<'_>,
    category: Option<HeuristicCategory>,
) -> anyhow::Result<Vec<HeuristicRow>> {
    HEURISTICS
        .iter()
        .copied()
        .filter(|heuristic| category.is_none_or(|category| heuristic.category() == category))
        .map(|heuristic| {
            let name = heuristic.name();
            let current_threshold = store
                .get_optional(name)
                .with_context(|| format!("missing threshold for heuristic {name}"))?;
            let threshold_source = store.source(name).unwrap_or(ThresholdSource::Default);
            Ok(HeuristicRow {
                name: name.to_string(),
                category: heuristic.category().as_str().to_string(),
                default_threshold: heuristic.default_threshold(),
                current_threshold,
                threshold_source,
                description: heuristic.description().to_string(),
                section_added_template: heuristic.section_added(true).map(str::to_string),
            })
        })
        .collect()
}

fn print_table(rows: &[HeuristicRow]) {
    if rows.is_empty() {
        println!("no heuristics");
        return;
    }
    println!(
        "{:<32} {:<14} {:>8} {:>8} {:<9} SECTION",
        "NAME", "CATEGORY", "DEFAULT", "CURRENT", "SOURCE"
    );
    for row in rows {
        println!(
            "{:<32} {:<14} {:>8.2} {:>8.2} {:<9} {}",
            row.name,
            row.category,
            row.default_threshold,
            row.current_threshold,
            source_label(&row.threshold_source),
            row.section_added_template.as_deref().unwrap_or("")
        );
    }
}

fn source_label(source: &ThresholdSource) -> &'static str {
    match source {
        ThresholdSource::Default => "default",
        ThresholdSource::Override { .. } => "override",
    }
}

pub fn category_from_str(raw: &str) -> anyhow::Result<CategoryFilter> {
    match raw {
        "coordination" => Ok(CategoryFilter::Coordination),
        "risk" => Ok(CategoryFilter::Risk),
        "plan-shape" => Ok(CategoryFilter::PlanShape),
        "quality-lint" => Ok(CategoryFilter::QualityLint),
        _ => bail!(
            "unknown heuristic category {raw}; expected coordination, risk, plan-shape, or quality-lint"
        ),
    }
}
