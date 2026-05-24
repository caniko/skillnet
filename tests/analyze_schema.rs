use std::{fs, path::PathBuf};

use assert_cmd::Command;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
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
        command.env("SKILLNET_CONFIG", self.repo.path().join("skillnet.toml"));
        command.env_remove("SKILLNET_DATABASE_URL");
        command.env_remove("SKILLNET_DB_URL");
        command.env_remove("DATABASE_URL");
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
fn analyze_json_schema_shape_is_stable() {
    let fixture = Fixture::new();
    let conn = fixture.conn();

    seed_row(&conn, "schema-fire-a", true, None);
    seed_row(&conn, "schema-fire-b", true, None);
    seed_row(
        &conn,
        "schema-miss-a",
        false,
        Some("missed-signal: long-serial-chain: recovery section would have helped"),
    );

    let output = fixture
        .command()
        .args([
            "calibration",
            "analyze",
            "--trigger",
            "long-serial-chain",
            "--min-n",
            "1",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let mut actual: Value = serde_json::from_slice(&output).unwrap();
    assert!(actual["analyzed_at"].as_str().is_some());
    actual["analyzed_at"] = json!("TIMESTAMP");

    assert_eq!(
        actual,
        json!({
            "schema_version": 1,
            "analyzed_at": "TIMESTAMP",
            "min_n": 1,
            "dataset_size": 3,
            "filter_tags": [],
            "filter_tags_applied": [],
            "triggers": [
                {
                    "trigger": "long-serial-chain",
                    "fires": 2,
                    "misses": 1,
                    "fire_rate": 2.0 / 3.0,
                    "signal_rate": 1.0 / 3.0,
                    "verdict": "hold",
                    "default_threshold": 4.0,
                    "current_threshold": 4.0,
                    "threshold_source": {
                        "type": "default"
                    },
                    "true_positives": 2,
                    "false_positives": 0,
                    "false_negatives": 1,
                    "true_negatives": 0,
                    "supporting_plan_ids": []
                }
            ],
            "proposals": [],
            "skew_warnings": []
        })
    );
}

fn seed_row(conn: &Connection, plan_id: &str, fired: bool, surprises: Option<&str>) {
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
        ) VALUES (?1, ?2, ?3, ?4, 'codex', 'refactor', 1, 1, 4, 1, '{}', ?5, '[]')",
        params![
            plan_id,
            created_at(plan_id),
            plan_id,
            format!("/tmp/{plan_id}"),
            format!("shape-{plan_id}"),
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
        ) VALUES (?1, 'long-serial-chain', 4.0, 4.0, ?2, 'Serial-chain recovery')",
        params![plan_id, fired],
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
        ) VALUES (?1, 2, NULL, 'shipped', '{}', NULL, ?2)",
        params![plan_id, surprises],
    )
    .unwrap();
}

fn created_at(plan_id: &str) -> i64 {
    plan_id.bytes().map(i64::from).sum()
}
