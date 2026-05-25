use std::sync::{Mutex, OnceLock};

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

    fn reopen(&mut self) {
        match self {
            Self::Sqlite { db, _temp } => {
                let db_path = _temp.path().join("nested/calibration.sqlite");
                *db = Db::open(&db_path).unwrap();
            }
            #[cfg(feature = "postgres")]
            Self::Postgres { db, _schema } => {
                *db = Db::open_postgres(&_schema.connection_url()).unwrap();
            }
        }
    }
}

fn backends() -> Vec<(&'static str, BackendFixture)> {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("nested/calibration.sqlite");
    let sqlite = BackendFixture::Sqlite {
        db: Db::open(&db_path).unwrap(),
        _temp: temp,
    };
    #[cfg_attr(not(feature = "postgres"), allow(unused_mut))]
    let mut backends = vec![("sqlite", sqlite)];

    #[cfg(feature = "postgres")]
    {
        if let Ok(url) = std::env::var("SKILLNET_TEST_PG_URL") {
            let schema = PgTestSchema::create(&url);
            let schema_url = schema.connection_url();
            backends.push((
                "postgres",
                BackendFixture::Postgres {
                    db: Db::open_postgres(&schema_url).unwrap(),
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
        eprintln!("skipping postgres calibration DB tests; SKILLNET_TEST_PG_URL is unset");
    });
}

#[test]
fn opens_migrates_round_trips_and_enforces_cascades() {
    for (name, mut fx) in backends() {
        let plan_id = assert_round_trip(fx.db_mut(), name);

        let applied_before = applied_migration_count(fx.db_mut(), name);
        fx.reopen();
        assert_eq!(
            applied_migration_count(fx.db_mut(), name),
            applied_before,
            "{name}: migration count changed after reopen"
        );

        assert_cascade_delete_tags_only(fx.db_mut(), &plan_id, name);
    }
}

#[test]
fn migration_creates_skill_invocations_table_and_indexes() {
    for (name, mut fx) in backends() {
        assert_eq!(
            schema_version_applied(fx.db_mut(), 3, name),
            1,
            "{name}: migration 003 was not recorded"
        );
        assert_eq!(
            table_exists(fx.db_mut(), "skill_invocations", name),
            1,
            "{name}: skill_invocations table is missing"
        );
        assert_eq!(
            skill_invocations_index_count(fx.db_mut(), name),
            3,
            "{name}: skill_invocations indexes are missing"
        );
    }
}

#[test]
fn execute_returning_id_assigns_distinct_increasing_ids() {
    for (name, mut fx) in backends() {
        let plan_id = insert_plan(fx.db_mut(), name, "returning-id");
        let mut ids = Vec::new();
        for trigger in ["first", "second", "third"] {
            let id = fx
                .db_mut()
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
                        P::from(&plan_id),
                        P::from(trigger),
                        P::from(2.0),
                        P::from(1.0),
                        P::from(true),
                        P::from("Returning id"),
                    ],
                )
                .unwrap_or_else(|err| panic!("{name}: failed to insert trigger: {err:#}"));
            ids.push(id);
        }

        assert!(
            ids.iter().all(|id| *id > 0),
            "{name}: expected positive trigger ids, got {ids:?}"
        );
        assert!(
            ids.windows(2).all(|pair| pair[0] < pair[1]),
            "{name}: expected strictly increasing trigger ids, got {ids:?}"
        );
        assert_eq!(
            ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
            ids.len(),
            "{name}: expected distinct trigger ids, got {ids:?}"
        );
    }
}

#[test]
fn transaction_commits_on_ok_rolls_back_on_err() {
    for (name, mut fx) in backends() {
        let commit_plan_id = insert_plan(fx.db_mut(), name, "transaction-commit");
        fx.db_mut()
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                    &[
                        P::from(&commit_plan_id),
                        P::from("txn"),
                        P::from("committed"),
                    ],
                )?;
                Ok(())
            })
            .unwrap_or_else(|err| panic!("{name}: commit transaction failed: {err:#}"));
        assert_eq!(
            tag_count(fx.db_mut(), &commit_plan_id, "txn", "committed", name),
            1,
            "{name}: committed tag was not visible after transaction"
        );

        let rollback_plan_id = insert_plan(fx.db_mut(), name, "transaction-rollback");
        let err = match fx.db_mut().transaction(|tx| {
            tx.execute(
                "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                &[
                    P::from(&rollback_plan_id),
                    P::from("txn"),
                    P::from("rolled-back"),
                ],
            )?;
            Err::<(), anyhow::Error>(anyhow::anyhow!("rollback"))
        }) {
            Ok(()) => panic!("{name}: rollback transaction unexpectedly succeeded"),
            Err(err) => err,
        };
        assert_eq!(
            err.to_string(),
            "rollback",
            "{name}: rollback error message was not preserved"
        );
        assert_eq!(
            tag_count(fx.db_mut(), &rollback_plan_id, "txn", "rolled-back", name),
            0,
            "{name}: rolled-back tag was visible after transaction error"
        );
    }
}

