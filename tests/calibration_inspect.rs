use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

struct Fixture {
    repo: TempDir,
    plans: Vec<TempDir>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            repo: tempdir().unwrap(),
            plans: Vec::new(),
        }
    }

    fn add_plan(&mut self, body: &Value) -> PathBuf {
        let plan = tempdir().unwrap();
        fs::write(
            plan.path().join(".calibration.json"),
            serde_json::to_string_pretty(body).unwrap(),
        )
        .unwrap();
        let path = plan.path().to_path_buf();
        self.plans.push(plan);
        path
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.env("skillnet_DATA_DIR", self.repo.path().join("data"));
        command
    }

    fn record(&self, plan_dir: &Path) {
        self.command()
            .args(["calibration", "record", plan_dir.to_str().unwrap()])
            .assert()
            .success();
    }
}

#[test]
fn tag_show_and_untag_manage_user_tags() {
    let mut fixture = Fixture::new();
    let plan_dir = fixture.add_plan(&sidecar("plan-codex", "codex", "refactor", true, 3));
    fixture.record(&plan_dir);

    fixture
        .command()
        .args(["calibration", "tag", "plan-codex", "project=auth"])
        .assert()
        .success();

    let show = fixture
        .command()
        .args(["calibration", "show", "plan-codex"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&show).unwrap();
    assert!(value["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tag| tag["key"] == "project" && tag["value"] == "auth"));

    fixture
        .command()
        .args(["calibration", "untag", "plan-codex", "project=auth"])
        .assert()
        .success();
    let show = fixture
        .command()
        .args(["calibration", "show", "plan-codex"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Value = serde_json::from_slice(&show).unwrap();
    assert!(!value["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tag| tag["key"] == "project" && tag["value"] == "auth"));
}

#[test]
fn tag_key_validation_and_auto_tag_protection_are_enforced() {
    let mut fixture = Fixture::new();
    let plan_dir = fixture.add_plan(&sidecar("plan-codex", "codex", "refactor", true, 3));
    fixture.record(&plan_dir);

    fixture
        .command()
        .args(["calibration", "tag", "plan-codex", "BadKey=value"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("tag key must match"));

    fixture
        .command()
        .args(["calibration", "untag", "plan-codex", "flavor=codex"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot remove derived auto-tag key",
        ));
}

#[test]
fn query_filters_by_tags_and_triggers() {
    let mut fixture = Fixture::new();
    let codex = fixture.add_plan(&sidecar("plan-codex", "codex", "refactor", true, 3));
    let claude = fixture.add_plan(&sidecar("plan-claude", "claude", "docs", false, 1));
    let low = fixture.add_plan(&sidecar("plan-low", "codex", "docs", false, 1));
    fixture.record(&codex);
    fixture.record(&claude);
    fixture.record(&low);

    let out = fixture
        .command()
        .args(["calibration", "query", "--tag", "flavor=codex"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("plan-codex"));
    assert!(text.contains("plan-low"));
    assert!(!text.contains("plan-claude"));

    let out = fixture
        .command()
        .args([
            "calibration",
            "query",
            "--tag",
            "flavor=codex",
            "--tag",
            "risk=mixed",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("plan-codex"));
    assert!(!text.contains("plan-low"));

    let out = fixture
        .command()
        .args([
            "calibration",
            "query",
            "--trigger",
            "chain-depth",
            "--fired",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("plan-codex"));
    assert!(!text.contains("plan-claude"));
}

#[test]
fn query_json_export_migrate_and_vacuum_work() {
    let mut fixture = Fixture::new();
    let first = fixture.add_plan(&sidecar("plan-codex", "codex", "refactor", true, 3));
    let second = fixture.add_plan(&sidecar("plan-claude", "claude", "docs", false, 1));
    fixture.record(&first);
    fixture.record(&second);

    let out = fixture
        .command()
        .args(["calibration", "query", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rows: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);

    let out = fixture
        .command()
        .args(["calibration", "export", "--format", "jsonl"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let lines = String::from_utf8(out).unwrap();
    let parsed = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(parsed.len(), 2);
    assert!(parsed.iter().all(|plan| plan["triggers"].is_array()));

    fixture
        .command()
        .args(["calibration", "migrate"])
        .assert()
        .success();
    fixture
        .command()
        .args(["calibration", "vacuum"])
        .assert()
        .success();
}

#[test]
fn show_unknown_id_fails_clearly() {
    let fixture = Fixture::new();
    fixture
        .command()
        .args(["calibration", "show", "missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unknown calibration plan id missing",
        ));
}

#[cfg(not(feature = "postgres"))]
#[test]
fn postgres_target_without_feature_fails_clearly() {
    let fixture = Fixture::new();
    fixture
        .command()
        .args([
            "--database-url",
            "postgres://user@localhost/skillnet",
            "calibration",
            "migrate",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "built without the `postgres` feature",
        ));
}

fn sidecar(id: &str, flavor: &str, worktype: &str, chain_fired: bool, repo_spread: u32) -> Value {
    let routing_dist = if repo_spread > 1 {
        json!({ "medium": 1, "high": 1 })
    } else {
        json!({ "low": 2 })
    };
    json!({
        "schema_version": 1,
        "plan": {
            "id": id,
            "name": format!("Synthetic {id}"),
            "flavor": flavor,
            "worktype": worktype,
            "created_at": 1717171717,
            "phase_count": 2,
            "wave_count": 1,
            "max_chain_depth": if chain_fired { 3 } else { 1 },
            "repo_spread": repo_spread,
            "routing_dist": routing_dist,
            "shape_hash": format!("shape-{id}")
        },
        "triggers": [
            {
                "name": "chain-depth",
                "input_value": if chain_fired { 3.0 } else { 1.0 },
                "threshold": 2.0,
                "fired": chain_fired,
                "section_added": "parallelism"
            },
            {
                "name": "repo-spread",
                "input_value": repo_spread,
                "threshold": 2.0,
                "fired": repo_spread > 1,
                "section_added": null
            }
        ],
        "phases": [
            {
                "ordinal": 1,
                "slug": "one",
                "routing_tier": "medium",
                "files": ["src/lib.rs"]
            },
            {
                "ordinal": 2,
                "slug": "two",
                "routing_tier": "high",
                "files": ["src/cli/args.rs"]
            }
        ],
        "meta_heuristics_fired": if chain_fired { json!(["chain-depth"]) } else { json!([]) },
        "tags": {
            "owner": "inspect"
        }
    })
}
