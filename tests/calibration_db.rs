use std::sync::{Mutex, OnceLock};

use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

#[path = "../src/calibration/db.rs"]
#[allow(dead_code)]
mod db;

use db::{Db, DbParam as P};

#[test]
fn opens_migrates_round_trips_and_enforces_cascades() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("nested/calibration.sqlite");

    let mut db = Db::open(&db_path).unwrap();
    assert!(db_path.is_file());

    let journal_mode = db.sqlite_pragma_string("journal_mode").unwrap();
    assert_eq!(journal_mode, "wal");

    let plan_id = assert_round_trip(&mut db);

    let applied_before = applied_migration_count(&db);
    drop(db);

    let mut db = Db::open(&db_path).unwrap();
    assert_eq!(applied_migration_count(&db), applied_before);

    assert_cascade_delete(&mut db, &plan_id);
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_opens_migrates_round_trips_and_enforces_cascades() {
    let Some(url) = std::env::var("SKILLNET_TEST_PG_URL").ok() else {
        eprintln!("skipping postgres round-trip test; SKILLNET_TEST_PG_URL is unset");
        return;
    };

    let schema = PgTestSchema::create(&url);
    let schema_url = schema.connection_url();
    let mut db = Db::open_postgres(&schema_url).unwrap();

    let plan_id = assert_round_trip(&mut db);
    let applied_before = applied_migration_count(&db);
    drop(db);

    let mut db = Db::open_postgres(&schema_url).unwrap();
    assert_eq!(applied_migration_count(&db), applied_before);

    assert_cascade_delete(&mut db, &plan_id);
}

#[cfg(not(feature = "postgres"))]
#[test]
fn postgres_round_trip_skipped_without_postgres_feature() {
    eprintln!("skipping postgres round-trip test; binary was built without the postgres feature");
}