#[test]
fn cascade_delete_purges_all_child_tables() {
    for (name, mut fx) in backends() {
        let plan_id = insert_plan(fx.db_mut(), name, "cascade-all");
        insert_child_rows(fx.db_mut(), &plan_id, name);
        fx.db_mut()
            .execute("DELETE FROM plans WHERE id = $1", &[P::from(&plan_id)])
            .unwrap_or_else(|err| panic!("{name}: failed to delete parent plan: {err:#}"));

        for table in ["triggers", "phases", "verifications", "tags"] {
            assert_eq!(
                child_count(fx.db_mut(), table, &plan_id, name),
                0,
                "{name}: {table} rows were not cascade-deleted"
            );
        }
    }
}

#[test]
fn query_optional_returns_none_for_missing_and_some_for_present() {
    for (name, mut fx) in backends() {
        let missing = Uuid::new_v4().to_string();
        let absent = fx
            .db_mut()
            .query_optional(
                "SELECT name FROM plans WHERE id = $1",
                &[P::from(&missing)],
                |row| row.get_string(0),
            )
            .unwrap_or_else(|err| panic!("{name}: failed to query missing plan: {err:#}"));
        assert_eq!(
            absent, None,
            "{name}: missing plan unexpectedly returned a row"
        );

        let plan_id = insert_plan(fx.db_mut(), name, "optional-present");
        let present = fx
            .db_mut()
            .query_optional(
                "SELECT name FROM plans WHERE id = $1",
                &[P::from(&plan_id)],
                |row| row.get_string(0),
            )
            .unwrap_or_else(|err| panic!("{name}: failed to query present plan: {err:#}"));
        assert_eq!(
            present,
            Some("optional-present".to_string()),
            "{name}: present plan returned the wrong name"
        );
    }
}

#[test]
fn query_all_returns_rows_in_order_by_clause() {
    for (name, mut fx) in backends() {
        let plan_id = insert_plan(fx.db_mut(), name, "ordered-phases");
        let files = json!(["src/lib.rs"]).to_string();
        for ordinal in [3_i64, 1, 4, 2] {
            let slug = format!("{ordinal:02}-ordered");
            fx.db_mut()
                .execute(
                    "INSERT INTO phases (plan_id, ordinal, slug, routing_tier, files)
                     VALUES ($1, $2, $3, $4, $5)",
                    &[
                        P::from(&plan_id),
                        P::from(ordinal),
                        P::from(&slug),
                        P::from("medium"),
                        P::from(&files),
                    ],
                )
                .unwrap_or_else(|err| panic!("{name}: failed to insert phase {ordinal}: {err:#}"));
        }

        let mut invocations = Vec::new();
        let rows = fx
            .db_mut()
            .query_all(
                "SELECT ordinal FROM phases WHERE plan_id = $1 ORDER BY ordinal DESC",
                &[P::from(&plan_id)],
                |row| {
                    let ordinal = row.get_i64(0)?;
                    invocations.push(ordinal);
                    Ok(ordinal)
                },
            )
            .unwrap_or_else(|err| panic!("{name}: failed to query ordered phases: {err:#}"));
        assert_eq!(rows, [4, 3, 2, 1], "{name}: query_all row order mismatch");
        assert_eq!(
            invocations,
            [4, 3, 2, 1],
            "{name}: query_all closure invocation order mismatch"
        );
    }
}

