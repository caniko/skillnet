use rand::Rng;

use super::{inputs::PlanInputs, outcome::TriggerOutcome};
use crate::calibration::sidecar::VerifyRecord;

pub const UNIFORM_RANDOM_RATE: f64 = 0.07;

pub struct MetaHeuristicInputs<'a> {
    pub plan: &'a PlanInputs,
    pub triggers: &'a [TriggerOutcome],
    pub verify: Option<&'a VerifyRecord>,
    pub novel_shape_signature: bool,
    pub routing_tier_outlier_count: u32,
    pub rerouting_event_count: u32,
    pub random_sample: Option<f64>,
}

impl<'a> MetaHeuristicInputs<'a> {
    pub fn new(plan: &'a PlanInputs, triggers: &'a [TriggerOutcome]) -> Self {
        Self {
            plan,
            triggers,
            verify: None,
            novel_shape_signature: false,
            routing_tier_outlier_count: 0,
            rerouting_event_count: 0,
            random_sample: None,
        }
    }
}

pub trait MetaHeuristic: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn fires(&self, inputs: &MetaHeuristicInputs<'_>) -> bool;
}

pub static META_HEURISTICS: &[&dyn MetaHeuristic] = &[
    &ThresholdProximity,
    &TriggerAbsenceWithRiskShape,
    &NovelShapeSignature,
    &RoutingTierOutlier,
    &VerifySurprise,
    &ReroutingEvent,
    &HighStakesCombo,
    &UniformRandom,
];

macro_rules! meta_heuristic {
    ($type:ident, $name:literal, $description:literal, $fires:expr) => {
        pub struct $type;

        impl MetaHeuristic for $type {
            fn name(&self) -> &'static str {
                $name
            }

            fn description(&self) -> &'static str {
                $description
            }

            fn fires(&self, inputs: &MetaHeuristicInputs<'_>) -> bool {
                $fires(inputs)
            }
        }
    };
}

meta_heuristic!(
    ThresholdProximity,
    "threshold-proximity",
    "A trigger input sits within 20 percent of its active threshold.",
    threshold_proximity
);

meta_heuristic!(
    TriggerAbsenceWithRiskShape,
    "trigger-absence-with-risk-shape",
    "No triggers fired, but the plan shape is risky.",
    trigger_absence_with_risk_shape
);

meta_heuristic!(
    NovelShapeSignature,
    "novel-shape-signature",
    "The plan shape signature has not appeared in the dataset.",
    novel_shape_signature
);

meta_heuristic!(
    RoutingTierOutlier,
    "routing-tier-outlier",
    "A phase routes higher or lower than its declared complexity class median.",
    routing_tier_outlier
);

meta_heuristic!(
    VerifySurprise,
    "verify-surprise",
    "Verify recorded a surprise, emergency change, or added phase.",
    verify_surprise
);

meta_heuristic!(
    ReroutingEvent,
    "rerouting-event",
    "A phase was executed at a different tier than recommended.",
    rerouting_event
);

meta_heuristic!(
    HighStakesCombo,
    "high-stakes-combo",
    "The plan combines a max phase with external-repo work.",
    high_stakes_combo
);

pub struct UniformRandom;

impl MetaHeuristic for UniformRandom {
    fn name(&self) -> &'static str {
        "uniform-random"
    }

    fn description(&self) -> &'static str {
        "A 7 percent anti-bias random sample."
    }

    fn fires(&self, inputs: &MetaHeuristicInputs<'_>) -> bool {
        let sample = inputs
            .random_sample
            .unwrap_or_else(|| rand::thread_rng().gen::<f64>());
        sample < UNIFORM_RANDOM_RATE
    }
}

fn threshold_proximity(inputs: &MetaHeuristicInputs<'_>) -> bool {
    inputs.triggers.iter().any(|trigger| {
        if trigger.threshold == 0.0 {
            trigger.input_value == 0.0
        } else {
            ((trigger.input_value - trigger.threshold).abs() / trigger.threshold.abs()) <= 0.2
        }
    })
}

fn trigger_absence_with_risk_shape(inputs: &MetaHeuristicInputs<'_>) -> bool {
    !inputs.triggers.iter().any(|trigger| trigger.fired)
        && (inputs
            .plan
            .routing_dist
            .get("max")
            .copied()
            .unwrap_or_default()
            >= 1
            || inputs.plan.repo_spread >= 3
            || inputs.plan.max_chain_depth >= 3)
}

fn novel_shape_signature(inputs: &MetaHeuristicInputs<'_>) -> bool {
    inputs.novel_shape_signature
}

fn routing_tier_outlier(inputs: &MetaHeuristicInputs<'_>) -> bool {
    inputs.routing_tier_outlier_count > 0
}

fn verify_surprise(inputs: &MetaHeuristicInputs<'_>) -> bool {
    let Some(verify) = inputs.verify else {
        return false;
    };
    verify.emergency_changes.is_some()
        || verify.surprises.as_ref().is_some_and(|surprises| {
            let lower = surprises.to_ascii_lowercase();
            lower.contains("missed-signal")
                || lower.contains("failure")
                || lower.contains("emergency")
                || lower.contains("added phase")
                || lower.contains("phase added")
        })
}

fn rerouting_event(inputs: &MetaHeuristicInputs<'_>) -> bool {
    inputs.rerouting_event_count > 0
        || inputs
            .verify
            .and_then(|verify| verify.surprises.as_ref())
            .is_some_and(|surprises| {
                let lower = surprises.to_ascii_lowercase();
                lower.contains("rerouting")
                    || lower.contains("re-routing")
                    || lower.contains("different tier")
                    || lower.contains("executed at")
            })
}

fn high_stakes_combo(inputs: &MetaHeuristicInputs<'_>) -> bool {
    inputs
        .plan
        .routing_dist
        .get("max")
        .copied()
        .unwrap_or_default()
        >= 1
        && inputs
            .plan
            .phases
            .iter()
            .any(|phase| phase.working_tree.is_some())
}
