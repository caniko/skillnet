#[cfg(feature = "postgres")]
use serde_json::json;
use tempfile::tempdir;
#[cfg(feature = "postgres")]
use uuid::Uuid;

#[path = "../src/calibration/db.rs"]
#[allow(dead_code)]
mod db;

use db::Db;
#[cfg(feature = "postgres")]
use db::DbParam as P;

#[test]
fn sqlite_enforces_wal_journal_mode() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("nested/calibration.sqlite");
    let db = Db::open(&db_path).unwrap();

    assert_eq!(
        db.sqlite_pragma_string("journal_mode").unwrap(),
        "wal",
        "sqlite: expected WAL journal mode"
    );
}

#[test]
fn sqlite_enforces_foreign_keys_pragma() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("nested/calibration.sqlite");
    let db = Db::open(&db_path).unwrap();

    let enabled: i64 = db
        .query_one("PRAGMA foreign_keys", &[], |row| row.get_i64(0))
        .unwrap();
    assert_eq!(enabled, 1, "sqlite: expected foreign_keys pragma enabled");
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_returning_id_handles_bigserial_above_i32_max() {
    let Some(schema) = postgres_schema_fixture() else {
        return;
    };
    let db = Db::open_postgres(&schema.connection_url()).unwrap();
    let plan_id = insert_plan(&db, "postgres-bigserial");

    let qualified_triggers = format!("{}.triggers", schema.name);
    let sequence_name: String = db
        .query_one(
            "SELECT pg_get_serial_sequence($1, $2)",
            &[P::from(qualified_triggers.as_str()), P::from("id")],
            |row| row.get_string(0),
        )
        .unwrap();
    db.query_one(
        "SELECT setval($1::regclass, $2)",
        &[P::from(sequence_name.as_str()), P::from(3_000_000_000_i64)],
        |row| row.get_i64(0),
    )
    .unwrap();

    let id = db
        .execute_returning_id(
            "INSERT INTO triggers (
                plan_id,
                name,
                input_value,
                threshold,
                fired,
                section_added
            ) VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                P::from(plan_id.as_str()),
                P::from("bigserial"),
                P::from(2.0),
                P::from(1.0),
                P::from(true),
                P::from("BIGSERIAL"),
            ],
        )
        .unwrap();

    assert_eq!(id, 3_000_000_001);
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_migration_isolated_to_search_path_schema() {
    let Some(schema) = postgres_schema_fixture() else {
        return;
    };
    let _db = Db::open_postgres(&schema.connection_url()).unwrap();
    let mut admin = postgres::Client::connect(&schema.url, postgres::NoTls)
        .expect("failed to connect to postgres test database");

    let isolated_exists: bool = admin
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = $1 AND table_name = 'schema_versions'
            )",
            &[&schema.name],
        )
        .expect("failed to inspect isolated test schema")
        .get(0);
    let public_exists: bool = admin
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = 'public' AND table_name = 'schema_versions'
            )",
            &[],
        )
        .expect("failed to inspect public schema")
        .get(0);

    assert!(
        isolated_exists,
        "postgres: schema_versions missing from test schema"
    );
    assert!(
        !public_exists,
        "postgres: migration leaked schema_versions into public"
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_jsonb_rejects_invalid_json() {
    let Some(schema) = postgres_schema_fixture() else {
        return;
    };
    let db = Db::open_postgres(&schema.connection_url()).unwrap();
    let plan_id = Uuid::new_v4().to_string();
    let capture_reasons = json!(["invalid-json"]).to_string();

    let err = db
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
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::text::jsonb, $12, $13)",
            &[
                P::from(plan_id.as_str()),
                P::from(1_717_171_717_i64),
                P::from("invalid json plan"),
                P::from("docs/planning/example"),
                P::from("codex"),
                P::from("test"),
                P::from(1_i64),
                P::from(1_i64),
                P::from(1_i64),
                P::from(1_i64),
                P::from("not-json"),
                P::from("invalid-json"),
                P::from(capture_reasons.as_str()),
            ],
        )
        .expect_err("postgres: invalid JSONB insert unexpectedly succeeded");
    let message = err.to_string().to_ascii_lowercase();
    assert!(
        message.contains("json") || message.contains("routing_dist"),
        "postgres: expected JSONB error to mention json or routing_dist, got: {err:#}"
    );
}

#[cfg(feature = "postgres")]
fn postgres_schema_fixture() -> Option<PgTestSchema> {
    match std::env::var("SKILLNET_TEST_PG_URL") {
        Ok(url) => Some(PgTestSchema::create(&url)),
        Err(_) => {
            log_postgres_skip_once();
            None
        }
    }
}

#[cfg(feature = "postgres")]
fn log_postgres_skip_once() {
    use std::sync::OnceLock;

    static SKIP_NOTICE: OnceLock<()> = OnceLock::new();
    SKIP_NOTICE.get_or_init(|| {
        eprintln!("skipping postgres backend calibration DB tests; SKILLNET_TEST_PG_URL is unset");
    });
}

#[cfg(feature = "postgres")]
fn insert_plan(db: &Db, label: &str) -> String {
    let plan_id = Uuid::new_v4().to_string();
    let routing_dist = json!({ "low": 1 }).to_string();
    let capture_reasons = json!([label]).to_string();
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
            P::from(plan_id.as_str()),
            P::from(1_717_171_717_i64),
            P::from(label),
            P::from("docs/planning/example"),
            P::from("codex"),
            P::from("test"),
            P::from(1_i64),
            P::from(1_i64),
            P::from(1_i64),
            P::from(1_i64),
            P::from(routing_dist.as_str()),
            P::from(label),
            P::from(capture_reasons.as_str()),
        ],
    )
    .unwrap();
    plan_id
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
