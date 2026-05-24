use std::collections::{BTreeMap, BTreeSet};

use super::{inputs::PlanInputs, outcome::TriggerOutcome};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum HeuristicCategory {
    Coordination,
    Risk,
    PlanShape,
    QualityLint,
}

impl HeuristicCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Coordination => "coordination",
            Self::Risk => "risk",
            Self::PlanShape => "plan-shape",
            Self::QualityLint => "quality-lint",
        }
    }
}

pub trait Heuristic: Send + Sync {
    fn name(&self) -> &'static str;
    fn category(&self) -> HeuristicCategory;
    fn default_threshold(&self) -> f64;
    fn description(&self) -> &'static str;
    fn section_added(&self, fired: bool) -> Option<&'static str>;
    fn evaluate(&self, plan: &PlanInputs, threshold: f64) -> TriggerOutcome;
}

pub struct Catalog;

impl Catalog {
    pub fn get(&self, name: &str) -> Option<&'static dyn Heuristic> {
        HEURISTICS
            .iter()
            .copied()
            .find(|heuristic| heuristic.name() == name)
    }

    pub fn by_category(
        &self,
        category: HeuristicCategory,
    ) -> impl Iterator<Item = &'static dyn Heuristic> {
        HEURISTICS
            .iter()
            .copied()
            .filter(move |heuristic| heuristic.category() == category)
    }

    pub fn iter(&self) -> impl Iterator<Item = &'static dyn Heuristic> {
        HEURISTICS.iter().copied()
    }
}

pub static CATALOG: Catalog = Catalog;

pub static HEURISTICS: &[&dyn Heuristic] = &[
    &SharedFileContention,
    &ExternalRepoPhases,
    &ConvergencePoint,
    &OwnershipBoundarySpread,
    &RiskConcentration,
    &RiskLateInPlan,
    &InfrastructureSpof,
    &RevendorPhase,
    &LongSerialChain,
    &MidPlanRerouting,
    &TrivialPhaseSwamp,
    &NoIntegratedVerification,
    &RoutingTierInversion,
    &MechanicalStreak,
    &HiddenPrerequisite,
];

macro_rules! outcome {
    ($heuristic:expr, $input:expr, $threshold:expr) => {{
        let input_value = $input;
        let threshold = $threshold;
        let fired = input_value >= threshold;
        TriggerOutcome {
            input_value,
            threshold,
            fired,
            section_added: $heuristic.section_added(fired).map(str::to_string),
        }
    }};
}

macro_rules! heuristic {
    ($type:ident, $name:literal, $category:ident, $threshold:literal, $description:literal, $section:expr, $eval:expr) => {
        pub struct $type;

        impl Heuristic for $type {
            fn name(&self) -> &'static str {
                $name
            }

            fn category(&self) -> HeuristicCategory {
                HeuristicCategory::$category
            }

            fn default_threshold(&self) -> f64 {
                $threshold
            }

            fn description(&self) -> &'static str {
                $description
            }

            fn section_added(&self, fired: bool) -> Option<&'static str> {
                fired.then_some($section).flatten()
            }

            fn evaluate(&self, plan: &PlanInputs, threshold: f64) -> TriggerOutcome {
                outcome!(self, $eval(plan), threshold)
            }
        }
    };
}

heuristic!(
    SharedFileContention,
    "shared-file-contention",
    Coordination,
    2.0,
    "Two or more phases touch the same file.",
    Some("Shared-file lockstep"),
    shared_file_contention
);

heuristic!(
    ExternalRepoPhases,
    "external-repo-phases",
    Coordination,
    1.0,
    "At least one phase works outside the primary working tree.",
    Some("External repo coordination"),
    external_repo_phases
);

heuristic!(
    ConvergencePoint,
    "convergence-point",
    Coordination,
    3.0,
    "A phase has three or more direct predecessors.",
    Some("Merge-readiness checklist"),
    convergence_point
);

heuristic!(
    OwnershipBoundarySpread,
    "ownership-boundary-spread",
    Coordination,
    2.0,
    "Phases span multiple repository or maintainer boundaries.",
    Some("PR sequencing & cross-owner coordination"),
    ownership_boundary_spread
);

heuristic!(
    RiskConcentration,
    "risk-concentration",
    Risk,
    2.0,
    "Two or more phases are routed to max.",
    Some("Risk-tier callout"),
    risk_concentration
);

heuristic!(
    RiskLateInPlan,
    "risk-late-in-plan",
    Risk,
    1.0,
    "A max-risk phase sits in the final third of waves.",
    Some("Late-risk warning"),
    risk_late_in_plan
);

