use std::{fs, path::PathBuf};

use assert_cmd::Command;
use rusqlite::Connection;
use tempfile::{tempdir, TempDir};

struct Fixture {
    root: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempdir().unwrap();
        let global = root.path().join("global");
        fs::create_dir_all(global.join("alpha")).unwrap();
        fs::create_dir_all(global.join("beta")).unwrap();
        fs::write(global.join("alpha/SKILL.md"), "---\nname: alpha\n---\n").unwrap();
        fs::write(global.join("beta/SKILL.md"), "---\nname: beta\n---\n").unwrap();
        fs::write(
            root.path().join("skillnet.toml"),
            format!(
                "[global]\ncanonical_path = {:?}\nviews = []\n",
                global.to_string_lossy()
            ),
        )
        .unwrap();
        fs::write(root.path().join("skillnet.catalog.toml"), "").unwrap();
        Self { root }
    }

    fn data_dir(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command
            .env("skillnet_DATA_DIR", self.data_dir())
            .env("SKILLNET_CONFIG", self.root.path().join("skillnet.toml"))
            .env(
                "SKILLNET_CATALOG_CONFIG",
                self.root.path().join("skillnet.catalog.toml"),
            )
            .env_remove("SKILLNET_DATABASE_URL")
            .env_remove("SKILLNET_DB_URL")
            .env_remove("DATABASE_URL");
        command
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir().join("multi-phase-plan/calibration.sqlite")).unwrap()
    }
}

#[test]
fn usage_record_is_idempotent_and_privacy_preserving() {
    let fixture = Fixture::new();
    for _ in 0..2 {
        fixture
            .command()
            .args([
                "usage",
                "record",
                "--skill",
                "global/alpha",
                "--harness",
                "codex",
                "--session",
                "session-hash",
                "--event-id",
                "event-1",
            ])
            .assert()
            .success();
    }

    let conn = fixture.conn();
    let (count, payload): (i64, String) = conn
        .query_row(
            "SELECT COUNT(*), max(payload) FROM skill_invocations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(payload, "{}");
}

#[test]
fn usage_report_zero_fills_current_catalog() {
    let fixture = Fixture::new();
    fixture
        .command()
        .args([
            "usage",
            "record",
            "--skill",
            "global/alpha",
            "--harness",
            "claude",
            "--session",
            "session-hash",
            "--event-id",
            "event-1",
        ])
        .assert()
        .success();

    let output = fixture
        .command()
        .args(["usage", "report", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("global/alpha"));
    assert!(text.contains("global/beta"));
    assert!(text.contains("\"uses\": 0"));
}
