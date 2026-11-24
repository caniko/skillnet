use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::json;
use tempfile::{tempdir, TempDir};

const POST_TOOL_USE_SKILL: &str = r#"{
  "session_id": " payload-session ",
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
        command.env_remove("CLAUDE_PROJECT_DIR");
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
    assert_eq!(session_id, "payload-session");
    assert_eq!(skill_name, "simplify");
    assert_eq!(tool_name, "Skill");
    assert_eq!(project_dir, "/tmp/skillnet-project");
    assert_eq!(outcome, "ok");
    assert_eq!(hook_event, "PostToolUse");
}

#[test]
fn hook_ingest_qualifies_project_skills_and_dedupes_per_harness() {
    let fixture = Fixture::new();
    let project = fixture.repo.path().join("feature-worktree");
    let skill = project.join(".agents/skills/simplify");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "").unwrap();
    std::fs::write(
        fixture.config_path(),
        format!(
            "[global]\nviews = []\n[[projects]]\nname = \"canix\"\npath = {:?}\n",
            project.to_str().unwrap()
        ),
    )
    .unwrap();
    let payload = serde_json::json!({
        "session_id": " ",
        "sessionId": " actual-session ",
        "projectDir": format!(" {} ", project.display()),
        "cwd": project.join(".agents"),
        "tool_name": " Skill ",
        "tool_input": {"skill": " simplify "}
    });
    for harness in ["codex", "codex", "claude"] {
        fixture
            .command()
            .env("CLAUDE_PROJECT_DIR", "/wrong")
            .args([
                "hook",
                "ingest",
                "--event",
                "PostToolUse",
                "--strict",
                "--harness",
                harness,
            ])
            .write_stdin(payload.to_string())
            .assert()
            .success();
    }
    let conn = fixture.conn();
    for harness in ["claude", "codex"] {
        let (count, event_id): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MIN(source_event_id) FROM skill_invocations
             WHERE harness = ?1 AND session_id = 'actual-session' AND project_dir = ?2
             AND skill_name = 'simplify' AND canonical_skill_name = 'canix/simplify'",
                rusqlite::params![harness, project.to_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert!(event_id.starts_with(&format!("{harness}:actual-session:PostToolUse:")));
    }
}

#[test]
fn hook_ingest_resolves_repo_root_and_claude_skill_shadows_global() {
    let fixture = Fixture::new();
    let global = fixture.repo.path().join("global_skills");
    let project = fixture.repo.path().join("feature-worktree");
    let nested = project.join("src/nested");
    for skill in [
        global.join("simplify"),
        project.join(".claude/skills/simplify"),
    ] {
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "").unwrap();
    }
    std::fs::write(project.join(".git"), "gitdir: /unused\n").unwrap();
    std::fs::create_dir_all(&nested).unwrap();
    let global_config = format!("[global]\ncanonical_path = {global:?}\nviews = []\n");
    for (session, project_config, use_env) in [
        (
            "env",
            format!("[[projects]]\nname = \"canix\"\npath = {project:?}\n"),
            true,
        ),
        (
            "cwd",
            format!("[[projects]]\nname = \"canix\"\npath = {project:?}\n"),
            false,
        ),
        ("unconfigured", String::new(), false),
    ] {
        std::fs::write(
            fixture.config_path(),
            format!("{global_config}{project_config}"),
        )
        .unwrap();
        let mut cmd = fixture.command();
        if use_env {
            cmd.env("CLAUDE_PROJECT_DIR", &project);
        }
        cmd.args(["hook", "ingest", "--event", "PostToolUse", "--strict"])
            .write_stdin(
                json!({
                    "session_id": session, "cwd": nested,
                    "tool_name":"Skill", "tool_input":{"skill":"simplify"},
                })
                .to_string(),
            )
            .assert()
            .success();
    }
    let db = fixture.conn();
    for (session, name) in [
        ("env", "canix/simplify"),
        ("cwd", "canix/simplify"),
        ("unconfigured", "simplify"),
    ] {
        let actual: (String, String) = db
            .query_row(
                "SELECT project_dir, canonical_skill_name FROM skill_invocations WHERE session_id = ?1",
                [session],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            actual,
            (project.to_str().unwrap().to_owned(), name.to_owned())
        );
    }
}

#[test]
fn hook_ingest_qualifies_global_skills_but_preserves_explicit_scope() {
    let fixture = Fixture::new();
    let global = fixture.repo.path().join("global_skills");
    std::fs::create_dir_all(global.join("simplify")).unwrap();
    std::fs::write(global.join("simplify/SKILL.md"), "").unwrap();
    std::fs::write(
        fixture.config_path(),
        format!(
            "[global]\ncanonical_path = {:?}\nviews = []\n",
            global.to_str().unwrap()
        ),
    )
    .unwrap();
    for skill in ["simplify", "somewhere/simplify"] {
        fixture
            .command()
            .args(["hook", "ingest", "--event", "PostToolUse", "--strict"])
            .env("CLAUDE_SESSION_ID", " fallback ")
            .env("CLAUDE_PROJECT_DIR", " /no-project ")
            .write_stdin(
                serde_json::json!({"tool_name":"Skill", "tool_input":{"skill":skill}}).to_string(),
            )
            .assert()
            .success();
    }
    let db = fixture.conn();
    let mut query = db
        .prepare(
            "SELECT canonical_skill_name FROM skill_invocations
         WHERE session_id = 'fallback' AND project_dir = '/no-project' AND harness = 'claude'
         ORDER BY canonical_skill_name",
        )
        .unwrap();
    let names = query
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(names, ["global/simplify", "somewhere/simplify"]);
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