#[test]
fn nullable_params_round_trip_through_optional_accessors() {
    for (name, mut fx) in backends() {
        let null_plan_id = insert_plan_with_worktype(fx.db_mut(), name, "nullable-worktype", None);
        let some_plan_id =
            insert_plan_with_worktype(fx.db_mut(), name, "present-worktype", Some("analysis"));

        let worktypes = fx
            .db_mut()
            .query_one(
                "SELECT
                    (SELECT worktype FROM plans WHERE id = $1),
                    (SELECT worktype FROM plans WHERE id = $2)",
                &[P::from(&null_plan_id), P::from(&some_plan_id)],
                |row| Ok((row.get_optional_string(0)?, row.get_optional_string(1)?)),
            )
            .unwrap_or_else(|err| panic!("{name}: failed to query nullable worktypes: {err:#}"));
        assert_eq!(
            worktypes.0, None,
            "{name}: NULL worktype did not round-trip"
        );
        assert_eq!(
            worktypes.1,
            Some("analysis".to_string()),
            "{name}: non-null worktype did not round-trip"
        );

        let null_verification_plan = insert_plan(fx.db_mut(), name, "nullable-elapsed");
        let some_verification_plan = insert_plan(fx.db_mut(), name, "present-elapsed");
        insert_verification_elapsed(fx.db_mut(), name, &null_verification_plan, None);
        insert_verification_elapsed(fx.db_mut(), name, &some_verification_plan, Some(42));

        let elapsed = fx
            .db_mut()
            .query_one(
                "SELECT
                    (SELECT elapsed_seconds FROM verifications WHERE plan_id = $1),
                    (SELECT elapsed_seconds FROM verifications WHERE plan_id = $2)",
                &[
                    P::from(&null_verification_plan),
                    P::from(&some_verification_plan),
                ],
                |row| Ok((row.get_optional_i64(0)?, row.get_optional_i64(1)?)),
            )
            .unwrap_or_else(|err| {
                panic!("{name}: failed to query nullable elapsed values: {err:#}")
            });
        assert_eq!(
            elapsed.0, None,
            "{name}: NULL elapsed_seconds did not round-trip"
        );
        assert_eq!(
            elapsed.1,
            Some(42),
            "{name}: non-null elapsed_seconds did not round-trip"
        );
    }
}

#[test]
fn placeholder_reuse_binds_same_value_twice() {
    for (name, mut fx) in backends() {
        let values = fx
            .db_mut()
            .query_one("SELECT $1, $1", &[P::from("reuse-value")], |row| {
                Ok((row.get_string(0)?, row.get_string(1)?))
            })
            .unwrap_or_else(|err| panic!("{name}: failed to query reused placeholder: {err:#}"));
        assert_eq!(
            values,
            ("reuse-value".to_string(), "reuse-value".to_string()),
            "{name}: reused placeholder did not bind the same value twice"
        );
    }
}

#[test]
fn placeholder_in_string_literal_is_not_rewritten() {
    for (name, mut fx) in backends() {
        let plan_id = insert_plan(fx.db_mut(), name, "literal-placeholder");
        fx.db_mut()
            .execute(
                "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                &[
                    P::from(&plan_id),
                    P::from("literal"),
                    P::from("$1 is a literal"),
                ],
            )
            .unwrap_or_else(|err| panic!("{name}: failed to insert literal tag: {err:#}"));

        let value = fx
            .db_mut()
            .query_one(
                "SELECT value FROM tags WHERE plan_id = $1 AND key = $2",
                &[P::from(&plan_id), P::from("literal")],
                |row| row.get_string(0),
            )
            .unwrap_or_else(|err| panic!("{name}: failed to query literal tag: {err:#}"));
        assert_eq!(
            value, "$1 is a literal",
            "{name}: placeholder-looking tag value changed"
        );
    }
}

