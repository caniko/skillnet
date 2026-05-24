use std::collections::BTreeMap;

use serde_json::json;
use skillnet::calibration::{
    catalog::{
        MetaHeuristicInputs, PhaseInputs, PlanInputs, ThresholdSource, ThresholdStore, CATALOG,
        HEURISTICS, META_HEURISTICS,
    },
    sidecar::VerifyRecord,
    Db,
};
use tempfile::tempdir;

#[test]
fn catalog_is_readable_by_name_category_and_iteration() {
    let heuristic = CATALOG.get("long-serial-chain").unwrap();
    assert_eq!(heuristic.default_threshold(), 4.0);
    assert_eq!(CATALOG.by_category(heuristic.category()).count(), 4);
    assert_eq!(CATALOG.iter().count(), HEURISTICS.len());
}

#[test]
fn every_heuristic_fires_and_misses_on_synthetic_inputs() {
    for case in heuristic_cases() {
        let heuristic = CATALOG.get(case.name).unwrap();
        let threshold = heuristic.default_threshold();

        let fired = heuristic.evaluate(&case.firing, threshold);
        assert!(fired.fired, "{} should fire, got {fired:?}", case.name);
        assert!(fired.input_value >= threshold);

        let missed = heuristic.evaluate(&case.missing, threshold);
        assert!(!missed.fired, "{} should miss, got {missed:?}", case.name);
        assert!(missed.input_value < threshold);
    }
}

#[test]
fn meta_heuristics_fire_and_miss_on_synthetic_inputs() {
    let plan = plan_with(vec![phase(1, "01-setup", "medium", &["src/lib.rs"])]);
    let near = vec![outcome(9.0, 10.0, false)];
    let far = vec![outcome(1.0, 10.0, false)];
    assert_meta("threshold-proximity", inputs(&plan, &near), true);
    assert_meta("threshold-proximity", inputs(&plan, &far), false);

    let mut risky = plan.clone();
    risky.routing_dist.insert("max".to_string(), 1);
    assert_meta(
        "trigger-absence-with-risk-shape",
        inputs(&risky, &[outcome(0.0, 1.0, false)]),
        true,
    );
    assert_meta(
        "trigger-absence-with-risk-shape",
        inputs(&risky, &[outcome(1.0, 1.0, true)]),
        false,
    );

    let mut novel = inputs(&plan, &far);
    novel.novel_shape_signature = true;
    assert_meta("novel-shape-signature", novel, true);
    assert_meta("novel-shape-signature", inputs(&plan, &far), false);

    let mut outlier = inputs(&plan, &far);
    outlier.routing_tier_outlier_count = 1;
    assert_meta("routing-tier-outlier", outlier, true);
    assert_meta("routing-tier-outlier", inputs(&plan, &far), false);

    let verify = VerifyRecord {
        verified_at: 1,
        elapsed_seconds: None,
        outcome: "partial".to_string(),
        phase_outcomes: BTreeMap::new(),
        emergency_changes: Some(json!({"phase": "added"})),
        surprises: None,
    };
    let mut verify_inputs = inputs(&plan, &far);
    verify_inputs.verify = Some(&verify);
    assert_meta("verify-surprise", verify_inputs, true);
    assert_meta("verify-surprise", inputs(&plan, &far), false);

    let mut rerouting = inputs(&plan, &far);
    rerouting.rerouting_event_count = 1;
    assert_meta("rerouting-event", rerouting, true);
    assert_meta("rerouting-event", inputs(&plan, &far), false);

    let mut high_stakes = risky.clone();
    high_stakes.phases[0].working_tree = Some("/tmp/external".to_string());
    assert_meta("high-stakes-combo", inputs(&high_stakes, &far), true);
    assert_meta("high-stakes-combo", inputs(&risky, &far), false);

    let mut sampled = inputs(&plan, &far);
    sampled.random_sample = Some(0.01);
    assert_meta("uniform-random", sampled, true);
    let mut unsampled = inputs(&plan, &far);
    unsampled.random_sample = Some(0.5);
    assert_meta("uniform-random", unsampled, false);
}

#[test]
fn threshold_store_seeds_preserves_and_persists_overrides() {
    let temp = tempdir().unwrap();
    let db = Db::open(&temp.path().join("calibration.sqlite")).unwrap();

    let mut store = ThresholdStore::load(&db).unwrap();
    assert_eq!(store.get("long-serial-chain"), 4.0);
    assert_eq!(
        store.source("long-serial-chain"),
        Some(ThresholdSource::Default)
    );

    store.set("long-serial-chain", 6.0, "test").unwrap();
    assert_eq!(store.get("long-serial-chain"), 6.0);

    let store = ThresholdStore::load(&db).unwrap();
    assert_eq!(store.get("long-serial-chain"), 6.0);
    assert!(matches!(
        store.source("long-serial-chain"),
        Some(ThresholdSource::Override { updated_by, .. }) if updated_by.as_deref() == Some("test")
    ));
}

