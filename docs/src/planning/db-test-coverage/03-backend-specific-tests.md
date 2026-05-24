# Phase 03 — Backend-specific tests (B-series)

> **Recommended Codex model: GPT 5.5 medium**
>
> Five tests that only run on one backend each. The Postgres ones (B3,
> B4, B5) need real Postgres-feature knowledge (BIGSERIAL sequence
> manipulation, `search_path` introspection, JSONB error surfacing).
> `low` would likely write B3 with a hard-coded sequence name and break
> the moment a column is renamed; `medium` understands schema-qualified
> sequence lookup.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`, with phase 01 landed.

## Goal

Five `#[test]` functions, each `#[cfg]`-gated to its backend, covering
behaviours that are not parity-testable because they don't exist on the
other backend.

## Why

The parity suite proves the abstraction. The backend-specific suite
proves the *floor* each backend stands on: SQLite WAL + foreign_keys
pragmas, Postgres BIGSERIAL width and schema isolation. A regression in
either floor breaks the parity tests with confusing errors.

## Out of scope

- Anything that can be parity-tested (belongs in phase 02).
- Cross-backend behaviours (phase 02 already covers them).
- Tests that require multi-process or multi-connection (phase 05).

## Plan

Add a new file `tests/calibration_db_backend.rs` that `#[path]`-includes
`../src/calibration/db.rs` and contains:

1. **B1 — `sqlite_enforces_wal_journal_mode`.** SQLite-only. Open a DB,
   call `db.sqlite_pragma_string("journal_mode")`, assert `"wal"`. This
   is the inline assertion from the original round-trip test, lifted
   into its own test.

2. **B2 — `sqlite_enforces_foreign_keys_pragma`.** SQLite-only. Open a
   DB and `query_one("PRAGMA foreign_keys", &[], |r| r.get_i64(0))` →
   `1`. Catches the easy mistake of disabling FKs in a future refactor
   (which would silently invalidate the parity A8 cascade test).

3. **B3 — `postgres_returning_id_handles_bigserial_above_i32_max`.**
   Postgres-only. Discover the `triggers_id_seq` (or whatever
   `pg_get_serial_sequence('triggers', 'id')` returns) and call
   `SELECT setval($1, $2)` with `3_000_000_000_i64`. Insert one
   `triggers` row via `execute_returning_id` and assert the returned id
   is exactly `3_000_000_001` (or `>= 3_000_000_001` depending on the
   sequence's `is_called` flag — read the docs and pick the precise
   value).

4. **B4 — `postgres_migration_isolated_to_search_path_schema`.**
   Postgres-only. The `PgTestSchema` helper isolates via `search_path`.
   Open `Db::open_postgres(schema_url)`, then connect a *second*
   admin-level client to the same database and run:
   ```sql
   SELECT EXISTS (
       SELECT 1 FROM information_schema.tables
       WHERE table_schema = $1 AND table_name = 'schema_versions'
   )
   ```
   with the test schema name. Assert `true`. Also query with
   `table_schema = 'public'` and assert `false` (the migration must
   *not* have leaked into `public`).

5. **B5 — `postgres_jsonb_rejects_invalid_json`.** Postgres-only.
   Attempt to insert a `plans` row whose `routing_dist` value is the
   literal string `"not-json"` (bound as text). The driver must return
   an `Err` whose `Display` mentions invalid JSON or the column name —
   not a panic, not a silent success. Use `assert!(err.to_string()
   .contains("json") || err.to_string().contains("routing_dist"))`
   to keep the assertion robust across `postgres` crate versions.

## Acceptance criteria

- [ ] `tests/calibration_db_backend.rs` exists and `#[path]`-includes
      `db.rs` the same way `calibration_db.rs` does.
- [ ] B1 and B2 are gated to default features (SQLite), run
      unconditionally.
- [ ] B3, B4, B5 are gated `#[cfg(feature = "postgres")]` and skip
      cleanly when `SKILLNET_TEST_PG_URL` is unset (same skip-notice
      pattern as phase 01).
- [ ] B3 demonstrates id round-trip above `i32::MAX` without truncation.
- [ ] B4 proves migration tables live in the per-test schema, not
      `public`.
- [ ] `cargo test --all-targets` and `cargo test --all-targets
      --features postgres` pass.
- [ ] `cargo clippy` clean on both feature configurations.

## Files likely touched

- `tests/calibration_db_backend.rs` (new)

## Pitfalls

- **B3 sequence name.** Don't hard-code `triggers_id_seq`. Use
  `pg_get_serial_sequence('<schema>.triggers', 'id')` so the test
  works when the schema is renamed. The query returns a fully-qualified
  sequence name suitable for `setval`.
- **B3 `is_called` semantics.** `setval(seq, N)` with no third argument
  sets `last_value = N` and `is_called = true`, so the *next* `nextval`
  returns `N + 1`. `setval(seq, N, false)` returns `N` on next call.
  Pick one form and write the assertion to match.
- **B4 admin-level connection.** Use the base URL (without the
  `search_path` override) so the second client sees both schemas. Don't
  copy the `Db`'s connection — `Db` doesn't expose the raw client.
- **B5 error-message stability.** `postgres` crate error messages
  change between versions. Anchor on column name or the substring
  "json" rather than the full message.
- **Test isolation across B3/B5.** Both modify state in the per-test
  schema. The harness already creates a fresh schema per test; ensure
  these tests acquire a fresh fixture (don't share with parity tests).

## Reference

- `pg_get_serial_sequence` docs:
  <https://www.postgresql.org/docs/current/functions-info.html>.
- `setval` docs:
  <https://www.postgresql.org/docs/current/functions-sequence.html>.
- `PgTestSchema` helper at `tests/calibration_db.rs:247-289`.