#[test]
fn execute_returns_rows_affected_count() {
    for (name, mut fx) in backends() {
        let mut plan_ids = Vec::new();
        for label in ["affected-count-a", "affected-count-b", "affected-count-c"] {
            plan_ids.push(insert_plan(fx.db_mut(), name, label));
        }
        for (plan_id, value) in plan_ids.iter().zip(["a", "b", "c"]) {
            fx.db_mut()
                .execute(
                    "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                    &[P::from(plan_id), P::from("affected"), P::from(value)],
                )
                .unwrap_or_else(|err| panic!("{name}: failed to insert affected tag: {err:#}"));
        }

        let updated = fx
            .db_mut()
            .execute(
                "UPDATE tags SET value = 'x' WHERE key = $1",
                &[P::from("affected")],
            )
            .unwrap_or_else(|err| panic!("{name}: failed to update affected tags: {err:#}"));
        assert_eq!(updated, 3, "{name}: UPDATE affected-row count mismatch");

        let deleted = fx
            .db_mut()
            .execute("DELETE FROM tags WHERE key = $1", &[P::from("missing")])
            .unwrap_or_else(|err| panic!("{name}: failed to delete missing tags: {err:#}"));
        assert_eq!(deleted, 0, "{name}: DELETE affected-row count mismatch");
    }
}

#[test]
fn json_blob_round_trips_semantic_equality() {
    for (name, mut fx) in backends() {
        let routing_dist = r#"{"medium":2,"high":1}"#;
        let plan_id = insert_plan_with_routing_dist(
            fx.db_mut(),
            name,
            "semantic-json",
            routing_dist,
            1_717_171_717,
        );
        let actual = fx
            .db_mut()
            .query_one(
                "SELECT routing_dist FROM plans WHERE id = $1",
                &[P::from(&plan_id)],
                |row| row.get_string(0),
            )
            .unwrap_or_else(|err| panic!("{name}: failed to query routing_dist: {err:#}"));
        let expected: serde_json::Value = serde_json::from_str(routing_dist)
            .unwrap_or_else(|err| panic!("{name}: failed to parse input JSON: {err:#}"));
        let actual: serde_json::Value = serde_json::from_str(&actual)
            .unwrap_or_else(|err| panic!("{name}: failed to parse returned JSON: {err:#}"));
        // Phase 02 design note: JSONB may normalize bytes, so parity is semantic equality.
        assert_eq!(actual, expected, "{name}: routing_dist JSON value mismatch");
    }
}

#[test]
fn unix_date_expr_groups_timestamps_into_calendar_days() {
    for (name, mut fx) in backends() {
        for (label, created_at) in [
            ("day-one-a", 1_717_171_700_i64),
            ("day-one-b", 1_717_171_705_i64),
            ("day-two", 1_717_258_100_i64),
        ] {
            insert_plan_with_created_at(fx.db_mut(), name, label, created_at);
        }

        let day_expr = fx.db_mut().unix_date_expr("created_at");
        let sql = format!(
            "SELECT {day_expr} AS day, COUNT(*)
             FROM plans
             GROUP BY 1
             ORDER BY 1"
        );
        let groups = fx
            .db_mut()
            .query_all(&sql, &[], |row| Ok((row.get_string(0)?, row.get_i64(1)?)))
            .unwrap_or_else(|err| panic!("{name}: failed to query unix date groups: {err:#}"));
        assert_eq!(
            groups.len(),
            2,
            "{name}: expected two calendar-day groups, got {groups:?}"
        );
        assert_eq!(
            groups.iter().map(|(_, count)| *count).collect::<Vec<_>>(),
            [2, 1],
            "{name}: calendar-day group counts mismatch: {groups:?}"
        );
    }
}

#[test]
fn migration_is_applied_exactly_once_across_reopens() {
    for (name, mut fx) in backends() {
        let applied_before = applied_migration_count(fx.db_mut(), name);
        fx.reopen();
        assert_eq!(
            applied_migration_count(fx.db_mut(), name),
            applied_before,
            "{name}: migration count changed after first reopen"
        );
        fx.reopen();
        assert_eq!(
            applied_migration_count(fx.db_mut(), name),
            applied_before,
            "{name}: migration count changed after second reopen"
        );
    }
}