struct HeuristicCase {
    name: &'static str,
    firing: PlanInputs,
    missing: PlanInputs,
}

fn heuristic_cases() -> Vec<HeuristicCase> {
    vec![
        case(
            "shared-file-contention",
            plan_with(vec![
                phase(1, "01-a", "medium", &["src/lib.rs"]),
                phase(2, "02-b", "medium", &["src/lib.rs"]),
            ]),
            plan_with(vec![
                phase(1, "01-a", "medium", &["src/lib.rs"]),
                phase(2, "02-b", "medium", &["src/main.rs"]),
            ]),
        ),
        case(
            "external-repo-phases",
            with_external(plan_with(vec![phase(1, "01-a", "medium", &["src/lib.rs"])])),
            plan_with(vec![phase(1, "01-a", "medium", &["src/lib.rs"])]),
        ),
        case(
            "convergence-point",
            plan_with(vec![
                phase(1, "01-a", "medium", &[]),
                phase(2, "02-b", "medium", &[]),
                phase(3, "03-c", "medium", &[]),
                phase_with_deps(4, "04-d", "medium", &[], &[1, 2, 3]),
            ]),
            plan_with(vec![
                phase(1, "01-a", "medium", &[]),
                phase_with_deps(2, "02-b", "medium", &[], &[1]),
            ]),
        ),
        case(
            "ownership-boundary-spread",
            with_repo_spread(plan_with(vec![phase(1, "01-a", "medium", &[])]), 2),
            with_repo_spread(plan_with(vec![phase(1, "01-a", "medium", &[])]), 1),
        ),
        case(
            "risk-concentration",
            with_routing(plan_with(vec![
                phase(1, "01-a", "max", &[]),
                phase(2, "02-b", "max", &[]),
            ])),
            with_routing(plan_with(vec![phase(1, "01-a", "max", &[])])),
        ),
        case(
            "risk-late-in-plan",
            with_waves(
                with_routing(plan_with(vec![
                    phase(1, "01-a", "medium", &[]),
                    phase(2, "02-b", "medium", &[]),
                    phase(3, "03-c", "max", &[]),
                ])),
                vec![vec![1], vec![2], vec![3]],
            ),
            with_waves(
                with_routing(plan_with(vec![
                    phase(1, "01-a", "max", &[]),
                    phase(2, "02-b", "medium", &[]),
                    phase(3, "03-c", "medium", &[]),
                ])),
                vec![vec![1], vec![2], vec![3]],
            ),
        ),
        case(
            "infrastructure-spof",
            plan_with(vec![
                phase(1, "01-ci", "medium", &["flake.nix"]),
                phase_with_deps(2, "02-app", "medium", &["src/lib.rs"], &[1]),
            ]),
            plan_with(vec![
                phase(1, "01-ci", "medium", &["flake.nix"]),
                phase(2, "02-app", "medium", &["src/lib.rs"]),
            ]),
        ),
        case(
            "revendor-phase",
            plan_with(vec![phase(1, "01-bump-deps", "medium", &["Cargo.lock"])]),
            plan_with(vec![phase(1, "01-feature", "medium", &["src/lib.rs"])]),
        ),
        case(
            "long-serial-chain",
            with_depth(plan_with(vec![phase(1, "01-a", "medium", &[])]), 4),
            with_depth(plan_with(vec![phase(1, "01-a", "medium", &[])]), 3),
        ),
        case(
            "mid-plan-rerouting",
            with_phase_count(plan_with(vec![]), 10),
            with_phase_count(plan_with(vec![]), 9),
        ),
        case(
            "trivial-phase-swamp",
            with_dist(plan_with(vec![]), &[("low", 4), ("high", 1)]),
            with_dist(plan_with(vec![]), &[("low", 3), ("high", 1)]),
        ),
        case(
            "no-integrated-verification",
            plan_with(vec![phase(1, "01-feature", "medium", &["src/lib.rs"])]),
            plan_with(vec![phase(
                1,
                "01-integration-verify",
                "medium",
                &["tests/e2e.rs"],
            )]),
        ),
        case(
            "routing-tier-inversion",
            with_dist(
                plan_with(vec![phase(1, "01-leaf", "max", &[])]),
                &[("max", 1)],
            ),
            with_dist(
                plan_with(vec![phase(1, "01-leaf", "medium", &[])]),
                &[("max", 1)],
            ),
        ),
        case(
            "mechanical-streak",
            plan_with(vec![
                phase(1, "01-a", "low", &[]),
                phase(2, "02-b", "low", &[]),
                phase(3, "03-c", "low", &[]),
            ]),
            plan_with(vec![
                phase(1, "01-a", "low", &[]),
                phase(2, "02-b", "medium", &[]),
                phase(3, "03-c", "low", &[]),
            ]),
        ),
        case(
            "hidden-prerequisite",
            plan_with(vec![phase_with_deps(1, "01-a", "medium", &[], &[99])]),
            plan_with(vec![phase(1, "01-a", "medium", &[])]),
        ),
    ]
}

