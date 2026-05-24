use std::{fs, path::PathBuf};

use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::{tempdir, TempDir};

const POST_TOOL_USE_SKILL: &str = r#"{
  "tool_name": "Skill",
  "tool_input": {
    "skill": "simplify"
  },
  "tool_response": {
    "is_error": false
  }
}"#;

const SETTINGS_PRISTINE: &str = include_str!("fixtures/hook/settings-pristine.json");

struct Fixture {
    temp: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            temp: tempdir().unwrap(),
        }
    }

    fn settings(&self) -> PathBuf {
        self.temp.path().join("settings.json")
    }

    fn data_dir(&self) -> PathBuf {
        self.temp.path().join("data")
    }

    fn db_path(&self) -> PathBuf {
        self.data_dir().join("multi-phase-plan/calibration.sqlite")
    }

    fn config_path(&self) -> PathBuf {
        self.temp.path().join("skillnet.toml")
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.env("SKILLNET_CONFIG", self.config_path());
        command.env("skillnet_DATA_DIR", self.data_dir());
        command.env_remove("SKILLNET_DATABASE_URL");
        command.env_remove("SKILLNET_DB_URL");
        command.env_remove("DATABASE_URL");
        command.env_remove("CLAUDE_HOOK_EVENT");
        command
    }
}

#[test]
fn hook_install_ingest_and_uninstall_round_trip() {
    let fixture = Fixture::new();
    fs::write(fixture.settings(), SETTINGS_PRISTINE).unwrap();

    fixture
        .command()
        .args([
            "hook",
            "install",
            "--settings",
            fixture.settings().to_str().unwrap(),
        ])
        .assert()
        .success();

    let installed = read_json(&fixture.settings());
    assert_eq!(managed_count(&installed, "PostToolUse"), 1);
    assert_eq!(installed["permissions"]["allow"][0], "Bash(git status:*)");

    fixture
        .command()
        .env("CLAUDE_SESSION_ID", "e2e-session")
        .args(["hook", "ingest", "--strict", "--event", "PostToolUse"])
        .write_stdin(POST_TOOL_USE_SKILL)
        .assert()
        .success();

    let conn = Connection::open(fixture.db_path()).unwrap();
    let (count, skill_name, hook_event): (i64, String, String) = conn
        .query_row(
            "SELECT COUNT(*), max(skill_name), max(hook_event) FROM skill_invocations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(skill_name, "simplify");
    assert_eq!(hook_event, "PostToolUse");

    fixture
        .command()
        .args([
            "hook",
            "uninstall",
            "--settings",
            fixture.settings().to_str().unwrap(),
        ])
        .assert()
        .success();

    let uninstalled = read_json(&fixture.settings());
    assert_eq!(managed_count(&uninstalled, "PostToolUse"), 0);
    assert_eq!(uninstalled["permissions"]["allow"][0], "Bash(git status:*)");
}

fn read_json(path: &PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn managed_count(settings: &Value, event: &str) -> usize {
    settings["hooks"][event]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|entry| entry["_skillnet_managed"] == true)
                .count()
        })
        .unwrap_or(0)
}