fn assert_round_trip(db: &mut Db, name: &str) -> String {
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
    .unwrap_or_else(|err| panic!("{name}: failed to insert plan: {err:#}"));
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
    .unwrap_or_else(|err| panic!("{name}: failed to insert trigger: {err:#}"));
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
    .unwrap_or_else(|err| panic!("{name}: failed to insert phase: {err:#}"));

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
        .unwrap_or_else(|err| panic!("{name}: failed to query round-trip row: {err:#}"));
    assert_eq!(row.0, "synthetic plan", "{name}: plan name mismatch");
    assert_eq!(row.1, routing_dist, "{name}: routing_dist mismatch");
    assert_eq!(row.2, capture_reasons, "{name}: capture_reasons mismatch");
    assert!(row.3, "{name}: trigger fired flag mismatch");
    assert_eq!(row.4, files, "{name}: phase files mismatch");

    plan_id
}

fn assert_cascade_delete_tags_only(db: &mut Db, plan_id: &str, name: &str) {
    db.execute(
        "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
        &[P::from(plan_id), P::from("repo"), P::from("ai-skills")],
    )
    .unwrap_or_else(|err| panic!("{name}: failed to insert cascade tag: {err:#}"));
    db.execute("DELETE FROM plans WHERE id = $1", &[P::from(plan_id)])
        .unwrap_or_else(|err| panic!("{name}: failed to delete cascade plan: {err:#}"));

    let tag_count: i64 = db
        .query_one(
            "SELECT COUNT(*) FROM tags WHERE plan_id = $1",
            &[P::from(plan_id)],
            |row| row.get_i64(0),
        )
        .unwrap_or_else(|err| panic!("{name}: failed to count cascade tags: {err:#}"));
    assert_eq!(tag_count, 0, "{name}: tags were not cascade-deleted");
}

fn insert_plan(db: &mut Db, name: &str, label: &str) -> String {
    insert_plan_with_worktype(db, name, label, Some("test"))
}

fn insert_plan_with_worktype(
    db: &mut Db,
    name: &str,
    label: &str,
    worktype: Option<&str>,
) -> String {
    insert_plan_with(
        db,
        name,
        label,
        1_717_171_717,
        &json!({ "low": 1 }).to_string(),
        worktype,
    )
}

fn insert_plan_with_created_at(db: &mut Db, name: &str, label: &str, created_at: i64) -> String {
    insert_plan_with(
        db,
        name,
        label,
        created_at,
        &json!({ "low": 1 }).to_string(),
        Some("test"),
    )
}

fn insert_plan_with_routing_dist(
    db: &mut Db,
    name: &str,
    label: &str,
    routing_dist: &str,
    created_at: i64,
) -> String {
    insert_plan_with(db, name, label, created_at, routing_dist, Some("test"))
}

fn insert_plan_with(
    db: &mut Db,
    name: &str,
    label: &str,
    created_at: i64,
    routing_dist: &str,
    worktype: Option<&str>,
) -> String {
    let plan_id = Uuid::new_v4().to_string();
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
            P::from(created_at),
            P::from(label),
            P::from("docs/planning/example"),
            P::from("codex"),
            P::nullable_text(worktype),
            P::from(1_i64),
            P::from(1_i64),
            P::from(1_i64),
            P::from(1_i64),
            P::from(routing_dist),
            P::from(label),
            P::from(&capture_reasons),
        ],
    )
    .unwrap_or_else(|err| panic!("{name}: failed to insert plan {label}: {err:#}"));
    plan_id
}

fn insert_verification_elapsed(db: &mut Db, name: &str, plan_id: &str, elapsed: Option<i64>) {
    let phase_outcomes = json!([{ "ordinal": 1, "outcome": "pass" }]).to_string();
    db.execute(
        "INSERT INTO verifications (
                plan_id,
                verified_at,
                elapsed_seconds,
                outcome,
                phase_outcomes,
                emergency_changes,
                surprises
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            P::from(plan_id),
            P::from(1_717_171_718_i64),
            P::nullable_i64(elapsed),
            P::from("pass"),
            P::from(&phase_outcomes),
            P::Null,
            P::from("none"),
        ],
    )
    .unwrap_or_else(|err| panic!("{name}: failed to insert verification: {err:#}"));
}

