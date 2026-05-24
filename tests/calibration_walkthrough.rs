use std::{fs, path::PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::{params, Connection};
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

    fn write_decisions(&self, name: &str, content: &str) -> PathBuf {
        let path = self.repo.path().join(name);
        fs::write(&path, content).unwrap();
        path
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
fn walkthrough_accept_writes_accepted_proposal_and_threshold() {
    let fixture = Fixture::new();
    seed_candidate(&fixture.conn());
    let decisions = fixture.write_decisions(
        "accept.json",
        r#"[
          {
            "trigger": "long-serial-chain",
            "action": "accept",
            "rationale": "10/10 fires were dead weight"
          }
        ]"#,
    );

    fixture
        .command()
        .args([
            "calibration",
            "walkthrough",
            "--non-interactive",
            "--decisions",
            decisions.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("long-serial-chain: 4 → 5"))
        .stdout(predicate::str::contains(
            "- **Rationale**: 10/10 fires were dead weight",
        ))
        .stderr(predicate::str::contains("proposal 1 accepted"));

    let conn = fixture.conn();
    let proposal: (String, String) = conn
        .query_row(
            "SELECT trigger_name, decision FROM calibration_proposals WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        proposal,
        ("long-serial-chain".to_string(), "accepted".to_string())
    );
    let threshold: (f64, String) = conn
        .query_row(
            "SELECT threshold, updated_by
             FROM heuristic_thresholds
             WHERE name = 'long-serial-chain'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(threshold, (5.0, "proposal:1".to_string()));
}

#[test]
fn walkthrough_skip_writes_no_proposal() {
    let fixture = Fixture::new();
    seed_candidate(&fixture.conn());
    let decisions = fixture.write_decisions(
        "skip.json",
        r#"[
          {
            "trigger": "long-serial-chain",
            "action": "skip",
            "rationale": "wait for more data"
          }
        ]"#,
    );

    fixture
        .command()
        .args([
            "calibration",
            "walkthrough",
            "--non-interactive",
            "--decisions",
            decisions.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("no accepted proposals"))
        .stderr(predicate::str::contains("skipped long-serial-chain"));

    assert_eq!(proposal_count(&fixture.conn()), 0);
}

#[test]
fn walkthrough_dry_run_writes_nothing() {
    let fixture = Fixture::new();
    seed_candidate(&fixture.conn());
    let decisions = fixture.write_decisions(
        "accept.json",
        r#"[
          {
            "trigger": "long-serial-chain",
            "action": "accept",
            "rationale": "dry run acceptance"
          }
        ]"#,
    );

    fixture
        .command()
        .args([
            "calibration",
            "walkthrough",
            "--non-interactive",
            "--decisions",
            decisions.to_str().unwrap(),
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("[DRY RUN]"))
        .stderr(predicate::str::contains(
            "[DRY RUN] would accept long-serial-chain",
        ));

    assert_eq!(proposal_count(&fixture.conn()), 0);
}

#[test]
fn walkthrough_since_future_emits_empty_changelog() {
    let fixture = Fixture::new();
    seed_candidate(&fixture.conn());
    let decisions = fixture.write_decisions(
        "accept.json",
        r#"[
          {
            "trigger": "long-serial-chain",
            "action": "accept",
            "rationale": "accepted"
          }
        ]"#,
    );

    fixture
        .command()
        .args([
            "calibration",
            "walkthrough",
            "--non-interactive",
            "--decisions",
            decisions.to_str().unwrap(),
            "--since",
            "2099-01-01",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("no accepted proposals"));
}

#[test]
fn walkthrough_missing_decision_defaults_to_skip() {
    let fixture = Fixture::new();
    seed_candidate(&fixture.conn());
    let decisions = fixture.write_decisions("empty.json", "[]");

    fixture
        .command()
        .args([
            "calibration",
            "walkthrough",
            "--non-interactive",
            "--decisions",
            decisions.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("skipped long-serial-chain"));

    assert_eq!(proposal_count(&fixture.conn()), 0);
}

#[test]
fn walkthrough_without_since_dumps_full_history() {
    let fixture = Fixture::new();
    seed_accepted_history(&fixture.conn());

    fixture
        .command()
        .args(["calibration", "walkthrough", "--non-interactive"])
        .assert()
        .success()
        .stdout(predicate::str::contains("long-serial-chain: 4 → 5"))
        .stdout(predicate::str::contains("- **Rationale**: historical"));
}

#[test]
fn walkthrough_skill_md_detects_latest_changelog_date() {
    let fixture = Fixture::new();
    seed_accepted_history(&fixture.conn());
    let skill_md = fixture.repo.path().join("SKILL.md");
    fs::write(
        &skill_md,
        "## Calibration changelog\n\n### 2099-01-01 — long-serial-chain: 4 → 5\n",
    )
    .unwrap();

    fixture
        .command()
        .args([
            "calibration",
            "walkthrough",
            "--non-interactive",
            "--skill-md",
            skill_md.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("no accepted proposals"));
}

fn proposal_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM calibration_proposals", [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn seed_candidate(conn: &Connection) {
    for i in 0..10 {
        seed_row(
            conn,
            SeedRow {
                plan_id: format!("long-fire-{i}"),
                trigger: "long-serial-chain",
                fired: true,
                threshold: 4.0,
                outcome: "shipped",
                surprises: Some("dead-weight: long-serial-chain: too much ceremony"),
            },
        );
    }
}

fn seed_accepted_history(conn: &Connection) {
    conn.execute(
        "INSERT INTO calibration_proposals (
            proposed_at,
            trigger_name,
            current_threshold,
            proposed_threshold,
            supporting_plan_ids,
            fire_rate,
            signal_rate,
            filter_tags,
            decision,
            decided_at,
            rationale
        ) VALUES (1, 'long-serial-chain', 4.0, 5.0, '[\"support-1\"]', 1.0, -1.0, NULL, 'accepted', 1, 'historical')",
        [],
    )
    .unwrap();
}

struct SeedRow<'a> {
    plan_id: String,
    trigger: &'a str,
    fired: bool,
    threshold: f64,
    outcome: &'a str,
    surprises: Option<&'a str>,
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
        ) VALUES (?1, ?2, ?3, ?4, 'codex', 'refactor', 1, 1, 1, 1, '{}', ?5, '[]')",
        params![
            &row.plan_id,
            created_at(&row.plan_id),
            &row.plan_id,
            format!("/tmp/{}", &row.plan_id),
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
        ) VALUES (?1, 2, NULL, ?2, '{}', NULL, ?3)",
        params![&row.plan_id, row.outcome, row.surprises],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tags (plan_id, key, value) VALUES (?1, 'flavor', 'codex')",
        params![&row.plan_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tags (plan_id, key, value) VALUES (?1, 'worktype', 'refactor')",
        params![&row.plan_id],
    )
    .unwrap();
}

fn created_at(plan_id: &str) -> i64 {
    plan_id.bytes().map(i64::from).sum()
}
