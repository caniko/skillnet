use std::{fs, path::PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::{tempdir, TempDir};

struct Fixture {
    repo: TempDir,
    plan: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let repo = tempdir().unwrap();
        let plan = tempdir().unwrap();
        write_plan(plan.path());
        Self { repo, plan }
    }

    fn plan_dir(&self) -> PathBuf {
        self.plan.path().to_path_buf()
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
        command.env("SKILLNET_CONFIG", self.repo.path().join("skillnet.toml"));
        command.env_remove("SKILLNET_DATABASE_URL");
        command.env_remove("SKILLNET_DB_URL");
        command
    }
}

#[test]
fn init_produces_sidecar_and_record_ingests_it() {
    let fixture = Fixture::new();
    fixture
        .command()
        .args(["calibration", "init", fixture.plan_dir().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(".calibration.json"));

    let sidecar_path = fixture.plan_dir().join(".calibration.json");
    let sidecar: Value = serde_json::from_str(&fs::read_to_string(&sidecar_path).unwrap()).unwrap();
    assert_eq!(sidecar["schema_version"], 1);
    assert_eq!(sidecar["plan"]["phase_count"], 3);
    assert_eq!(sidecar["triggers"].as_array().unwrap().len(), 15);

    fixture
        .command()
        .args([
            "calibration",
            "record",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("recorded"));

    let conn = fixture.conn();
    assert_eq!(count(&conn, "plans"), 1);
    assert_eq!(count(&conn, "triggers"), 15);
}

#[test]
fn init_refuses_overwrite_and_force_preserves_state() {
    let fixture = Fixture::new();
    fixture
        .command()
        .args(["calibration", "init", fixture.plan_dir().to_str().unwrap()])
        .assert()
        .success();

    fixture
        .command()
        .args(["calibration", "init", fixture.plan_dir().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("refusing to overwrite"));

    let sidecar_path = fixture.plan_dir().join(".calibration.json");
    let mut sidecar: Value =
        serde_json::from_str(&fs::read_to_string(&sidecar_path).unwrap()).unwrap();
    sidecar["plan"]["id"] = Value::String("stable-id".to_string());
    sidecar["tags"]["owner"] = Value::String("calibration".to_string());
    sidecar["verify"] = serde_json::json!({
        "verified_at": 1,
        "elapsed_seconds": 3,
        "outcome": "partial",
        "phase_outcomes": {"01-foundation": "passed"},
        "emergency_changes": null,
        "surprises": "missed-signal: hidden-prerequisite: fixture"
    });
    fs::write(
        &sidecar_path,
        serde_json::to_string_pretty(&sidecar).unwrap(),
    )
    .unwrap();

    fixture
        .command()
        .args([
            "calibration",
            "init",
            "--force",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .success();

    let forced: Value = serde_json::from_str(&fs::read_to_string(&sidecar_path).unwrap()).unwrap();
    assert_eq!(forced["plan"]["id"], "stable-id");
    assert_eq!(forced["tags"]["owner"], "calibration");
    assert_eq!(forced["verify"]["outcome"], "partial");
}

#[test]
fn eval_json_matches_recorded_triggers() {
    let fixture = Fixture::new();
    let eval_output = fixture
        .command()
        .args(["calibration", "eval", fixture.plan_dir().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let eval_json: Value = serde_json::from_slice(&eval_output).unwrap();
    let eval_rows = eval_json.as_array().unwrap();

    fixture
        .command()
        .args(["calibration", "init", fixture.plan_dir().to_str().unwrap()])
        .assert()
        .success();
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
    let mut rows = conn
        .prepare("SELECT name, input_value, threshold, fired FROM triggers ORDER BY name")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut eval_rows = eval_rows
        .iter()
        .map(|row| {
            (
                row["name"].as_str().unwrap().to_string(),
                row["input_value"].as_f64().unwrap(),
                row["threshold"].as_f64().unwrap(),
                row["fired"].as_bool().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    eval_rows.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(eval_rows, rows);
}

#[test]
fn meta_heuristics_shape_hash_and_heuristics_commands_work() {
    let fixture = Fixture::new();

    let meta_output = fixture
        .command()
        .args([
            "calibration",
            "meta-heuristics",
            fixture.plan_dir().to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let meta_json: Value = serde_json::from_slice(&meta_output).unwrap();
    let fired = meta_json["fired"].as_array().unwrap();
    assert!(contains(fired, "threshold-proximity"));
    assert!(contains(fired, "high-stakes-combo"));

    let first_hash = shape_hash(&fixture);
    let second_hash = shape_hash(&fixture);
    assert_eq!(first_hash, second_hash);
    fs::write(
        fixture.plan_dir().join("03-integrate.md"),
        PHASE_3.replace("tests/e2e.rs", "tests/e2e.rs\n- tests/smoke.rs"),
    )
    .unwrap();
    assert_ne!(first_hash, shape_hash(&fixture));

    let list_output = fixture
        .command()
        .args(["calibration", "heuristics", "list", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list_json: Value = serde_json::from_slice(&list_output).unwrap();
    assert_eq!(list_json.as_array().unwrap().len(), 15);

    let category_output = fixture
        .command()
        .args([
            "calibration",
            "heuristics",
            "list",
            "--format",
            "json",
            "--category",
            "coordination",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let category_json: Value = serde_json::from_slice(&category_output).unwrap();
    assert_eq!(category_json.as_array().unwrap().len(), 4);

    fixture
        .command()
        .args(["calibration", "heuristics", "show", "long-serial-chain"])
        .assert()
        .success()
        .stdout(predicate::str::contains("long-serial-chain"))
        .stdout(predicate::str::contains("Serial-chain recovery"));

    fixture
        .command()
        .args(["calibration", "heuristics", "show", "missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown heuristic missing"));
}

#[test]
fn helper_commands_appear_in_help() {
    fixture_command()
        .args(["calibration", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("eval"))
        .stdout(predicate::str::contains("meta-heuristics"))
        .stdout(predicate::str::contains("shape-hash"))
        .stdout(predicate::str::contains("heuristics"));

    fixture_command()
        .args(["calibration", "heuristics", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("show"));
}

fn fixture_command() -> Command {
    let temp = tempdir().unwrap();
    let mut command = Command::cargo_bin("skillnet").unwrap();
    command.env("SKILLNET_CONFIG", temp.path().join("skillnet.toml"));
    command.env_remove("SKILLNET_DATABASE_URL");
    command.env_remove("SKILLNET_DB_URL");
    command
}

fn shape_hash(fixture: &Fixture) -> String {
    String::from_utf8(
        fixture
            .command()
            .args([
                "calibration",
                "shape-hash",
                fixture.plan_dir().to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap()
    .trim()
    .to_string()
}

fn contains(values: &[Value], needle: &str) -> bool {
    values.iter().any(|value| value.as_str() == Some(needle))
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn write_plan(path: &std::path::Path) {
    fs::write(path.join("README.md"), README).unwrap();
    fs::write(path.join("01-foundation.md"), PHASE_1).unwrap();
    fs::write(path.join("02-external.md"), PHASE_2).unwrap();
    fs::write(path.join("03-integrate.md"), PHASE_3).unwrap();
}

const README: &str = r#"# Synthetic calibration plan

> **Recommended Codex model: GPT 5.5 medium**

| Phase | File | Depends on | Touches | Can parallel with |
|---|---|---|---|---|
| 01 | [01-foundation.md](./01-foundation.md) | — | `src/lib.rs` | 02 |
| 02 | [02-external.md](./02-external.md) | — | `src/lib.rs` | 01 |
| 03 | [03-integrate.md](./03-integrate.md) | 01, 02 | `tests/e2e.rs` | — |
"#;

const PHASE_1: &str = r#"# Phase 01 — foundation

> **Recommended Codex model: GPT 5.5 medium**

## Working tree

same as primary

## Files likely touched

- `src/lib.rs`
"#;

const PHASE_2: &str = r#"# Phase 02 — external

> **Recommended Codex model: GPT 5.5 high**

## Working tree

/tmp/external-repo

## Files likely touched

- `src/lib.rs`
"#;

const PHASE_3: &str = r#"# Phase 03 — integrate

> **Recommended Codex model: GPT 5.5 max**

## Working tree

same as primary

## Files likely touched

- `tests/e2e.rs`
"#;