heuristic!(
    InfrastructureSpof,
    "infrastructure-spof",
    Risk,
    1.0,
    "An infrastructure phase has downstream dependents.",
    Some("infra-SPOF"),
    infrastructure_spof
);

heuristic!(
    RevendorPhase,
    "revendor-phase",
    Risk,
    1.0,
    "A phase touches vendoring, dependency bumps, or lockfiles.",
    Some("Compat surface"),
    revendor_phase
);

heuristic!(
    LongSerialChain,
    "long-serial-chain",
    PlanShape,
    4.0,
    "The dependency chain is at least four phases deep.",
    Some("Serial-chain recovery"),
    long_serial_chain
);

heuristic!(
    MidPlanRerouting,
    "mid-plan-rerouting",
    PlanShape,
    10.0,
    "The plan has ten or more phases.",
    Some("Mid-plan re-routing checkpoint"),
    mid_plan_rerouting
);

heuristic!(
    TrivialPhaseSwamp,
    "trivial-phase-swamp",
    PlanShape,
    4.0,
    "Low/medium phases outnumber high/max phases by at least four to one.",
    Some("Cleanup batch"),
    trivial_phase_swamp
);

heuristic!(
    NoIntegratedVerification,
    "no-integrated-verification",
    PlanShape,
    1.0,
    "No phase appears to exercise the end-to-end outcome.",
    None,
    no_integrated_verification
);

heuristic!(
    RoutingTierInversion,
    "routing-tier-inversion",
    QualityLint,
    1.0,
    "A leaf phase routes at least as high as the plan orchestrator.",
    None,
    routing_tier_inversion
);

heuristic!(
    MechanicalStreak,
    "mechanical-streak",
    QualityLint,
    3.0,
    "Three or more consecutive phases are routed low.",
    None,
    mechanical_streak
);

heuristic!(
    HiddenPrerequisite,
    "hidden-prerequisite",
    QualityLint,
    1.0,
    "A phase has an invalid or implicit prerequisite edge.",
    None,
    hidden_prerequisite
);

fn shared_file_contention(plan: &PlanInputs) -> f64 {
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for phase in &plan.phases {
        for file in &phase.files {
            *counts.entry(file.as_str()).or_default() += 1;
        }
    }
    counts.values().copied().max().unwrap_or(0).into()
}

fn external_repo_phases(plan: &PlanInputs) -> f64 {
    plan.phases
        .iter()
        .filter(|phase| phase.working_tree.is_some())
        .count() as f64
}

fn convergence_point(plan: &PlanInputs) -> f64 {
    plan.phases
        .iter()
        .map(|phase| phase.depends_on.len())
        .max()
        .unwrap_or(0) as f64
}

fn ownership_boundary_spread(plan: &PlanInputs) -> f64 {
    let mut working_trees = plan
        .phases
        .iter()
        .filter_map(|phase| phase.working_tree.as_deref())
        .collect::<BTreeSet<_>>();
    if plan.repo_spread > 0 {
        f64::from(plan.repo_spread)
    } else if working_trees.is_empty() {
        1.0
    } else {
        working_trees.insert("primary");
        working_trees.len() as f64
    }
}

fn risk_concentration(plan: &PlanInputs) -> f64 {
    f64::from(plan.routing_dist.get("max").copied().unwrap_or(0))
}

fn risk_late_in_plan(plan: &PlanInputs) -> f64 {
    if plan.waves.is_empty() {
        return 0.0;
    }

    let first_late_wave = (plan.waves.len() * 2) / 3;
    let max_phase_ordinals = plan
        .phases
        .iter()
        .filter(|phase| phase.routing_tier == "max")
        .map(|phase| phase.ordinal)
        .collect::<BTreeSet<_>>();
    plan.waves
        .iter()
        .enumerate()
        .filter(|(index, wave)| {
            *index >= first_late_wave
                && wave
                    .iter()
                    .any(|ordinal| max_phase_ordinals.contains(ordinal))
        })
        .count() as f64
}

fn infrastructure_spof(plan: &PlanInputs) -> f64 {
    let infra_ordinals = plan
        .phases
        .iter()
        .filter(|phase| phase.files.iter().any(|file| is_infra_file(file)))
        .map(|phase| phase.ordinal)
        .collect::<BTreeSet<_>>();
    if infra_ordinals.is_empty() {
        return 0.0;
    }
    plan.phases
        .iter()
        .filter(|phase| {
            phase
                .depends_on
                .iter()
                .any(|ordinal| infra_ordinals.contains(ordinal))
        })
        .count() as f64
}

