//! Analyze verified calibration rows and generate threshold recommendations.
//!
//! Calibration analysis output schema (SemVer-stable since 0.4.0):
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "analyzed_at": "YYYY-MM-DD HH:MM:SS",
//!   "min_n": 10,
//!   "dataset_size": 0,
//!   "filter_tags": [{"key": "flavor", "value": "codex"}],
//!   "filter_tags_applied": [{"key": "flavor", "value": "codex"}],
//!   "triggers": [{
//!     "trigger": "long-serial-chain",
//!     "fires": 2,
//!     "misses": 1,
//!     "fire_rate": 0.6666666666666666,
//!     "signal_rate": 0.5,
//!     "verdict": "hold",
//!     "default_threshold": 4.0,
//!     "current_threshold": 4.0,
//!     "threshold_source": {"type": "default"},
//!     "true_positives": 2,
//!     "false_positives": 0,
//!     "false_negatives": 1,
//!     "true_negatives": 0,
//!     "supporting_plan_ids": []
//!   }],
//!   "proposals": [],
//!   "skew_warnings": []
//! }
//! ```
//!
//! See `docs/src/calibration/json-schema.md` for the canonical field
//! reference and SemVer commitment.
//!
//! The analyzer scores only plans that have a verification row. For each
//! trigger `T`:
//!
//! - `N_fires` = number of verified plan rows where `T.fired = true`.
//! - `N_misses` = number of verified plan rows where `T.fired = false`.
//! - `fire_rate = N_fires / (N_fires + N_misses)`.
//! - `helpful(T, plan)` = `T.fired` and `verification.outcome` is `shipped`
//!   or `partial`, and the verifier did not list `T.name` as dead weight.
//! - `failure_mode(T, plan)` = the verifier listed `T.name` as a missed
//!   signal, or the `emergency_changes` JSON contains a hint matching the
//!   trigger's name or section scope.
//!
//! Surprise text is intentionally not NLP-parsed. The supported structured
//! prefixes are `dead-weight: <trigger-name>: <note>` for false positives and
//! `missed-signal: <expected-trigger-name>: <note>` for false negatives.
//! Other surprise lines are informational only.
//!
//! The custom signal score is:
//!
//! ```text
//! true_positives  = count(fired AND helpful)
//! false_positives = count(fired AND NOT helpful)
//! false_negatives = count(NOT fired AND failure_mode)
//! true_negatives  = count(NOT fired AND NOT failure_mode)
//! signal_rate = (true_positives - false_positives - false_negatives)
//!               / max(1, true_positives + false_positives + false_negatives)
//! ```
//!
//! The range is `[-1, +1]`. Negative means the trigger is net harmful, near
//! zero means wasted ceremony, and positive means the trigger earns its place.
//! Proposals are suppressed until `N_fires >= min_n`; early silence is
//! expected, because the analyzer should not recommend threshold changes from
//! a few data points.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use serde::Serialize;

use super::{
    catalog::{ThresholdSource, ThresholdStore},
    db::DbParam as P,
    Db,
};

