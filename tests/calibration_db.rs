use std::{env, path::PathBuf};

use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

#[path = "../src/calibration/db.rs"]
#[allow(dead_code)]
mod db;

use db::Db;

#[test]
fn opens_migrates_round_trips_and_enforces_cascades() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("nested/calibration.sqlite");

    let db = Db::open(&db_path).unwrap();
    assert!(db_path.is_file());

    let journal_mode: String = db
        .connection()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode, "wal");

    let plan_id = Uuid::new_v4().to_string();
    let routing_dist = json!({ "medium": 2, "high": 1 }).to_string();
    let capture_reasons = json!(["phase-count", "repo-spread"]).to_string();
    db.connection()
        .execute(
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
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            (
                &plan_id,
                1_717_171_717_i64,
                "synthetic plan",
                "docs/planning/example",
                "codex",
                "refactor",
                3_i64,
                2_i64,
                1_i64,
                4_i64,
                &routing_dist,
                "shape-123",
                &capture_reasons,
            ),
        )
        .unwrap();

    let row: (String, String, String) = db
        .connection()
        .query_row(
            "SELECT name, routing_dist, capture_reasons FROM plans WHERE id = ?1",
            [&plan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, "synthetic plan");
    assert_eq!(row.1, routing_dist);
    assert_eq!(row.2, capture_reasons);

    let applied_before = applied_migration_count(&db);
    drop(db);

    let db = Db::open(&db_path).unwrap();
    assert_eq!(applied_migration_count(&db), applied_before);

    db.connection()
        .execute(
            "INSERT INTO tags (plan_id, key, value) VALUES (?1, ?2, ?3)",
            (&plan_id, "repo", "ai-skills"),
        )
        .unwrap();
    db.connection()
        .execute("DELETE FROM plans WHERE id = ?1", [&plan_id])
        .unwrap();

    let tag_count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM tags WHERE plan_id = ?1",
            [&plan_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tag_count, 0);
}

#[test]
fn default_path_uses_runtime_data_location() {
    let path = Db::default_path();
    let expected = if let Ok(dir) = env::var("SKILLNET_DATA_DIR") {
        PathBuf::from(dir)
            .join("multi-phase-plan")
            .join("calibration.sqlite")
    } else if let Ok(repo_root) = env::var("AI_SKILLS_REPO") {
        PathBuf::from(repo_root).join("data/multi-phase-plan/calibration.sqlite")
    } else if let Ok(dir) = env::var("XDG_DATA_HOME") {
        PathBuf::from(dir)
            .join("skillnet")
            .join("multi-phase-plan")
            .join("calibration.sqlite")
    } else {
        PathBuf::from(env::var("HOME").unwrap())
            .join(".local/share/skillnet/multi-phase-plan/calibration.sqlite")
    };

    assert_eq!(path, expected);
}

fn applied_migration_count(db: &Db) -> i64 {
    db.connection()
        .query_row("SELECT COUNT(*) FROM schema_versions", [], |row| row.get(0))
        .unwrap()
}
