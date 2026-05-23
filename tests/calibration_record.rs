use std::{fs, path::Path};

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

struct Fixture {
    repo: TempDir,
    plan: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let repo = tempdir().unwrap();
        let schema_dir = repo.path().join("data/multi-phase-plan/schema");
        fs::create_dir_all(&schema_dir).unwrap();
        fs::copy(
            "data/multi-phase-plan/schema/001-initial.sql",
            schema_dir.join("001-initial.sql"),
        )
        .unwrap();

        Self {
            repo,
            plan: tempdir().unwrap(),
        }
    }

    fn plan_dir(&self) -> &Path {
        self.plan.path()
    }

    fn db_path(&self) -> std::path::PathBuf {
        self.repo
            .path()
            .join("data/multi-phase-plan/calibration.sqlite")
    }

    fn write_sidecar(&self, body: &Value) {
        fs::write(
            self.plan_dir().join(".calibration.json"),
            serde_json::to_string_pretty(body).unwrap(),
        )
        .unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.env("AI_SKILLS_REPO", self.repo.path());
        command
    }

    fn conn(&self) -> Connection {
        Connection::open(self.db_path()).unwrap()
    }
}

#[test]
fn record_writes_rows_and_is_idempotent() {
    let fixture = Fixture::new();
    let mut sidecar = valid_sidecar(false);
    fixture.write_sidecar(&sidecar);

    fixture
        .command()
        .args([
            "calibration",
            "record",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("recorded plan-0001"));

    let conn = fixture.conn();
    assert_eq!(count(&conn, "plans"), 1);
    assert_eq!(count(&conn, "triggers"), 2);
    assert_eq!(count(&conn, "phases"), 2);
    assert_tag(&conn, "flavor", "codex");
    assert_tag(&conn, "worktype", "refactor");
    assert_tag(&conn, "scope", "cross-org");
    assert_tag(&conn, "risk", "mixed");
    assert_tag(&conn, "signal", "phase-count");
    assert_tag(&conn, "signal", "repo-spread");
    assert_tag(&conn, "owner", "calibration");
    drop(conn);

    fixture
        .command()
        .args([
            "calibration",
            "record",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .success();
    let conn = fixture.conn();
    assert_eq!(count(&conn, "plans"), 1);
    assert_eq!(count(&conn, "triggers"), 2);
    assert_eq!(count(&conn, "phases"), 2);
    drop(conn);

    sidecar["triggers"].as_array_mut().unwrap().pop();
    fixture.write_sidecar(&sidecar);
    fixture
        .command()
        .args([
            "calibration",
            "record",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 triggers"));

    let conn = fixture.conn();
    assert_eq!(count(&conn, "plans"), 1);
    assert_eq!(count(&conn, "triggers"), 1);
    assert_eq!(count(&conn, "phases"), 2);
}

#[test]
fn verify_requires_section_then_records_outcome_tag() {
    let fixture = Fixture::new();
    let sidecar = valid_sidecar(false);
    fixture.write_sidecar(&sidecar);

    fixture
        .command()
        .args([
            "calibration",
            "record",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .success();

    fixture
        .command()
        .args([
            "calibration",
            "verify",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no verify section in sidecar"));

    let sidecar = valid_sidecar(true);
    fixture.write_sidecar(&sidecar);
    fixture
        .command()
        .args([
            "calibration",
            "verify",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "verified plan-0001: shipped (1/2 phases passed)",
        ));

    let conn = fixture.conn();
    assert_eq!(count(&conn, "verifications"), 1);
    assert_tag(&conn, "outcome", "shipped");

    let mut sidecar = valid_sidecar(true);
    sidecar["verify"]["outcome"] = json!("partial");
    fixture.write_sidecar(&sidecar);
    fixture
        .command()
        .args([
            "calibration",
            "verify",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .success();

    let conn = fixture.conn();
    assert_eq!(count(&conn, "verifications"), 1);
    assert_eq!(tag_count(&conn, "outcome"), 1);
    assert_tag(&conn, "outcome", "partial");
}

#[test]
fn malformed_json_reports_parse_location() {
    let fixture = Fixture::new();
    fs::write(
        fixture.plan_dir().join(".calibration.json"),
        "{\n  \"schema_version\": 1,\n",
    )
    .unwrap();

    fixture
        .command()
        .args([
            "calibration",
            "record",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("malformed calibration sidecar"))
        .stderr(predicate::str::contains("line"));
}

#[test]
fn wrong_schema_version_is_rejected() {
    let fixture = Fixture::new();
    let mut sidecar = valid_sidecar(false);
    sidecar["schema_version"] = json!(2);
    fixture.write_sidecar(&sidecar);

    fixture
        .command()
        .args([
            "calibration",
            "record",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unsupported calibration sidecar schema_version 2; expected 1",
        ));
}

fn valid_sidecar(with_verify: bool) -> Value {
    let mut sidecar = json!({
        "schema_version": 1,
        "plan": {
            "id": "plan-0001",
            "name": "Synthetic plan",
            "flavor": "codex",
            "worktype": "refactor",
            "created_at": 1717171717,
            "phase_count": 2,
            "wave_count": 1,
            "max_chain_depth": 1,
            "repo_spread": 3,
            "routing_dist": {
                "medium": 1,
                "high": 1
            },
            "shape_hash": "shape-abc"
        },
        "triggers": [
            {
                "name": "phase-count",
                "input_value": 9.0,
                "threshold": 8.0,
                "fired": true,
                "section_added": "parallelism"
            },
            {
                "name": "repo-spread",
                "input_value": 3.0,
                "threshold": 2.0,
                "fired": true,
                "section_added": null
            }
        ],
        "phases": [
            {
                "ordinal": 1,
                "slug": "schema",
                "routing_tier": "medium",
                "files": ["src/calibration/db.rs"]
            },
            {
                "ordinal": 2,
                "slug": "cli",
                "routing_tier": "high",
                "files": ["src/cli/args.rs"]
            }
        ],
        "meta_heuristics_fired": ["phase-count", "repo-spread"],
        "tags": {
            "owner": "calibration"
        }
    });

    if with_verify {
        sidecar["verify"] = json!({
            "verified_at": 1717171818,
            "elapsed_seconds": 42,
            "outcome": "shipped",
            "phase_outcomes": {
                "schema": "passed",
                "cli": "failed"
            },
            "emergency_changes": {
                "files": ["src/cli/mod.rs"]
            },
            "surprises": "none"
        });
    }

    sidecar
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn assert_tag(conn: &Connection, key: &str, value: &str) {
    let found: bool = conn
        .query_row(
            "SELECT EXISTS (
                SELECT 1 FROM tags
                WHERE plan_id = 'plan-0001' AND key = ?1 AND value = ?2
            )",
            (key, value),
            |row| row.get(0),
        )
        .unwrap();
    assert!(found, "missing tag {key}:{value}");
}

fn tag_count(conn: &Connection, key: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM tags WHERE plan_id = 'plan-0001' AND key = ?1",
        [key],
        |row| row.get(0),
    )
    .unwrap()
}