fn assert_round_trip(db: &mut Db) -> String {
    let plan_id = Uuid::new_v4().to_string();
    let routing_dist = json!({ "medium": 2, "high": 1 }).to_string();
    let capture_reasons = json!(["phase-count", "repo-spread"]).to_string();
    let files = json!(["src/lib.rs"]).to_string();
    db.execute(
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
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        &[
            P::from(&plan_id),
            P::from(1_717_171_717_i64),
            P::from("synthetic plan"),
            P::from("docs/planning/example"),
            P::from("codex"),
            P::from("refactor"),
            P::from(3_i64),
            P::from(2_i64),
            P::from(1_i64),
            P::from(4_i64),
            P::from(&routing_dist),
            P::from("shape-123"),
            P::from(&capture_reasons),
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO triggers (
                plan_id,
                name,
                input_value,
                threshold,
                fired,
                section_added
            ) VALUES ($1, $2, $3, $4, $5, $6)",
        &[
            P::from(&plan_id),
            P::from("phase-count"),
            P::from(2.0),
            P::from(1.0),
            P::from(true),
            P::from("Phase count"),
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO phases (plan_id, ordinal, slug, routing_tier, files)
         VALUES ($1, $2, $3, $4, $5)",
        &[
            P::from(&plan_id),
            P::from(1_i64),
            P::from("01-round-trip"),
            P::from("medium"),
            P::from(&files),
        ],
    )
    .unwrap();

    let row: (String, String, String, bool, String) = db
        .query_one(
            "SELECT p.name, p.routing_dist, p.capture_reasons, t.fired, ph.files
             FROM plans p
             JOIN triggers t ON t.plan_id = p.id
             JOIN phases ph ON ph.plan_id = p.id
             WHERE p.id = $1",
            &[P::from(&plan_id)],
            |row| {
                Ok((
                    row.get_string(0)?,
                    row.get_string(1)?,
                    row.get_string(2)?,
                    row.get_bool(3)?,
                    row.get_string(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, "synthetic plan");
    assert_eq!(row.1, routing_dist);
    assert_eq!(row.2, capture_reasons);
    assert!(row.3);
    assert_eq!(row.4, files);

    plan_id
}

fn assert_cascade_delete(db: &mut Db, plan_id: &str) {
    db.execute(
        "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
        &[P::from(plan_id), P::from("repo"), P::from("ai-skills")],
    )
    .unwrap();
    db.execute("DELETE FROM plans WHERE id = $1", &[P::from(plan_id)])
        .unwrap();

    let tag_count: i64 = db
        .query_one(
            "SELECT COUNT(*) FROM tags WHERE plan_id = $1",
            &[P::from(plan_id)],
            |row| row.get_i64(0),
        )
        .unwrap();
    assert_eq!(tag_count, 0);
}

#[test]
fn default_path_uses_runtime_data_location() {
    let _guard = env_lock().lock().unwrap();
    let temp = tempdir().unwrap();
    let _env = EnvSnapshot::capture();
    std::env::set_var("skillnet_DATA_DIR", temp.path().join("lower"));
    std::env::set_var("SKILLNET_DATA_DIR", temp.path().join("upper"));
    std::env::set_var("AI_SKILLS_REPO", temp.path().join("repo"));
    std::env::set_var("XDG_DATA_HOME", temp.path().join("xdg"));

    let path = Db::default_path();
    assert_eq!(
        path,
        temp.path()
            .join("lower")
            .join("multi-phase-plan")
            .join("calibration.sqlite")
    );
}

#[test]
fn default_path_falls_back_to_uppercase_runtime_data_location() {
    let _guard = env_lock().lock().unwrap();
    let temp = tempdir().unwrap();
    let _env = EnvSnapshot::capture();
    std::env::remove_var("skillnet_DATA_DIR");
    std::env::set_var("SKILLNET_DATA_DIR", temp.path().join("upper"));
    std::env::set_var("AI_SKILLS_REPO", temp.path().join("repo"));
    std::env::set_var("XDG_DATA_HOME", temp.path().join("xdg"));

    assert_eq!(
        Db::default_path(),
        temp.path()
            .join("upper")
            .join("multi-phase-plan")
            .join("calibration.sqlite")
    );
}

#[test]
fn default_path_ignores_legacy_repo_fallback() {
    let _guard = env_lock().lock().unwrap();
    let temp = tempdir().unwrap();
    let _env = EnvSnapshot::capture();
    std::env::remove_var("skillnet_DATA_DIR");
    std::env::remove_var("SKILLNET_DATA_DIR");
    std::env::set_var("AI_SKILLS_REPO", temp.path().join("repo"));
    std::env::set_var("XDG_DATA_HOME", temp.path().join("xdg"));

    assert_eq!(
        Db::default_path(),
        temp.path()
            .join("xdg")
            .join("skillnet")
            .join("multi-phase-plan")
            .join("calibration.sqlite")
    );
}

fn applied_migration_count(db: &Db) -> i64 {
    db.query_one("SELECT COUNT(*) FROM schema_versions", &[], |row| {
        row.get_i64(0)
    })
    .unwrap()
}

#[cfg(feature = "postgres")]
struct PgTestSchema {
    url: String,
    name: String,
}

#[cfg(feature = "postgres")]
impl PgTestSchema {
    fn create(url: &str) -> Self {
        let name = format!("test_{}", Uuid::new_v4().simple());
        let mut client = postgres::Client::connect(url, postgres::NoTls)
            .expect("failed to connect to postgres test database");
        client
            .batch_execute(&format!("CREATE SCHEMA {name}"))
            .expect("failed to create postgres test schema");
        Self {
            url: url.to_string(),
            name,
        }
    }

    fn connection_url(&self) -> String {
        let separator = if self.url.contains('?') { '&' } else { '?' };
        format!(
            "{}{}options=-c%20search_path%3D{}",
            self.url, separator, self.name
        )
    }
}

#[cfg(feature = "postgres")]
impl Drop for PgTestSchema {
    fn drop(&mut self) {
        let Ok(mut client) = postgres::Client::connect(&self.url, postgres::NoTls) else {
            eprintln!("failed to reconnect to postgres test database while dropping test schema");
            return;
        };
        if let Err(err) =
            client.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.name))
        {
            eprintln!("failed to drop postgres test schema {}: {err}", self.name);
        }
    }
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvSnapshot {
    skillnet_data_dir: Option<String>,
    skillnet_data_dir_upper: Option<String>,
    ai_skills_repo: Option<String>,
    xdg_data_home: Option<String>,
}

impl EnvSnapshot {
    fn capture() -> Self {
        Self {
            skillnet_data_dir: std::env::var("skillnet_DATA_DIR").ok(),
            skillnet_data_dir_upper: std::env::var("SKILLNET_DATA_DIR").ok(),
            ai_skills_repo: std::env::var("AI_SKILLS_REPO").ok(),
            xdg_data_home: std::env::var("XDG_DATA_HOME").ok(),
        }
    }
}

impl Drop for EnvSnapshot {
    fn drop(&mut self) {
        restore_env("skillnet_DATA_DIR", self.skillnet_data_dir.as_deref());
        restore_env("SKILLNET_DATA_DIR", self.skillnet_data_dir_upper.as_deref());
        restore_env("AI_SKILLS_REPO", self.ai_skills_repo.as_deref());
        restore_env("XDG_DATA_HOME", self.xdg_data_home.as_deref());
    }
}

fn restore_env(key: &str, value: Option<&str>) {
    match value {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}
