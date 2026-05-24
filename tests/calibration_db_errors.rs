use std::panic::{self, AssertUnwindSafe};
#[cfg(feature = "postgres")]
use std::sync::OnceLock;

use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

#[path = "../src/calibration/db.rs"]
#[allow(dead_code)]
mod db;

use db::{Db, DbParam as P};

enum BackendFixture {
    Sqlite {
        db: Db,
        _temp: tempfile::TempDir,
    },
    #[cfg(feature = "postgres")]
    Postgres {
        db: Db,
        _schema: PgTestSchema,
    },
}

impl BackendFixture {
    fn db_mut(&mut self) -> &mut Db {
        match self {
            Self::Sqlite { db, .. } => db,
            #[cfg(feature = "postgres")]
            Self::Postgres { db, .. } => db,
        }
    }
}

fn backends() -> Vec<(&'static str, BackendFixture)> {
    let temp = tempdir().expect("failed to create sqlite tempdir");
    let db_path = temp.path().join("nested/calibration.sqlite");
    let sqlite = BackendFixture::Sqlite {
        db: Db::open(&db_path).expect("failed to open sqlite test database"),
        _temp: temp,
    };
    #[cfg(not(feature = "postgres"))]
    let backends = vec![("sqlite", sqlite)];
    #[cfg(feature = "postgres")]
    let mut backends = vec![("sqlite", sqlite)];

    #[cfg(feature = "postgres")]
    {
        if let Ok(url) = std::env::var("SKILLNET_TEST_PG_URL") {
            let schema = PgTestSchema::create(&url);
            let schema_url = schema.connection_url();
            backends.push((
                "postgres",
                BackendFixture::Postgres {
                    db: Db::open_postgres(&schema_url)
                        .expect("failed to open postgres test database"),
                    _schema: schema,
                },
            ));
        } else {
            log_postgres_skip_once();
        }
    }

    backends
}

#[cfg(feature = "postgres")]
fn log_postgres_skip_once() {
    static SKIP_NOTICE: OnceLock<()> = OnceLock::new();
    SKIP_NOTICE.get_or_init(|| {
        eprintln!("skipping postgres calibration DB error tests; SKILLNET_TEST_PG_URL is unset");
    });
}

#[test]
fn query_one_errors_when_no_rows_match() {
    for (name, mut fx) in backends() {
        let result = fx.db_mut().query_one(
            "SELECT value FROM tags WHERE key = $1",
            &[P::from("missing-key")],
            |row| row.get_string(0),
        );
        assert!(
            result.is_err(),
            "{name}: query_one unexpectedly succeeded with zero matching rows"
        );
        let err = match result {
            Ok(value) => panic!("{name}: query_one unexpectedly returned {value:?}"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            !message.is_empty(),
            "{name}: no-row query_one error message was empty"
        );
        assert!(
            message.contains("no rows") || message.contains("tags"),
            "{name}: no-row query_one error did not name the failure or table: {message}"
        );
    }
}

#[test]
fn query_one_errors_when_multiple_rows_match() {
    for (name, mut fx) in backends() {
        let plan_id = insert_plan(fx.db_mut(), name, "query-one-multiple");
        for value in ["first", "second"] {
            fx.db_mut()
                .execute(
                    "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                    &[P::from(&plan_id), P::from("duplicate"), P::from(value)],
                )
                .unwrap_or_else(|err| panic!("{name}: failed to insert duplicate tag: {err:#}"));
        }

        let result = fx.db_mut().query_one(
            "SELECT value FROM tags WHERE plan_id = $1 AND key = $2",
            &[P::from(&plan_id), P::from("duplicate")],
            |row| row.get_string(0),
        );
        assert!(
            result.is_err(),
            "{name}: query_one unexpectedly succeeded with multiple matching rows"
        );
        let err = match result {
            Ok(value) => panic!("{name}: query_one returned first matching row {value:?}"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            !message.is_empty(),
            "{name}: multiple-row query_one error message was empty"
        );
        assert!(
            message.contains("more than one")
                || message.contains("multiple")
                || message.contains("tags")
                || message.contains("SELECT value"),
            "{name}: multiple-row query_one error did not name the failure or query: {message}"
        );
    }
}

#[test]
fn db_row_accessor_type_mismatch_returns_anyhow_error() {
    for (name, mut fx) in backends() {
        let plan_id = insert_plan(fx.db_mut(), name, "type-mismatch");
        fx.db_mut()
            .execute(
                "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                &[
                    P::from(&plan_id),
                    P::from("typed-key"),
                    P::from("typed-value"),
                ],
            )
            .unwrap_or_else(|err| panic!("{name}: failed to insert tag: {err:#}"));

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            fx.db_mut().query_one(
                "SELECT key FROM tags WHERE plan_id = $1 AND key = $2",
                &[P::from(&plan_id), P::from("typed-key")],
                |row| row.get_i64(0),
            )
        }));
        let query_result = match result {
            Ok(result) => result,
            Err(payload) => panic::resume_unwind(payload),
        };
        assert!(
            query_result.is_err(),
            "{name}: get_i64 unexpectedly succeeded for text column"
        );
        let err = match query_result {
            Ok(value) => panic!("{name}: get_i64 unexpectedly returned {value} for text column"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            !message.is_empty(),
            "{name}: type-mismatch accessor error message was empty"
        );
        assert!(
            message.contains("column 0")
                || message.contains("integer")
                || message.contains("key")
                || message.contains("SELECT key"),
            "{name}: type-mismatch accessor error was not actionable: {message}"
        );
    }
}

