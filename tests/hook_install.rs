use std::{fs, path::PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::{tempdir, TempDir};

const SETTINGS_WITH_CUSTOM: &str = r#"{
  "permissions": {
    "allow": [
      "Bash(git status:*)"
    ]
  },
  "env": {
    "EXAMPLE": "preserved"
  },
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "echo user-hook"
          }
        ]
      }
    ]
  }
}"#;

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

    fn config(&self) -> PathBuf {
        self.temp.path().join("skillnet.toml")
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.env("SKILLNET_CONFIG", self.config());
        command.env_remove("SKILLNET_DATABASE_URL");
        command.env_remove("SKILLNET_DB_URL");
        command.env_remove("DATABASE_URL");
        command
    }
}

#[test]
fn install_empty_file_adds_one_post_tool_use_managed_entry() {
    let fixture = Fixture::new();
    fs::write(fixture.settings(), "").unwrap();

    fixture
        .command()
        .args([
            "hook",
            "install",
            "--settings",
            fixture.settings().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("installed"));

    let settings = read_json(&fixture.settings());
    assert_eq!(
        managed_count(&settings, "PostToolUse"),
        1,
        "expected exactly one PostToolUse managed entry"
    );
    assert_eq!(
        command_for(&settings, "PostToolUse"),
        "skillnet hook ingest --event PostToolUse"
    );
}

#[test]
fn install_twice_is_byte_idempotent() {
    let fixture = Fixture::new();
    fs::write(fixture.settings(), "{}\n").unwrap();

    install(&fixture);
    let first = fs::read(fixture.settings()).unwrap();
    install(&fixture);
    let second = fs::read(fixture.settings()).unwrap();

    assert_eq!(first, second);
    assert_eq!(
        managed_count(&read_json(&fixture.settings()), "PostToolUse"),
        1
    );
}

#[test]
fn install_existing_file_creates_single_backup_with_original_bytes() {
    let fixture = Fixture::new();
    let original = b"{\"env\":{\"EXAMPLE\":\"before\"}}\n";
    fs::write(fixture.settings(), original).unwrap();

    install(&fixture);
    install(&fixture);

    let backup = fixture.settings().with_file_name("settings.json.bak");
    assert_eq!(fs::read(backup).unwrap(), original);
}

#[test]
fn install_preserves_unrelated_post_tool_use_entry() {
    let fixture = Fixture::new();
    fs::write(fixture.settings(), SETTINGS_WITH_CUSTOM).unwrap();

    install(&fixture);

    let settings = read_json(&fixture.settings());
    assert_eq!(managed_count(&settings, "PostToolUse"), 1);
    assert_eq!(unmanaged_count(&settings, "PostToolUse"), 1);
    assert_eq!(settings["permissions"]["allow"][0], "Bash(git status:*)");
    assert_eq!(settings["env"]["EXAMPLE"], "preserved");
}

#[test]
fn uninstall_removes_managed_entries_and_preserves_user_settings() {
    let fixture = Fixture::new();
    fs::write(fixture.settings(), SETTINGS_WITH_CUSTOM).unwrap();

    install(&fixture);
    fixture
        .command()
        .args([
            "hook",
            "uninstall",
            "--settings",
            fixture.settings().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("uninstalled"));

    let settings = read_json(&fixture.settings());
    assert_eq!(managed_count(&settings, "PostToolUse"), 0);
    assert_eq!(unmanaged_count(&settings, "PostToolUse"), 1);
    assert!(settings["hooks"].get("SessionEnd").is_none());
    assert_eq!(settings["permissions"]["allow"][0], "Bash(git status:*)");
    assert_eq!(settings["env"]["EXAMPLE"], "preserved");
}

#[test]
fn uninstall_removes_empty_hooks_object_after_fresh_install() {
    let fixture = Fixture::new();
    fs::write(fixture.settings(), "{}\n").unwrap();

    install(&fixture);
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

    let settings = read_json(&fixture.settings());
    assert!(settings.get("hooks").is_none());
}

#[test]
fn uninstall_missing_settings_file_is_noop() {
    let fixture = Fixture::new();

    fixture
        .command()
        .args([
            "hook",
            "uninstall",
            "--settings",
            fixture.settings().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("uninstalled"));

    assert!(!fixture.settings().exists());
}

#[test]
fn status_reports_not_installed_then_installed() {
    let fixture = Fixture::new();
    fs::write(fixture.settings(), "{}\n").unwrap();

    fixture
        .command()
        .args([
            "hook",
            "status",
            "--settings",
            fixture.settings().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("not installed"));

    install(&fixture);

    fixture
        .command()
        .args([
            "hook",
            "status",
            "--settings",
            fixture.settings().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("installed"));
}

#[test]
fn install_refuses_invalid_json_without_truncating_file() {
    let fixture = Fixture::new();
    fs::write(fixture.settings(), "{ // comment\n").unwrap();

    fixture
        .command()
        .args([
            "hook",
            "install",
            "--settings",
            fixture.settings().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "failed to parse Claude settings JSON",
        ));

    assert_eq!(
        fs::read_to_string(fixture.settings()).unwrap(),
        "{ // comment\n"
    );
}

fn install(fixture: &Fixture) {
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
}

fn read_json(path: &PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn managed_count(settings: &Value, event: &str) -> usize {
    entries(settings, event)
        .iter()
        .filter(|entry| entry["_skillnet_managed"] == true)
        .count()
}

fn unmanaged_count(settings: &Value, event: &str) -> usize {
    entries(settings, event)
        .iter()
        .filter(|entry| entry["_skillnet_managed"] != true)
        .count()
}

fn command_for(settings: &Value, event: &str) -> String {
    entries(settings, event)[0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string()
}

fn entries<'a>(settings: &'a Value, event: &str) -> Vec<&'a Value> {
    settings["hooks"][event]
        .as_array()
        .unwrap_or_else(|| panic!("missing hooks.{event} array"))
        .iter()
        .collect()
}
