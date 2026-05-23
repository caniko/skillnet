use anyhow::{bail, Context};
use serde::Serialize;
use serde_json::Value;

use super::{db::DbParam as P, db::DbRow, format, Db};

#[derive(Debug)]
pub struct QueryOptions {
    pub tags: Vec<(String, String)>,
    pub trigger: Option<String>,
    pub fired: bool,
    pub missed: bool,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Serialize)]
pub struct PlanDump {
    #[serde(flatten)]
    pub plan: PlanRow,
    pub triggers: Vec<TriggerRow>,
    pub phases: Vec<PhaseRow>,
    pub tags: Vec<TagRow>,
    pub verify: Option<VerificationRow>,
}

#[derive(Debug, Serialize)]
pub struct PlanRow {
    pub id: String,
    pub created_at: i64,
    pub name: String,
    pub path: String,
    pub flavor: String,
    pub worktype: Option<String>,
    pub phase_count: i64,
    pub wave_count: i64,
    pub max_chain_depth: i64,
    pub repo_spread: i64,
    pub routing_dist: Value,
    pub shape_hash: String,
    pub capture_reasons: Value,
}

#[derive(Debug, Serialize)]
pub struct TriggerRow {
    pub name: String,
    pub input_value: f64,
    pub threshold: f64,
    pub fired: bool,
    pub section_added: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PhaseRow {
    pub ordinal: i64,
    pub slug: String,
    pub routing_tier: String,
    pub files: Value,
}

#[derive(Debug, Serialize)]
pub struct TagRow {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct VerificationRow {
    pub verified_at: i64,
    pub elapsed_seconds: Option<i64>,
    pub outcome: String,
    pub phase_outcomes: Value,
    pub emergency_changes: Option<Value>,
    pub surprises: Option<String>,
}

#[derive(Debug, Serialize)]
struct QueryRow {
    #[serde(flatten)]
    plan: PlanRow,
    tags: Vec<TagRow>,
}

pub fn show(db: &Db, plan_id: &str) -> anyhow::Result<()> {
    let dump = load_plan_dump(db, plan_id)?;
    println!("{}", serde_json::to_string_pretty(&dump)?);
    Ok(())
}

pub fn query(db: &Db, options: QueryOptions, format_kind: OutputFormat) -> anyhow::Result<()> {
    let plans = query_plans(db, &options)?;
    match format_kind {
        OutputFormat::Table => print_query_table(db, &plans),
        OutputFormat::Json => {
            let rows = plans
                .into_iter()
                .map(|plan| {
                    let tags = load_tags(db, &plan.id)?;
                    Ok(QueryRow { plan, tags })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
            Ok(())
        }
    }
}

pub fn load_plan_dump(db: &Db, plan_id: &str) -> anyhow::Result<PlanDump> {
    let plan = load_plan(db, plan_id)?;
    let triggers = load_triggers(db, plan_id)?;
    let phases = load_phases(db, plan_id)?;
    let tags = load_tags(db, plan_id)?;
    let verify = load_verification(db, plan_id)?;
    Ok(PlanDump {
        plan,
        triggers,
        phases,
        tags,
        verify,
    })
}

fn load_plan(db: &Db, plan_id: &str) -> anyhow::Result<PlanRow> {
    db.query_optional(
        "SELECT id, created_at, name, path, flavor, worktype, phase_count, wave_count,
                max_chain_depth, repo_spread, routing_dist, shape_hash, capture_reasons
         FROM plans WHERE id = $1",
        &[P::from(plan_id)],
        plan_from_row,
    )
    .context("failed to load plan")?
    .map_or_else(|| bail!("unknown calibration plan id {plan_id}"), Ok)
}

fn query_plans(db: &Db, options: &QueryOptions) -> anyhow::Result<Vec<PlanRow>> {
    let mut sql = String::from(
        "SELECT id, created_at, name, path, flavor, worktype, phase_count, wave_count,
                max_chain_depth, repo_spread, routing_dist, shape_hash, capture_reasons
         FROM plans",
    );
    let mut clauses = Vec::new();
    let mut params = Vec::new();

    for (key, value) in &options.tags {
        let key_placeholder = push_param(&mut params, key);
        let value_placeholder = push_param(&mut params, value);
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM tags WHERE tags.plan_id = plans.id AND key = {key_placeholder} AND value = {value_placeholder})"
        ));
    }

    if let Some(trigger) = &options.trigger {
        let trigger_placeholder = push_param(&mut params, trigger);
        let mut clause = format!(
            "EXISTS (SELECT 1 FROM triggers WHERE triggers.plan_id = plans.id AND name = {trigger_placeholder}"
        );
        if options.fired || options.missed {
            let fired_placeholder = push_param(&mut params, options.fired);
            clause.push_str(&format!(" AND fired = {fired_placeholder}"));
        }
        clause.push(')');
        clauses.push(clause);
    }

    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    let limit_placeholder = push_param(&mut params, i64::from(options.limit));
    sql.push_str(&format!(
        " ORDER BY created_at DESC, id ASC LIMIT {limit_placeholder}"
    ));

