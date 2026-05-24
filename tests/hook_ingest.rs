use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::{tempdir, TempDir};

const POST_TOOL_USE_SKILL: &str = r#"{
  "session_id": "ignored-session",
  "tool_name": "Skill",
  "tool_input": {
    "skill": "simplify",
    "args": {
      "target": "src/lib.rs"
    }
  },
  "tool_response": {
    "is_error": false,
    "content": "ok"
  }
}"#;

struct Fixture {
    repo: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            repo: tempdir().unwrap(),
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.repo.path().join("data")
    }

    fn config_path(&self) -> PathBuf {
        self.repo.path().join("skillnet.toml")
    }

    fn db_path(&self) -> PathBuf {
        self.data_dir().join("multi-phase-plan/calibration.sqlite")
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.env("skillnet_DATA_DIR", self.data_dir());
        command.env("SKILLNET_CONFIG", self.config_path());
        command.env_remove("SKILLNET_DATABASE_URL");
        command.env_remove("SKILLNET_DB_URL");
        command.env_remove("DATABASE_URL");
        command.env_remove("CLAUDE_HOOK_EVENT");
        command
    }

    fn conn(&self) -> Connection {
        Connection::open(self.db_path()).unwrap()
    }
}

#[test]
fn hook_ingest_records_post_tool_use_skill_payload() {
    let fixture = Fixture::new();

    fixture
        .command()
        .env("CLAUDE_SESSION_ID", "test-session-1")
        .env("CLAUDE_PROJECT_DIR", "/tmp/skillnet-project")
        .args(["hook", "ingest", "--event", "PostToolUse", "--strict"])
        .write_stdin(POST_TOOL_USE_SKILL)
        .assert()
        .success();

    let conn = fixture.conn();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM skill_invocations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);

    let (session_id, skill_name, tool_name, project_dir, outcome, hook_event): (
        String,
        String,
        String,
        String,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT session_id, skill_name, tool_name, project_dir, outcome, hook_event
             FROM skill_invocations",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(session_id, "test-session-1");
    assert_eq!(skill_name, "simplify");
    assert_eq!(tool_name, "Skill");
    assert_eq!(project_dir, "/tmp/skillnet-project");
    assert_eq!(outcome, "ok");
    assert_eq!(hook_event, "PostToolUse");
}

#[test]
fn hook_ingest_strict_rejects_malformed_payload() {
    let fixture = Fixture::new();

    fixture
        .command()
        .env("CLAUDE_SESSION_ID", "test-session-2")
        .args(["hook", "ingest", "--event", "PostToolUse", "--strict"])
        .write_stdin("{")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "failed to parse hook payload JSON",
        ));
}

#[test]
fn hook_ingest_non_strict_exits_zero_on_database_failure() {
    let fixture = Fixture::new();

    fixture
        .command()
        .env("CLAUDE_SESSION_ID", "test-session-3")
        .args([
            "--database-url",
            "not://a/postgres/url",
            "hook",
            "ingest",
            "--event",
            "PostToolUse",
        ])
        .write_stdin(POST_TOOL_USE_SKILL)
        .assert()
        .success()
        .stderr(predicate::str::contains("skillnet hook ingest failed"));
}
