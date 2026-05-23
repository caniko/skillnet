# Phase 02 — Postgres backend

> **Recommended Codex model: GPT 5.5 high**
>
> Adds a second backend behind the abstraction from phase 01: new
> dependency, a parallel migration set translated to Postgres dialect,
> connection lifecycle, error mapping, and `RETURNING`-based last-insert.
> Several "looks portable" things in the existing SQLite schema are not
> (INTEGER booleans, INTEGER unix timestamps, TEXT-as-JSON vs JSONB,
> AUTOINCREMENT). Quality of dialect translation determines whether
> downstream phases discover landmines. High-tier is warranted.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`, with phase 01 landed.

## Goal

Implement `Backend::Postgres` so `Db::open_postgres(url)` produces a working
calibration store with the same logical schema as the SQLite backend, behind
a `postgres` Cargo feature (default-off). All existing tests still pass with
the feature disabled; new feature-gated tests cover the Postgres path.

## Why

The HM module update in phase 04 needs an actual second backend to switch
to. Keeping it behind a Cargo feature lets crates.io users who only want
SQLite avoid pulling the Postgres driver and its native deps.

## Out of scope

- Connection pooling beyond what the chosen driver gives out of the box.
- Async/await migration of the rest of the crate.
- Automatic data migration from SQLite to Postgres.

## Plan

1. **Driver choice.** Use the synchronous `postgres` crate (sibling of
   `tokio-postgres`) so the rest of the codebase stays sync. Add it as an
   optional dependency:
   ```toml
   [dependencies]
   postgres = { version = "0.19", optional = true }
   
   [features]
   default = []
   postgres = ["dep:postgres"]
   ```
   Do not enable `postgres` by default — the crates.io footprint must stay
   the same for SQLite-only users.
2. **Schema translation.** Add `data/multi-phase-plan/schema-pg/001-initial.sql`
   mirroring the SQLite schema with Postgres types:
   - `INTEGER PRIMARY KEY AUTOINCREMENT` → `BIGSERIAL PRIMARY KEY` (or
     `GENERATED ALWAYS AS IDENTITY`).
   - `INTEGER` unix timestamps → `BIGINT`.
   - `INTEGER` booleans → `BOOLEAN` (and update inserts in the Postgres
     backend to bind `bool`).
   - `TEXT` JSON blobs → `JSONB` where the column stores valid JSON
     (`routing_dist`, `capture_reasons`, `phase_outcomes`,
     `emergency_changes`, `files`, `supporting_plan_ids`, `filter_tags`);
     plain `TEXT` for the surprises free-form field.
   - Indexes and `REFERENCES … ON DELETE CASCADE` carry over verbatim.
   Also add the `schema_versions` table with `applied_at BIGINT NOT NULL`.
3. **Backend module.** New file `src/calibration/db_postgres.rs` gated by
   `#[cfg(feature = "postgres")]`. Implement the same surface from phase
   01: execute / query_one / query_all / transaction / last_insert_id.
   - Map placeholder convention to native `$N` (no rewrite needed if phase
     01 chose `$N` as canonical).
   - `last_insert_id` uses `RETURNING id` returned by the helper; expose
     it as `execute_returning_id(sql, params) -> i64`.
4. **`Db::open` plumbing.** Add `Db::open_postgres(url: &str)` behind the
   feature. The existing `Db::open(path)` stays as the SQLite entry point.
   `default_path()` remains SQLite-specific; Postgres has no path.
5. **Migration runner.** The runner from phase 01 must be backend-agnostic.
   Load `schema-pg/*.sql` when the active backend is Postgres, `schema/*.sql`
   otherwise. Keep the `MIGRATIONS` array indirection so the file set is
   compiled in via `include_str!`.
6. **`include` in `Cargo.toml`.** Add `data/multi-phase-plan/schema-pg/*.sql`
   to the published `include` list so the feature works for `cargo install`
   users who enable `--features postgres`.
7. **Local validation.** `cargo build --features postgres`,
   `cargo clippy --all-targets --features postgres -- -D warnings`,
   `cargo test --all-targets` (no feature) and a Postgres-feature test
   added in phase 05 (this phase only verifies it compiles and a smoke
   test connects when `SKILLNET_TEST_PG_URL` is set).

## Acceptance criteria

- [ ] `Cargo.toml` declares a `postgres` feature gating an optional
      `postgres` crate dependency; default features unchanged.
- [ ] `data/multi-phase-plan/schema-pg/001-initial.sql` exists with the
      translated schema and is included in the published crate.
- [ ] `Db::open_postgres(url)` compiles under `--features postgres` and
      applies the schema on a fresh database (proven by a `#[cfg(feature =
      "postgres")]` smoke test that skips when `SKILLNET_TEST_PG_URL` is
      unset).
- [ ] `cargo build` (no features) and `cargo build --features postgres`
      both succeed.
- [ ] `cargo clippy --all-targets --features postgres -- -D warnings` clean.
- [ ] No production code path requires the `postgres` feature to compile
      the SQLite backend.

## Files likely touched

- `Cargo.toml` (feature + optional dep + include list).
- `src/calibration/db.rs` (backend dispatch, `open_postgres`).
- `src/calibration/db_postgres.rs` (new, feature-gated).
- `data/multi-phase-plan/schema-pg/001-initial.sql` (new).
- `tests/calibration_db.rs` (smoke test gated on feature + env var).

## Pitfalls

- **Type coercion at insert sites.** SQLite is permissive; Postgres is
  strict. A `bool` bound to a `BOOLEAN` column must be a `bool`, not an
  `i32`. Audit every `INSERT` after schema translation.
- **JSON binding.** `JSONB` accepts a string with an explicit cast
  (`$1::jsonb`) or via the driver's `serde_json::Value` support. Pick one
  convention and apply it consistently.
- **AUTOINCREMENT vs BIGSERIAL ranges.** Tests that hard-code small ids
  may break under `BIGSERIAL`; don't assert specific generated ids.
- **Connection lifetime.** `postgres::Client` is `!Sync`. Wrap it the same
  way the SQLite `Connection` is held; do not introduce `Arc<Mutex<…>>`
  unless callers need it (they don't today).

## Reference

- Phase 01 abstraction: `01-db-backend-abstraction.md`.
- SQLite schema source of truth: `data/multi-phase-plan/schema/001-initial.sql`.
- `postgres` crate: <https://docs.rs/postgres>.