fn insert_child_rows(db: &mut Db, plan_id: &str, name: &str) {
    let phase_outcomes = json!([{ "ordinal": 1, "outcome": "pass" }]).to_string();
    let files = json!(["src/lib.rs"]).to_string();
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
            P::from(plan_id),
            P::from("cascade-trigger"),
            P::from(2.0),
            P::from(1.0),
            P::from(true),
            P::from("Cascade"),
        ],
    )
    .unwrap_or_else(|err| panic!("{name}: failed to insert cascade trigger: {err:#}"));
    db.execute(
        "INSERT INTO phases (plan_id, ordinal, slug, routing_tier, files)
         VALUES ($1, $2, $3, $4, $5)",
        &[
            P::from(plan_id),
            P::from(1_i64),
            P::from("01-cascade"),
            P::from("medium"),
            P::from(&files),
        ],
    )
    .unwrap_or_else(|err| panic!("{name}: failed to insert cascade phase: {err:#}"));
    db.execute(
        "INSERT INTO verifications (
                plan_id,
                verified_at,
                elapsed_seconds,
                outcome,
                phase_outcomes,
                emergency_changes,
                surprises
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            P::from(plan_id),
            P::from(1_717_171_718_i64),
            P::from(60_i64),
            P::from("pass"),
            P::from(&phase_outcomes),
            P::Null,
            P::from("none"),
        ],
    )
    .unwrap_or_else(|err| panic!("{name}: failed to insert cascade verification: {err:#}"));
    db.execute(
        "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
        &[P::from(plan_id), P::from("cascade"), P::from("yes")],
    )
    .unwrap_or_else(|err| panic!("{name}: failed to insert cascade tag: {err:#}"));
}

fn child_count(db: &mut Db, table: &str, plan_id: &str, name: &str) -> i64 {
    assert!(
        matches!(table, "triggers" | "phases" | "verifications" | "tags"),
        "{name}: unsupported child table {table}"
    );
    db.query_one(
        &format!("SELECT COUNT(*) FROM {table} WHERE plan_id = $1"),
        &[P::from(plan_id)],
        |row| row.get_i64(0),
    )
    .unwrap_or_else(|err| panic!("{name}: failed to count {table}: {err:#}"))
}

fn tag_count(db: &mut Db, plan_id: &str, key: &str, value: &str, name: &str) -> i64 {
    db.query_one(
        "SELECT COUNT(*) FROM tags WHERE plan_id = $1 AND key = $2 AND value = $3",
        &[P::from(plan_id), P::from(key), P::from(value)],
        |row| row.get_i64(0),
    )
    .unwrap_or_else(|err| panic!("{name}: failed to count tags: {err:#}"))
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

fn applied_migration_count(db: &mut Db, name: &str) -> i64 {
    db.query_one("SELECT COUNT(*) FROM schema_versions", &[], |row| {
        row.get_i64(0)
    })
    .unwrap_or_else(|err| panic!("{name}: failed to count applied migrations: {err:#}"))
}

fn schema_version_applied(db: &mut Db, version: i64, name: &str) -> i64 {
    db.query_one(
        "SELECT COUNT(*) FROM schema_versions WHERE version = $1",
        &[P::from(version)],
        |row| row.get_i64(0),
    )
    .unwrap_or_else(|err| panic!("{name}: failed to inspect schema_versions: {err:#}"))
}

fn table_exists(db: &mut Db, table: &str, name: &str) -> i64 {
    let sql = match name {
        "postgres" => {
            "SELECT COUNT(*) FROM information_schema.tables
             WHERE table_schema = current_schema() AND table_name = $1"
        }
        _ => {
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = $1"
        }
    };
    db.query_one(sql, &[P::from(table)], |row| row.get_i64(0))
        .unwrap_or_else(|err| panic!("{name}: failed to inspect table {table}: {err:#}"))
}

fn skill_invocations_index_count(db: &mut Db, name: &str) -> i64 {
    let indexes = [
        P::from("skill_invocations_skill_name_idx"),
        P::from("skill_invocations_session_idx"),
        P::from("skill_invocations_started_at_idx"),
    ];
    let sql = match name {
        "postgres" => {
            "SELECT COUNT(*) FROM pg_indexes
             WHERE schemaname = current_schema()
               AND indexname IN ($1, $2, $3)"
        }
        _ => {
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index'
               AND name IN ($1, $2, $3)"
        }
    };
    db.query_one(sql, &indexes, |row| row.get_i64(0))
        .unwrap_or_else(|err| {
            panic!("{name}: failed to inspect skill_invocations indexes: {err:#}")
        })
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
