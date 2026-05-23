use std::{fs, path::PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::{params, Connection};
use serde_json::Value;
use tempfile::{tempdir, TempDir};

struct Fixture {
    repo: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let repo = tempdir().unwrap();
        let schema_dir = repo.path().join("data/multi-phase-plan/schema");
        fs::create_dir_all(&schema_dir).unwrap();
        fs::copy(schema_path(), schema_dir.join("001-initial.sql")).unwrap();

        let fixture = Self { repo };
        fixture.init_db();
        fixture
    }

    fn db_path(&self) -> PathBuf {
        self.repo
            .path()
            .join("data/multi-phase-plan/calibration.sqlite")
    }

    fn conn(&self) -> Connection {
        Connection::open(self.db_path()).unwrap()
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.env("skillnet_DATA_DIR", self.repo.path().join("data"));
        command
    }

    fn init_db(&self) {
        let conn = self.conn();
        let schema = fs::read_to_string(schema_path()).unwrap();
        conn.execute_batch(&schema).unwrap();
        conn.execute(
            "INSERT INTO schema_versions (version, applied_at) VALUES (1, 1)",
            [],
        )
        .unwrap();
    }
}

fn schema_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/multi-phase-plan/schema/001-initial.sql")
}

#[test]
fn analyze_reports_lower_raise_and_min_n_guard() {
    let fixture = Fixture::new();
    let conn = fixture.conn();

    for i in 0..15 {
        seed_row(
            &conn,
            SeedRow {
                plan_id: format!("t1-fire-{i}"),
                trigger: "T1",
                fired: true,
                threshold: 10.0,
                flavor: "codex",
                worktype: "refactor",
                outcome: "shipped",
                surprises: None,
                emergency_changes: None,
            },
        );
    }
    for i in 0..2 {
        seed_row(
            &conn,
            SeedRow {
                plan_id: format!("t1-miss-{i}"),
                trigger: "T1",
                fired: false,
                threshold: 10.0,
                flavor: "codex",
                worktype: "refactor",
                outcome: "shipped",
                surprises: Some("missed-signal: T1: missing section"),
                emergency_changes: None,
            },
        );
    }

    for i in 0..20 {
        seed_row(
            &conn,
            SeedRow {
                plan_id: format!("t2-fire-{i}"),
                trigger: "T2",
                fired: true,
                threshold: 4.0,
                flavor: "codex",
                worktype: "refactor",
                outcome: "shipped",
                surprises: Some("dead-weight: T2: noisy section"),
                emergency_changes: None,
            },
        );
    }

    for i in 0..5 {
        seed_row(
            &conn,
            SeedRow {
                plan_id: format!("t3-fire-{i}"),
                trigger: "T3",
                fired: true,
                threshold: 8.0,
                flavor: "codex",
                worktype: "refactor",
                outcome: "shipped",
                surprises: None,
                emergency_changes: None,
            },
        );
    }

    fixture
        .command()
        .args(["calibration", "analyze"])
        .assert()
        .success()
        .stdout(predicate::str::contains("T1"))
        .stdout(predicate::str::contains("lower threshold (10->9)"))
        .stdout(predicate::str::contains("T2"))
        .stdout(predicate::str::contains("raise threshold (4->5)"))
        .stdout(predicate::str::contains("T3"))
        .stdout(predicate::str::contains("n=5 < min-n=10"))
        .stdout(predicate::str::contains("PROPOSALS (2):"));
}

