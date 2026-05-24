use skillnet::calibration::catalog::HEURISTICS;

#[test]
fn default_threshold_snapshot() {
    let defaults = HEURISTICS
        .iter()
        .map(|heuristic| (heuristic.name(), heuristic.default_threshold()))
        .collect::<Vec<_>>();

    assert_eq!(
        defaults,
        vec![
            ("shared-file-contention", 2.0),
            ("external-repo-phases", 1.0),
            ("convergence-point", 3.0),
            ("ownership-boundary-spread", 2.0),
            ("risk-concentration", 2.0),
            ("risk-late-in-plan", 1.0),
            ("infrastructure-spof", 1.0),
            ("revendor-phase", 1.0),
            ("long-serial-chain", 4.0),
            ("mid-plan-rerouting", 10.0),
            ("trivial-phase-swamp", 4.0),
            ("no-integrated-verification", 1.0),
            ("routing-tier-inversion", 1.0),
            ("mechanical-streak", 3.0),
            ("hidden-prerequisite", 1.0),
        ]
    );
}