fn revendor_phase(plan: &PlanInputs) -> f64 {
    plan.phases
        .iter()
        .filter(|phase| {
            let slug = phase.slug.to_ascii_lowercase();
            mentions_dependency_surface(&slug)
                || phase
                    .files
                    .iter()
                    .any(|file| mentions_dependency_surface(&file.to_ascii_lowercase()))
        })
        .count() as f64
}

fn long_serial_chain(plan: &PlanInputs) -> f64 {
    f64::from(plan.max_chain_depth)
}

fn mid_plan_rerouting(plan: &PlanInputs) -> f64 {
    f64::from(plan.phase_count)
}

fn trivial_phase_swamp(plan: &PlanInputs) -> f64 {
    let trivial = plan.routing_dist.get("low").copied().unwrap_or(0)
        + plan.routing_dist.get("medium").copied().unwrap_or(0);
    let non_trivial = plan.routing_dist.get("high").copied().unwrap_or(0)
        + plan.routing_dist.get("max").copied().unwrap_or(0);
    if non_trivial == 0 {
        f64::from(trivial)
    } else {
        f64::from(trivial) / f64::from(non_trivial)
    }
}

fn no_integrated_verification(plan: &PlanInputs) -> f64 {
    if plan.phases.iter().any(looks_like_integrated_verification) {
        0.0
    } else {
        1.0
    }
}

fn routing_tier_inversion(plan: &PlanInputs) -> f64 {
    let orchestrator_rank = plan
        .routing_dist
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(tier, _)| tier_rank(tier))
        .max()
        .unwrap_or(0);
    plan.phases
        .iter()
        .filter(|phase| {
            let is_leaf = !plan
                .phases
                .iter()
                .any(|candidate| candidate.depends_on.contains(&phase.ordinal));
            is_leaf && tier_rank(&phase.routing_tier) >= orchestrator_rank && orchestrator_rank > 0
        })
        .count() as f64
}

fn mechanical_streak(plan: &PlanInputs) -> f64 {
    let mut longest = 0_u32;
    let mut current = 0_u32;
    let mut phases = plan.phases.iter().collect::<Vec<_>>();
    phases.sort_by_key(|phase| phase.ordinal);
    for phase in phases {
        if phase.routing_tier == "low" {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    f64::from(longest)
}

fn hidden_prerequisite(plan: &PlanInputs) -> f64 {
    let ordinals = plan
        .phases
        .iter()
        .map(|phase| phase.ordinal)
        .collect::<BTreeSet<_>>();
    plan.phases
        .iter()
        .flat_map(|phase| &phase.depends_on)
        .filter(|ordinal| !ordinals.contains(ordinal))
        .count() as f64
}

fn is_infra_file(file: &str) -> bool {
    let file = file.to_ascii_lowercase();
    file.contains(".github/workflows/")
        || file.contains(".forgejo/workflows/")
        || file.contains("/ci/")
        || file.ends_with("flake.nix")
        || file.ends_with("flake.lock")
        || file.ends_with("cargo.lock")
        || file.ends_with("package-lock.json")
        || file.ends_with("pnpm-lock.yaml")
        || file.ends_with("yarn.lock")
        || file.contains("build.rs")
        || file.contains("justfile")
        || file.contains("makefile")
}

fn mentions_dependency_surface(value: &str) -> bool {
    value.contains("vendor")
        || value.contains("vendoring")
        || value.contains("bump")
        || value.contains("dependency")
        || value.contains("dependencies")
        || value.contains("lockfile")
        || value.ends_with("cargo.lock")
        || value.ends_with("flake.lock")
        || value.ends_with("package-lock.json")
        || value.ends_with("pnpm-lock.yaml")
        || value.ends_with("yarn.lock")
}

fn looks_like_integrated_verification(phase: &super::inputs::PhaseInputs) -> bool {
    let slug = phase.slug.to_ascii_lowercase();
    slug.contains("verify")
        || slug.contains("verification")
        || slug.contains("e2e")
        || slug.contains("end-to-end")
        || slug.contains("integration")
        || phase.files.iter().any(|file| {
            let file = file.to_ascii_lowercase();
            file.contains("e2e") || file.contains("integration") || file.contains("smoke")
        })
}

fn tier_rank(tier: &str) -> u8 {
    match tier {
        "low" | "5.5 low" => 1,
        "medium" | "5.5 medium" => 2,
        "high" | "5.5 high" => 3,
        "max" | "5.5 max" => 4,
        _ => 0,
    }
}