#[test]
fn analyze_json_and_filter_tags_restrict_dataset() {
    let fixture = Fixture::new();
    let conn = fixture.conn();

    for i in 0..30 {
        seed_row(
            &conn,
            SeedRow {
                plan_id: format!("codex-{i}"),
                trigger: "T4",
                fired: true,
                threshold: 3.0,
                flavor: "codex",
                worktype: "refactor",
                outcome: "shipped",
                surprises: None,
                emergency_changes: None,
            },
        );
        seed_row(
            &conn,
            SeedRow {
                plan_id: format!("claude-{i}"),
                trigger: "T4",
                fired: true,
                threshold: 3.0,
                flavor: "claude",
                worktype: "refactor",
                outcome: "shipped",
                surprises: Some("dead-weight: T4: noisy"),
                emergency_changes: None,
            },
        );
    }

    let output = fixture
        .command()
        .args([
            "calibration",
            "analyze",
            "--filter-tag",
            "flavor=codex",
            "--trigger",
            "T4",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["triggers"][0]["fires"], 30);
    assert_eq!(json["triggers"][0]["false_positives"], 0);
    assert_eq!(json["skew_warnings"].as_array().unwrap().len(), 0);
}

#[test]
fn analyze_skew_warning_runs_without_filter_tags() {
    let fixture = Fixture::new();
    let conn = fixture.conn();

    for i in 0..30 {
        seed_row(
            &conn,
            SeedRow {
                plan_id: format!("codex-{i}"),
                trigger: "T4",
                fired: true,
                threshold: 3.0,
                flavor: "codex",
                worktype: "refactor",
                outcome: "shipped",
                surprises: None,
                emergency_changes: None,
            },
        );
        seed_row(
            &conn,
            SeedRow {
                plan_id: format!("claude-{i}"),
                trigger: "T4",
                fired: true,
                threshold: 3.0,
                flavor: "claude",
                worktype: "refactor",
                outcome: "shipped",
                surprises: Some("dead-weight: T4: noisy"),
                emergency_changes: None,
            },
        );
    }

    fixture
        .command()
        .args(["calibration", "analyze", "--trigger", "T4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SKEW WARNINGS"))
        .stdout(predicate::str::contains(
            "trigger T4 shows skew across flavor",
        ));
}

#[test]
fn proposal_decision_and_changelog_flow() {
    let fixture = Fixture::new();
    let conn = fixture.conn();
    seed_row(
        &conn,
        SeedRow {
            plan_id: "support-1".to_string(),
            trigger: "T5",
            fired: true,
            threshold: 6.0,
            flavor: "codex",
            worktype: "refactor",
            outcome: "shipped",
            surprises: None,
            emergency_changes: None,
        },
    );
    drop(conn);

    fixture
        .command()
        .args([
            "calibration",
            "propose",
            "--trigger",
            "T5",
            "--new-threshold",
            "7",
            "--rationale",
            "fires too broadly",
            "--supporting-plan-ids",
            "support-1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("proposal 1 pending"));

    let conn = fixture.conn();
    let decision: String = conn
        .query_row(
            "SELECT decision FROM calibration_proposals WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(decision, "pending");
    drop(conn);

    fixture
        .command()
        .args(["calibration", "proposals", "--pending"])
        .assert()
        .success()
        .stdout(predicate::str::contains("T5"))
        .stdout(predicate::str::contains("pending"));

    fixture
        .command()
        .args([
            "calibration",
            "decide",
            "1",
            "accept",
            "--rationale",
            "accepted after review",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("proposal 1 accepted"));

    fixture
        .command()
        .args(["calibration", "proposals", "--accepted"])
        .assert()
        .success()
        .stdout(predicate::str::contains("T5"))
        .stdout(predicate::str::contains("accepted"));

    fixture
        .command()
        .args([
            "calibration",
            "decide",
            "1",
            "reject",
            "--rationale",
            "changed mind",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("proposal 1 is already accepted"));

    fixture
        .command()
        .args(["calibration", "export-changelog", "--since", "1970-01-01"])
        .assert()
        .success()
        .stdout(predicate::str::contains("### "))
        .stdout(predicate::str::contains("T5: 6 → 7"))
        .stdout(predicate::str::contains(
            "- **Rationale**: accepted after review",
        ))
        .stdout(predicate::str::contains(
            "- **Supporting plans**: 1, ids: support-1",
        ));

    fixture
        .command()
        .args(["calibration", "export-changelog", "--since", "2999-01-01"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no accepted proposals"));
}

struct SeedRow<'a> {
    plan_id: String,
    trigger: &'a str,
    fired: bool,
    threshold: f64,
    flavor: &'a str,
    worktype: &'a str,
    outcome: &'a str,
    surprises: Option<&'a str>,
    emergency_changes: Option<&'a str>,
}

fn seed_row(conn: &Connection, row: SeedRow<'_>) {
    conn.execute(
        "INSERT INTO plans (
            id,
            created_at,
            name,
            path,
            flavor,
            worktype,
            phase_count,
            wave_count,
            max_chain_depth,
            repo_spread,
            routing_dist,
            shape_hash,
            capture_reasons
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 1, 1, 1, '{}', ?7, '[]')",
        params![
            &row.plan_id,
            created_at(&row.plan_id),
            &row.plan_id,
            format!("/tmp/{}", &row.plan_id),
            row.flavor,
            row.worktype,
            format!("shape-{}", &row.plan_id),
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO triggers (
            plan_id,
            name,
            input_value,
            threshold,
            fired,
            section_added
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            &row.plan_id,
            row.trigger,
            row.threshold,
            row.threshold,
            row.fired,
            row.trigger
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO verifications (
            plan_id,
            verified_at,
            elapsed_seconds,
            outcome,
            phase_outcomes,
            emergency_changes,
            surprises
        ) VALUES (?1, 2, NULL, ?2, '{}', ?3, ?4)",
        params![
            &row.plan_id,
            row.outcome,
            row.emergency_changes,
            row.surprises
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tags (plan_id, key, value) VALUES (?1, 'flavor', ?2)",
        params![&row.plan_id, row.flavor],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tags (plan_id, key, value) VALUES (?1, 'worktype', ?2)",
        params![&row.plan_id, row.worktype],
    )
    .unwrap();
}

fn created_at(plan_id: &str) -> i64 {
    plan_id.bytes().map(i64::from).sum()
}