    db.query_all(&sql, &params, plan_from_row)
        .context("failed to run calibration query")
}

fn print_query_table(db: &Db, plans: &[PlanRow]) -> anyhow::Result<()> {
    let rows = plans
        .iter()
        .map(|plan| {
            let tags = load_tags(db, &plan.id)?;
            let summary = tags
                .iter()
                .map(|tag| format!("{}={}", tag.key, tag.value))
                .collect::<Vec<_>>()
                .join(" ");
            Ok(vec![
                plan.id.clone(),
                plan.created_at.to_string(),
                plan.name.clone(),
                plan.flavor.clone(),
                plan.worktype.clone().unwrap_or_default(),
                format::truncate_chars(&summary, 40),
            ])
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    print!(
        "{}",
        format::table(
            &["id", "created_at", "name", "flavor", "worktype", "tags"],
            &rows
        )
    );
    Ok(())
}

fn load_triggers(db: &Db, plan_id: &str) -> anyhow::Result<Vec<TriggerRow>> {
    db.query_all(
        "SELECT name, input_value, threshold, fired, section_added
         FROM triggers WHERE plan_id = $1 ORDER BY id",
        &[P::from(plan_id)],
        |row| {
            Ok(TriggerRow {
                name: row.get_string(0)?,
                input_value: row.get_f64(1)?,
                threshold: row.get_f64(2)?,
                fired: row.get_bool(3)?,
                section_added: row.get_optional_string(4)?,
            })
        },
    )
    .context("failed to load triggers")
}

fn load_phases(db: &Db, plan_id: &str) -> anyhow::Result<Vec<PhaseRow>> {
    db.query_all(
        "SELECT ordinal, slug, routing_tier, files
         FROM phases WHERE plan_id = $1 ORDER BY ordinal",
        &[P::from(plan_id)],
        |row| {
            let files = row.get_string(3)?;
            Ok(PhaseRow {
                ordinal: row.get_i64(0)?,
                slug: row.get_string(1)?,
                routing_tier: row.get_string(2)?,
                files: parse_json(&files, "phase files")?,
            })
        },
    )
    .context("failed to load phases")
}

fn load_tags(db: &Db, plan_id: &str) -> anyhow::Result<Vec<TagRow>> {
    db.query_all(
        "SELECT key, value FROM tags WHERE plan_id = $1 ORDER BY key, value",
        &[P::from(plan_id)],
        |row| {
            Ok(TagRow {
                key: row.get_string(0)?,
                value: row.get_string(1)?,
            })
        },
    )
    .context("failed to load tags")
}

fn load_verification(db: &Db, plan_id: &str) -> anyhow::Result<Option<VerificationRow>> {
    db.query_optional(
        "SELECT verified_at, elapsed_seconds, outcome, phase_outcomes, emergency_changes, surprises
         FROM verifications WHERE plan_id = $1 ORDER BY id DESC LIMIT 1",
        &[P::from(plan_id)],
        |row| {
            let phase_outcomes = row.get_string(3)?;
            let emergency_changes = row.get_optional_string(4)?;
            Ok(VerificationRow {
                verified_at: row.get_i64(0)?,
                elapsed_seconds: row.get_optional_i64(1)?,
                outcome: row.get_string(2)?,
                phase_outcomes: parse_json(&phase_outcomes, "phase outcomes")?,
                emergency_changes: emergency_changes
                    .as_deref()
                    .map(|raw| parse_json(raw, "emergency changes"))
                    .transpose()?,
                surprises: row.get_optional_string(5)?,
            })
        },
    )
    .context("failed to load verification")
}

fn plan_from_row(row: &DbRow) -> anyhow::Result<PlanRow> {
    let routing_dist = row.get_string(10)?;
    let capture_reasons = row.get_string(12)?;
    Ok(PlanRow {
        id: row.get_string(0)?,
        created_at: row.get_i64(1)?,
        name: row.get_string(2)?,
        path: row.get_string(3)?,
        flavor: row.get_string(4)?,
        worktype: row.get_optional_string(5)?,
        phase_count: row.get_i64(6)?,
        wave_count: row.get_i64(7)?,
        max_chain_depth: row.get_i64(8)?,
        repo_spread: row.get_i64(9)?,
        routing_dist: parse_json(&routing_dist, "routing_dist")?,
        shape_hash: row.get_string(11)?,
        capture_reasons: parse_json(&capture_reasons, "capture_reasons")?,
    })
}

fn push_param<'a>(params: &mut Vec<P<'a>>, value: impl Into<P<'a>>) -> String {
    let placeholder = format!("${}", params.len() + 1);
    params.push(value.into());
    placeholder
}

fn parse_json(raw: &str, label: &str) -> anyhow::Result<Value> {
    serde_json::from_str(raw).with_context(|| format!("malformed {label} JSON"))
}