fn assert_meta(name: &str, inputs: MetaHeuristicInputs<'_>, expected: bool) {
    let heuristic = META_HEURISTICS
        .iter()
        .find(|heuristic| heuristic.name() == name)
        .unwrap();
    assert_eq!(heuristic.fires(&inputs), expected, "{name}");
}

fn inputs<'a>(
    plan: &'a PlanInputs,
    triggers: &'a [skillnet::calibration::catalog::TriggerOutcome],
) -> MetaHeuristicInputs<'a> {
    MetaHeuristicInputs::new(plan, triggers)
}

fn outcome(
    input_value: f64,
    threshold: f64,
    fired: bool,
) -> skillnet::calibration::catalog::TriggerOutcome {
    skillnet::calibration::catalog::TriggerOutcome {
        input_value,
        threshold,
        fired,
        section_added: None,
    }
}

fn case(name: &'static str, firing: PlanInputs, missing: PlanInputs) -> HeuristicCase {
    HeuristicCase {
        name,
        firing,
        missing,
    }
}

fn plan_with(phases: Vec<PhaseInputs>) -> PlanInputs {
    let mut routing_dist = BTreeMap::new();
    for phase in &phases {
        *routing_dist.entry(phase.routing_tier.clone()).or_default() += 1;
    }
    PlanInputs {
        path: "/tmp/plan".into(),
        name: "synthetic".to_string(),
        flavor: "codex".to_string(),
        worktype: Some("refactor".to_string()),
        phase_count: phases.len() as u32,
        wave_count: 1,
        max_chain_depth: 1,
        repo_spread: 1,
        routing_dist,
        waves: vec![phases.iter().map(|phase| phase.ordinal).collect()],
        phases,
    }
}

fn phase(ordinal: u32, slug: &str, routing_tier: &str, files: &[&str]) -> PhaseInputs {
    phase_with_deps(ordinal, slug, routing_tier, files, &[])
}

fn phase_with_deps(
    ordinal: u32,
    slug: &str,
    routing_tier: &str,
    files: &[&str],
    depends_on: &[u32],
) -> PhaseInputs {
    PhaseInputs {
        ordinal,
        slug: slug.to_string(),
        routing_tier: routing_tier.to_string(),
        files: files.iter().map(|file| (*file).to_string()).collect(),
        working_tree: None,
        depends_on: depends_on.to_vec(),
    }
}

fn with_external(mut plan: PlanInputs) -> PlanInputs {
    plan.phases[0].working_tree = Some("/tmp/external".to_string());
    plan
}

fn with_repo_spread(mut plan: PlanInputs, repo_spread: u32) -> PlanInputs {
    plan.repo_spread = repo_spread;
    plan
}

fn with_depth(mut plan: PlanInputs, depth: u32) -> PlanInputs {
    plan.max_chain_depth = depth;
    plan
}

fn with_phase_count(mut plan: PlanInputs, phase_count: u32) -> PlanInputs {
    plan.phase_count = phase_count;
    plan
}

fn with_waves(mut plan: PlanInputs, waves: Vec<Vec<u32>>) -> PlanInputs {
    plan.wave_count = waves.len() as u32;
    plan.waves = waves;
    plan
}

fn with_dist(mut plan: PlanInputs, entries: &[(&str, u32)]) -> PlanInputs {
    plan.routing_dist = entries
        .iter()
        .map(|(tier, count)| ((*tier).to_string(), *count))
        .collect();
    plan
}

fn with_routing(mut plan: PlanInputs) -> PlanInputs {
    plan.routing_dist.clear();
    for phase in &plan.phases {
        *plan
            .routing_dist
            .entry(phase.routing_tier.clone())
            .or_default() += 1;
    }
    plan
}