const ANALYZE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub struct AnalyzeOptions {
    pub filter_tags: Vec<(String, String)>,
    pub trigger: Option<String>,
    pub min_n: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeReport {
    pub schema_version: u32,
    pub analyzed_at: String,
    pub min_n: u32,
    pub dataset_size: u32,
    pub filter_tags: Vec<TagFilter>,
    pub filter_tags_applied: Vec<TagFilter>,
    pub triggers: Vec<TriggerAnalysis>,
    pub proposals: Vec<ThresholdProposal>,
    pub skew_warnings: Vec<SkewWarning>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TagFilter {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct TriggerAnalysis {
    pub trigger: String,
    pub fires: u32,
    pub misses: u32,
    pub fire_rate: f64,
    pub signal_rate: Option<f64>,
    pub verdict: String,
    pub default_threshold: Option<f64>,
    pub current_threshold: Option<f64>,
    pub threshold_source: Option<ThresholdSource>,
    pub true_positives: u32,
    pub false_positives: u32,
    pub false_negatives: u32,
    pub true_negatives: u32,
    pub supporting_plan_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ThresholdProposal {
    pub trigger: String,
    pub action: ProposalAction,
    pub current_threshold: f64,
    pub proposed_threshold: f64,
    pub fire_rate: f64,
    pub signal_rate: f64,
    pub supporting_plan_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalAction {
    LowerThreshold,
    RaiseThreshold,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkewWarning {
    pub trigger: String,
    pub tag_key: String,
    pub tag_value: String,
    pub global_signal_rate: f64,
    pub band_signal_rate: f64,
    pub band_fires: u32,
    pub message: String,
}

#[derive(Clone, Debug)]
struct TriggerRow {
    plan_id: String,
    name: String,
    threshold: f64,
    fired: bool,
    section_added: Option<String>,
    created_at: i64,
    outcome: String,
    surprises: Option<String>,
    emergency_changes: Option<String>,
}

#[derive(Debug)]
struct RawStats {
    trigger: String,
    fires: u32,
    misses: u32,
    fire_rate: f64,
    signal_rate: f64,
    default_threshold: Option<f64>,
    current_threshold: Option<f64>,
    threshold_source: Option<ThresholdSource>,
    true_positives: u32,
    false_positives: u32,
    false_negatives: u32,
    true_negatives: u32,
    false_positive_plan_ids: Vec<String>,
    false_negative_plan_ids: Vec<String>,
}

pub fn run(db: &Db, options: AnalyzeOptions, format: OutputFormat) -> anyhow::Result<()> {
    let report = analyze(db, &options)?;
    match format {
        OutputFormat::Table => print_table(&report),
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
}

pub fn analyze(db: &Db, options: &AnalyzeOptions) -> anyhow::Result<AnalyzeReport> {
    let thresholds = ThresholdStore::load(db)?;
    let tags = load_tags(db)?;
    let rows = load_rows(db, options, &tags)?;
    let mut by_trigger: BTreeMap<String, Vec<TriggerRow>> = BTreeMap::new();
    for row in rows {
        by_trigger.entry(row.name.clone()).or_default().push(row);
    }

    let mut triggers = Vec::new();
    let mut proposals = Vec::new();
    for (trigger, rows) in by_trigger {
        let mut raw = compute_raw_stats(&trigger, &rows);
        if let Some(active_threshold) = thresholds.get_optional(&trigger) {
            raw.default_threshold = thresholds.default_threshold(&trigger);
            raw.current_threshold = Some(active_threshold);
            raw.threshold_source = thresholds.source(&trigger);
        }
        let (analysis, proposal) = finalize_stats(raw, options.min_n);
        if let Some(proposal) = proposal {
            proposals.push(proposal);
        }
        triggers.push(analysis);
    }

    let skew_warnings = if options.filter_tags.is_empty() {
        skew_warnings(db, options, &triggers)?
    } else {
        Vec::new()
    };

    let filter_tags = options
        .filter_tags
        .iter()
        .map(|(key, value)| TagFilter {
            key: key.clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    let dataset_size = triggers
        .iter()
        .map(|trigger| trigger.fires + trigger.misses)
        .sum();

    Ok(AnalyzeReport {
        schema_version: ANALYZE_SCHEMA_VERSION,
        analyzed_at: analyzed_at(db)?,
        min_n: options.min_n,
        dataset_size,
        filter_tags: filter_tags.clone(),
        filter_tags_applied: filter_tags,
        triggers,
        proposals,
        skew_warnings,
    })
}

fn analyzed_at(db: &Db) -> anyhow::Result<String> {
    db.query_one("SELECT CURRENT_TIMESTAMP", &[], |row| row.get_string(0))
        .context("failed to compute analysis timestamp")
}

pub fn latest_threshold(
    db: &Db,
    trigger: &str,
    filter_tags: &[(String, String)],
) -> anyhow::Result<Option<f64>> {
    let thresholds = ThresholdStore::load(db)?;
    if let Some(threshold) = thresholds.get_optional(trigger) {
        return Ok(Some(threshold));
    }

    let tags = load_tags(db)?;
    let rows = load_rows(
        db,
        &AnalyzeOptions {
            filter_tags: filter_tags.to_vec(),
            trigger: Some(trigger.to_string()),
            min_n: 1,
        },
        &tags,
    )?;
    Ok(rows
        .into_iter()
        .max_by_key(|row| row.created_at)
        .map(|row| row.threshold))
}

fn load_rows(
    db: &Db,
    options: &AnalyzeOptions,
    tags: &BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
) -> anyhow::Result<Vec<TriggerRow>> {
    let rows = db.query_all(
        "SELECT
            t.plan_id,
            t.name,
            t.threshold,
            t.fired,
            t.section_added,
            p.created_at,
            v.outcome,
            v.surprises,
            v.emergency_changes
        FROM triggers t
        JOIN plans p ON p.id = t.plan_id
        JOIN verifications v ON v.plan_id = t.plan_id
        ORDER BY p.created_at, t.id",
        &[],
        |row| {
            Ok(TriggerRow {
                plan_id: row.get_string(0)?,
                name: row.get_string(1)?,
                threshold: row.get_f64(2)?,
                fired: row.get_bool(3)?,
                section_added: row.get_optional_string(4)?,
                created_at: row.get_i64(5)?,
                outcome: row.get_string(6)?,
                surprises: row.get_optional_string(7)?,
                emergency_changes: row.get_optional_string(8)?,
            })
        },
    )?;

    let mut out = Vec::new();
    for row in rows {
        if options
            .trigger
            .as_ref()
            .is_some_and(|trigger| trigger != &row.name)
        {
            continue;
        }
        if !matches_filters(&row.plan_id, &options.filter_tags, tags) {
            continue;
        }
        out.push(row);
    }
    Ok(out)
}

fn load_tags(db: &Db) -> anyhow::Result<BTreeMap<String, BTreeMap<String, BTreeSet<String>>>> {
    let rows = db.query_all(
        "SELECT plan_id, key, value FROM tags ORDER BY plan_id, key, value",
        &[],
        |row| Ok((row.get_string(0)?, row.get_string(1)?, row.get_string(2)?)),
    )?;

    let mut tags: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for (plan_id, key, value) in rows {
        tags.entry(plan_id)
            .or_default()
            .entry(key)
            .or_default()
            .insert(value);
    }
    Ok(tags)
}

fn matches_filters(
    plan_id: &str,
    filters: &[(String, String)],
    tags: &BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
) -> bool {
    filters.iter().all(|(key, value)| {
        tags.get(plan_id)
            .and_then(|plan_tags| plan_tags.get(key))
            .is_some_and(|values| values.contains(value))
    })
}

fn compute_raw_stats(trigger: &str, rows: &[TriggerRow]) -> RawStats {
    let mut true_positives = 0;
    let mut false_positives = 0;
    let mut false_negatives = 0;
    let mut true_negatives = 0;
    let mut fires = 0;
    let mut misses = 0;
    let mut false_positive_plan_ids = Vec::new();
    let mut false_negative_plan_ids = Vec::new();

    for row in rows {
        let dead_weight = has_structured_surprise(row.surprises.as_deref(), "dead-weight", trigger);
        let missed_signal =
            has_structured_surprise(row.surprises.as_deref(), "missed-signal", trigger);
        let failure_mode = missed_signal || emergency_matches(row, trigger);
        let helpful =
            row.fired && matches!(row.outcome.as_str(), "shipped" | "partial") && !dead_weight;

        if row.fired {
            fires += 1;
            if helpful {
                true_positives += 1;
            } else {
                false_positives += 1;
                false_positive_plan_ids.push(row.plan_id.clone());
            }
        } else {
            misses += 1;
            if failure_mode {
                false_negatives += 1;
                false_negative_plan_ids.push(row.plan_id.clone());
            } else {
                true_negatives += 1;
            }
        }
    }

    let denominator = (true_positives + false_positives + false_negatives).max(1);
    let signal_rate =
        f64::from(true_positives) - f64::from(false_positives) - f64::from(false_negatives);
    let signal_rate = signal_rate / f64::from(denominator);
    let total = fires + misses;
    let fire_rate = if total == 0 {
        0.0
    } else {
        f64::from(fires) / f64::from(total)
    };

    RawStats {
        trigger: trigger.to_string(),
        fires,
        misses,
        fire_rate,
        signal_rate,
        current_threshold: rows
            .iter()
            .max_by_key(|row| row.created_at)
            .map(|row| row.threshold),
        default_threshold: None,
        threshold_source: None,
        true_positives,
        false_positives,
        false_negatives,
        true_negatives,
        false_positive_plan_ids,
        false_negative_plan_ids,
    }
}

fn finalize_stats(raw: RawStats, min_n: u32) -> (TriggerAnalysis, Option<ThresholdProposal>) {
    if raw.fires < min_n {
        let analysis = TriggerAnalysis {
            verdict: format!("n={} < min-n={min_n}", raw.fires),
            signal_rate: None,
            supporting_plan_ids: Vec::new(),
            trigger: raw.trigger,
            fires: raw.fires,
            misses: raw.misses,
            fire_rate: raw.fire_rate,
            current_threshold: raw.current_threshold,
            default_threshold: raw.default_threshold,
            threshold_source: raw.threshold_source,
            true_positives: raw.true_positives,
            false_positives: raw.false_positives,
            false_negatives: raw.false_negatives,
            true_negatives: raw.true_negatives,
        };
        return (analysis, None);
    }

    let proposal = raw.current_threshold.and_then(|current_threshold| {
        if raw.signal_rate >= 0.3 && raw.false_negatives >= 2 {
            let proposed_threshold = current_threshold - notch_size(&raw.trigger);
            Some(ThresholdProposal {
                trigger: raw.trigger.clone(),
                action: ProposalAction::LowerThreshold,
                current_threshold,
                proposed_threshold,
                fire_rate: raw.fire_rate,
                signal_rate: raw.signal_rate,
                supporting_plan_ids: raw.false_negative_plan_ids.clone(),
            })
        } else if raw.signal_rate <= 0.0 && raw.fire_rate >= 0.5 {
            let proposed_threshold = current_threshold + notch_size(&raw.trigger);
            Some(ThresholdProposal {
                trigger: raw.trigger.clone(),
                action: ProposalAction::RaiseThreshold,
                current_threshold,
                proposed_threshold,
                fire_rate: raw.fire_rate,
                signal_rate: raw.signal_rate,
                supporting_plan_ids: raw.false_positive_plan_ids.clone(),
            })
        } else {
            None
        }
    });

    let verdict = proposal
        .as_ref()
        .map_or_else(|| "hold".to_string(), proposal_verdict);
    let supporting_plan_ids = proposal
        .as_ref()
        .map(|proposal| proposal.supporting_plan_ids.clone())
        .unwrap_or_default();

    let analysis = TriggerAnalysis {
        trigger: raw.trigger,
        fires: raw.fires,
        misses: raw.misses,
        fire_rate: raw.fire_rate,
        signal_rate: Some(raw.signal_rate),
        verdict,
        current_threshold: raw.current_threshold,
        default_threshold: raw.default_threshold,
        threshold_source: raw.threshold_source,
        true_positives: raw.true_positives,
        false_positives: raw.false_positives,
        false_negatives: raw.false_negatives,
        true_negatives: raw.true_negatives,
        supporting_plan_ids,
    };
    (analysis, proposal)
}

fn proposal_verdict(proposal: &ThresholdProposal) -> String {
    let old = pretty_threshold(proposal.current_threshold);
    let new = pretty_threshold(proposal.proposed_threshold);
    match proposal.action {
        ProposalAction::LowerThreshold => format!("lower threshold ({old}->{new})"),
        ProposalAction::RaiseThreshold => format!("raise threshold ({old}->{new})"),
    }
}

fn skew_warnings(
    db: &Db,
    options: &AnalyzeOptions,
    global_triggers: &[TriggerAnalysis],
) -> anyhow::Result<Vec<SkewWarning>> {
    let mut warnings = Vec::new();
    for tag_key in ["flavor", "worktype"] {
        for tag_value in tag_values(db, tag_key)? {
            let mut band_filters = options.filter_tags.clone();
            band_filters.push((tag_key.to_string(), tag_value.clone()));
            let band = analyze(
                db,
                &AnalyzeOptions {
                    filter_tags: band_filters,
                    trigger: options.trigger.clone(),
                    min_n: 1,
                },
            )?;
            for global in global_triggers {
                let Some(global_signal_rate) = global.signal_rate else {
                    continue;
                };
                let Some(band_trigger) = band
                    .triggers
                    .iter()
                    .find(|candidate| candidate.trigger == global.trigger)
                else {
                    continue;
                };
                let Some(band_signal_rate) = band_trigger.signal_rate else {
                    continue;
                };
                if band_trigger.fires >= 30 && (band_signal_rate - global_signal_rate).abs() > 0.3 {
                    warnings.push(SkewWarning {
                        trigger: global.trigger.clone(),
                        tag_key: tag_key.to_string(),
                        tag_value: tag_value.clone(),
                        global_signal_rate,
                        band_signal_rate,
                        band_fires: band_trigger.fires,
                        message: format!(
                            "trigger {} shows skew across {tag_key}: consider per-band thresholds",
                            global.trigger
                        ),
                    });
                }
            }
        }
    }
    Ok(warnings)
}

fn tag_values(db: &Db, key: &str) -> anyhow::Result<Vec<String>> {
    db.query_all(
        "SELECT DISTINCT value FROM tags WHERE key = $1 ORDER BY value",
        &[P::from(key)],
        |row| row.get_string(0),
    )
    .with_context(|| format!("failed to read {key} tag values"))
}

fn has_structured_surprise(surprises: Option<&str>, prefix: &str, trigger: &str) -> bool {
    surprises
        .into_iter()
        .flat_map(str::lines)
        .filter_map(parse_surprise_line)
        .any(|(line_prefix, line_trigger)| line_prefix == prefix && line_trigger == trigger)
}

fn parse_surprise_line(line: &str) -> Option<(&str, &str)> {
    let (prefix, rest) = line.trim().split_once(':')?;
    let (trigger, _) = rest.trim().split_once(':')?;
    Some((prefix.trim(), trigger.trim()))
}

fn emergency_matches(row: &TriggerRow, trigger: &str) -> bool {
    let Some(emergency_changes) = &row.emergency_changes else {
        return false;
    };
    emergency_changes.contains(trigger)
        || row
            .section_added
            .as_ref()
            .is_some_and(|section| emergency_changes.contains(section))
}

fn notch_size(trigger: &str) -> f64 {
    match trigger {
        "trivial-nontrivial-ratio" | "trivial-non-trivial-ratio" | "trivial:non-trivial" => 0.5,
        _ => 1.0,
    }
}

pub fn pretty_threshold(value: f64) -> String {
    if (value.fract()).abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn print_table(report: &AnalyzeReport) {
    println!(
        "{:<28} {:>5} {:>6} {:>7} {:>8}  VERDICT",
        "TRIGGER", "FIRES", "MISSES", "FIRE%", "SIGNAL"
    );
    for trigger in &report.triggers {
        println!(
            "{:<28} {:>5} {:>6} {:>7} {:>8}  {}",
            trigger.trigger,
            trigger.fires,
            trigger.misses,
            format!("{:.1}%", trigger.fire_rate * 100.0),
            format_signal(trigger.signal_rate),
            trigger.verdict
        );
    }

    println!("PROPOSALS ({}):", report.proposals.len());
    for proposal in &report.proposals {
        println!(
            "  - {}: {} -> {}",
            proposal.trigger,
            pretty_threshold(proposal.current_threshold),
            pretty_threshold(proposal.proposed_threshold)
        );
        println!(
            "    fire% {:.1}% / signal {:+.2}",
            proposal.fire_rate * 100.0,
            proposal.signal_rate
        );
        println!(
            "    supporting plans: {} (run `skillnet calibration query ...`)",
            proposal.supporting_plan_ids.len()
        );
        println!("    run `skillnet calibration propose ...` to formalize");
    }

    println!("SKEW WARNINGS ({}):", report.skew_warnings.len());
    for warning in &report.skew_warnings {
        println!(
            "  - {} [{}={}]: global {:+.2}, band {:+.2}, fires {}",
            warning.message,
            warning.tag_key,
            warning.tag_value,
            warning.global_signal_rate,
            warning.band_signal_rate,
            warning.band_fires
        );
    }
}

fn format_signal(signal: Option<f64>) -> String {
    signal.map_or_else(|| "n/a".to_string(), |signal| format!("{signal:+.2}"))
}
