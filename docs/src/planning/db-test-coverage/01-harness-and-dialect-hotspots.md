# Phase 01 — Harness refactor + dialect hotspots

> **Recommended Codex model: GPT 5.5 medium**
>
> Refactor the two duplicated round-trip tests into a table-driven
> harness, then layer the highest-value parity tests (A1, A4, A8) on top.
> Moderate complexity, leaf-to-sub-agent role: the harness shape decides
> what every later phase looks like. `low` would likely produce a harness
> that doesn't compose cleanly with the Postgres `#[cfg]` gating; `high`
> isn't justified — the design space is small and well-defined by the
> existing `PgTestSchema` helper.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`.

## Goal

Single harness in `tests/calibration_db.rs` that yields a `Vec<(&'static
str, BackendFixture)>`, with one entry for SQLite (always) and one for
Postgres (when `SKILLNET_TEST_PG_URL` is set and the `postgres` feature
is on). All future parity tests iterate this vec. The three dialect-hot
parity tests land here so the harness is exercised by more than just the
round-trip.

## Why

The current file duplicates the round-trip across an `#[cfg]` divider; a
third parity test would copy-paste a third time. The table-driven harness
costs ~40 lines once and pays back per test thereafter. Doing it together
with three real users (A1, A4, A8) proves the shape before phases 02–05
build on it.

## Out of scope

- A2/A3/A5/A6/A7/A9/A10/A11/A12 (phase 02).
- Backend-specific tests (phase 03).
- Error-path tests (phase 04).
- Splitting the test file into multiple files (defer until size warrants).

## Plan

1. **Extract `BackendFixture` and `backends()`** at the top of
   `tests/calibration_db.rs`:
   ```rust
   enum BackendFixture {
       Sqlite { _temp: tempfile::TempDir, db: Db },
       #[cfg(feature = "postgres")]
       Postgres { _schema: PgTestSchema, db: Db },
   }

   impl BackendFixture {
       fn db_mut(&mut self) -> &mut Db { ... }
   }

   fn backends() -> Vec<(&'static str, BackendFixture)> { ... }
   ```
   `backends()` always returns the SQLite fixture; appends Postgres when
   the feature is on **and** `SKILLNET_TEST_PG_URL` is set. When the
   feature is on but the env var is unset, log a one-line skip notice
   *once* (use `OnceLock` to dedupe across tests).

2. **Rewrite the existing round-trip** to use the harness:
   ```rust
   #[test]
   fn opens_migrates_round_trips_and_enforces_cascades() {
       for (name, mut fx) in backends() {
           let plan_id = assert_round_trip(fx.db_mut());
           // backend-agnostic post-conditions
           assert_cascade_delete_tags_only(fx.db_mut(), &plan_id, name);
       }
   }
   ```
   Keep `assert_round_trip` and `assert_cascade_delete*` as free
   functions so phases 02–04 can reuse them. The SQLite-specific
   `journal_mode` assertion moves to phase 03 (B-series).

3. **A1 — `execute_returning_id_assigns_distinct_increasing_ids`.**
   For each backend, insert three `triggers` rows back-to-back via
   `execute_returning_id`, assert ids are pairwise distinct, monotonically
   increasing, and `> 0`. (Postgres `BIGSERIAL` and SQLite
   `last_insert_rowid` both satisfy this; the test pins the contract.)

4. **A4 — `transaction_commits_on_ok_rolls_back_on_err`.** Two subtests
   per backend:
   - `transaction(|tx| Ok(insert))` → row visible after the closure.
   - `transaction(|tx| { insert; Err(anyhow!("rollback")) })` → row
     absent, and the `Err` is propagated with the original message
     reachable through `anyhow::Error::to_string()`.
   Insert a `tags` row (cheap, has a string key/value) for both.

5. **A8 — `cascade_delete_purges_all_child_tables`.** Generalise the
   current `assert_cascade_delete`: insert one row each in `triggers`,
   `phases`, `verifications`, and `tags`; delete the parent plan; assert
   all four counts go to zero. Runs on both backends.

6. **Local validation.** Inside the dev shell:
   - `cargo test --all-targets`
   - `cargo test --all-targets --features postgres` (with and without
     `SKILLNET_TEST_PG_URL`; both must pass — without, the Postgres entry
     is just absent from the harness vec).
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo clippy --all-targets --features postgres -- -D warnings`

## Acceptance criteria

- [ ] `tests/calibration_db.rs` defines `BackendFixture`, `backends()`,
      and uses both in the existing round-trip test (no `#[cfg]`
      branching at the test level).
- [ ] A1, A4, A8 are implemented as iteration over `backends()`; each
      test panics with the backend `name` in the message on failure.
- [ ] The Postgres branch of `backends()` is skipped (not failed and not
      silently absent) when the feature is on but `SKILLNET_TEST_PG_URL`
      is unset; a single skip notice is logged.
- [ ] `cargo test --all-targets` and `cargo test --all-targets --features
      postgres` both pass.
- [ ] `cargo clippy` clean on both feature configurations.
- [ ] The previously inline SQLite `journal_mode` assertion has been
      moved or scheduled for phase 03 — it must not be silently dropped.

## Files likely touched

- `tests/calibration_db.rs`

## Pitfalls

- **Fixture drop order.** `BackendFixture` owns `Db` and the
  schema/tempdir. Drop sequence matters for Postgres: drop `Db` (closes
  the connection) before dropping `PgTestSchema` (which `DROP SCHEMA …
  CASCADE` over a fresh admin connection). Rust drops struct fields in
  declaration order — keep `db` before `_schema`.
- **`tempfile::TempDir` cross-call lifetime.** Don't return the fixture
  by value and immediately call `.db_mut()` on it through `.0.1` access
  — bind the fixture to a local first or the tempdir drops mid-test.
- **Postgres skip notice duplication.** Without dedup the message
  appears once per test. Use `OnceLock<()>` or a `static AtomicBool`.
- **`assert_round_trip` borrowing `Db` mutably.** It already takes `&mut
  Db`; keep that — `execute` is now `&mut self`.
- **A4 rollback assertion must read through a *fresh* query**, not the
  `Tx` (the `Tx` is gone by then). Re-`query_one` via the outer `Db`.

## Reference

- Current test file: `tests/calibration_db.rs`.
- `Db` public surface: `src/calibration/db.rs:50-260`.
- Existing Postgres helper: `PgTestSchema` (lines 247-289).
