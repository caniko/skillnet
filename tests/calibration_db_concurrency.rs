use std::{sync::mpsc, thread, time::Duration};

use tempfile::tempdir;
use uuid::Uuid;

#[path = "../src/calibration/db.rs"]
#[allow(dead_code)]
mod db;

use db::{Db, DbParam as P};

#[test]
#[ignore = "timing-sensitive SQLite WAL smoke test; run explicitly with --ignored"]
fn sqlite_wal_allows_concurrent_readers() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("calibration.sqlite");
    let mut writer = Db::open(&db_path).unwrap();
    let plan_id = insert_plan(&writer, "wal-readers");

    let (tx, rx) = mpsc::channel();
    let reader_path = db_path.clone();
    let reader_plan_id = plan_id.clone();

    writer
        .transaction(|writer_tx| {
            writer_tx.execute(
                "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                &[
                    P::from(&plan_id),
                    P::from("concurrency"),
                    P::from("uncommitted"),
                ],
            )?;

            thread::spawn(move || {
                let result = (|| -> anyhow::Result<i64> {
                    let reader = Db::open(&reader_path)?;
                    tag_count(&reader, &reader_plan_id, "concurrency", "uncommitted")
                })();
                tx.send(result.map_err(|err| format!("{err:#}"))).unwrap();
            });

            let observed = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_else(|_| {
                panic!("WAL reader was blocked by uncommitted writer for more than 2s")
            });
            assert_eq!(
                observed.unwrap(),
                0,
                "reader observed an uncommitted SQLite WAL write"
            );

            Ok(())
        })
        .unwrap();

    let reader = Db::open(&db_path).unwrap();
    assert_eq!(
        tag_count(&reader, &plan_id, "concurrency", "uncommitted").unwrap(),
        1,
        "reader did not observe committed SQLite WAL write"
    );
}

#[test]
#[ignore = "timing-sensitive SQLite WAL smoke test; run explicitly with --ignored"]
fn sqlite_writers_serialise_without_panic() {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("calibration.sqlite");
    let db = Db::open(&db_path).unwrap();
    let plan_id = insert_plan(&db, "wal-writers");

    let handles = [0, 1].map(|writer_id| {
        let db_path = db_path.clone();
        let plan_id = plan_id.clone();
        thread::spawn(move || -> anyhow::Result<()> {
            let writer = Db::open(&db_path)?;
            for row in 0..50 {
                let key = format!("writer-{writer_id}");
                let value = format!("row-{row}");
                writer.execute(
                    "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)",
                    &[P::from(&plan_id), P::from(&key), P::from(&value)],
                )?;
            }
            Ok(())
        })
    });

    for handle in handles {
        handle
            .join()
            .expect("SQLite writer thread panicked")
            .expect("SQLite writer failed; SQLITE_BUSY indicates missing retry/backoff policy");
    }

    let db = Db::open(&db_path).unwrap();
    assert_eq!(
        db.query_one("SELECT COUNT(*) FROM tags", &[], |row| row.get_i64(0))
            .unwrap(),
        100,
        "serialized SQLite writers did not insert every tag"
    );
}

fn insert_plan(db: &Db, label: &str) -> String {
    let plan_id = Uuid::new_v4().to_string();
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
            P::from("{\"low\":1}"),
            P::from(label),
            P::from("[\"concurrency\"]"),
        ],
    )
    .unwrap_or_else(|err| panic!("failed to insert plan {label}: {err:#}"));
    plan_id
}

fn tag_count(db: &Db, plan_id: &str, key: &str, value: &str) -> anyhow::Result<i64> {
    db.query_one(
        "SELECT COUNT(*) FROM tags WHERE plan_id = $1 AND key = $2 AND value = $3",
        &[P::from(plan_id), P::from(key), P::from(value)],
        |row| row.get_i64(0),
    )
}