#[test]
fn transaction_rolls_back_on_panic_inside_closure() {
    for (name, mut fx) in backends() {
        let plan_id = insert_plan(fx.db_mut(), name, "panic-rollback");
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = fx.db_mut().transaction::<()>(|tx| {
                tx.execute(
                    "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                    &[P::from(&plan_id), P::from("panic"), P::from("rolled-back")],
                )?;
                panic!("{name}: intentional transaction panic");
            });
        }));
        assert!(
            result.is_err(),
            "{name}: transaction closure panic was not observed by catch_unwind"
        );

        let count = tag_count(fx.db_mut(), &plan_id, "panic", "rolled-back", name);
        assert_eq!(
            count, 0,
            "{name}: row inserted before transaction panic was not rolled back"
        );
    }
}

#[cfg(feature = "postgres")]
#[test]
fn open_postgres_with_invalid_url_returns_actionable_error() {
    let result = Db::open_postgres("not://a/url");
    assert!(
        result.is_err(),
        "open_postgres unexpectedly accepted an invalid URL"
    );
    let err = match result {
        Ok(_) => panic!("open_postgres unexpectedly accepted an invalid URL"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(
        !message.is_empty(),
        "invalid postgres URL error message was empty"
    );
    assert!(
        message.contains("not://a/url")
            || message.contains("postgres")
            || message.contains("connect"),
        "invalid postgres URL error was not actionable: {message}"
    );
}

#[test]
fn open_sqlite_in_unwritable_dir_returns_actionable_error() {
    let temp = tempdir().expect("failed to create tempdir");
    let notadir = temp.path().join("notadir");
    std::fs::write(&notadir, b"regular file").expect("failed to create notadir test file");
    let db_path = notadir.join("x.sqlite");

    let result = Db::open(&db_path);
    assert!(
        result.is_err(),
        "Db::open unexpectedly created sqlite database under {}",
        notadir.display()
    );
    let err = match result {
        Ok(_) => panic!(
            "Db::open unexpectedly created sqlite database under {}",
            notadir.display()
        ),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(
        !message.is_empty(),
        "unwritable sqlite path error message was empty"
    );
    assert!(
        message.contains("notadir"),
        "unwritable sqlite path error did not name notadir: {message}"
    );
}

fn insert_plan(db: &mut Db, name: &str, label: &str) -> String {
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
            P::from(&plan_id),
            P::from(1_717_171_717_i64),
            P::from(label),
            P::from("docs/planning/example"),
            P::from("codex"),
            P::from("test"),
            P::from(1_i64),
            P::from(1_i64),
            P::from(1_i64),
            P::from(1_i64),
            P::from(&routing_dist),
            P::from(label),
            P::from(&capture_reasons),
        ],
    )
    .unwrap_or_else(|err| panic!("{name}: failed to insert plan {label}: {err:#}"));
    plan_id
}

fn tag_count(db: &mut Db, plan_id: &str, key: &str, value: &str, name: &str) -> i64 {
    db.query_one(
        "SELECT COUNT(*) FROM tags WHERE plan_id = $1 AND key = $2 AND value = $3",
        &[P::from(plan_id), P::from(key), P::from(value)],
        |row| row.get_i64(0),
    )
    .unwrap_or_else(|err| panic!("{name}: failed to count tags: {err:#}"))
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
