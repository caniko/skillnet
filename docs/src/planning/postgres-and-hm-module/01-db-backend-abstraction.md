# Phase 01 — DB backend abstraction

> **Recommended Codex model: GPT 5.5 high**
>
> Refactor with many call sites (every file under `src/calibration/` touches
> `db.connection()` or `connection_mut()` and several construct
> `rusqlite::Transaction` directly). The redesign must keep the SQLite path
> behaviour byte-identical while shaping the API so a Postgres backend can
> slot in without rewriting record/propose/decide/analyze a second time. A
> medium-tier model will likely produce a leaky abstraction that forces
> rework in phase 02; high-tier is justified once, not per phase.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`. No new dependencies in this
phase — Postgres crates land in phase 02.

## Goal

Introduce a backend-agnostic shape for `calibration::db` such that:

- All current SQLite behaviour is preserved (WAL, foreign_keys, bundled
  migrations, `Db::default_path()` semantics).
- Callers in `record.rs`, `propose.rs`, `decide.rs`, `analyze.rs`,
  `changelog.rs` no longer reach into `rusqlite::Connection` directly for the
  shapes they need; instead they call methods on `Db` (or a small set of
  backend-neutral helpers) that a future Postgres backend can implement.

## Why

The current API exposes `&Connection` / `&mut Connection`, so every caller
hand-builds SQL with `?N` placeholders and `rusqlite` types. Postgres uses
`$N` placeholders and different type bindings; bolting it on without
abstraction means duplicating each SQL site or scattering `cfg` switches.
Doing the refactor once, with SQLite still the only backend, keeps this
phase reviewable in isolation.

## Out of scope

- Adding Postgres dependencies or schemas (phase 02).
- Adding config/CLI knobs for backend choice (phase 03).
- Touching the HM module (phase 04).

## Plan

1. Read every call site of `db.connection()` / `connection_mut()` in
   `src/calibration/{record,propose,decide,analyze,changelog}.rs`. Catalogue
   the operations actually performed: `execute`, `query_row`,
   `prepare`+`query_map`, transactions, `last_insert_rowid`.
2. Pick an abstraction shape. Recommended: keep `Db` as the public type but
   make its backend an internal enum (`Backend::Sqlite(rusqlite::Connection)`
   today, `Backend::Postgres(...)` in phase 02). Expose backend-neutral
   methods such as:
   - `Db::execute(&mut self, sql: &str, params: &[&dyn ToSqlParam])` (or a
     small custom param type that maps to both drivers).
   - `Db::query_one<T>(...)`, `Db::query_all<T>(...)` returning owned rows
     via a `Row` trait.
   - `Db::transaction(|tx| { ... })` taking a closure that receives a
     `&mut Tx` with the same surface.
   - `Db::last_insert_id(&self) -> i64` (Postgres will use `RETURNING id` in
     phase 02; keep this hook).
3. Translate each call site. Where SQL is dialect-portable today (most of
   the schema), leave the SQL strings as-is but route them through the new
   API. Where placeholders differ (`?1` vs `$1`), centralise placeholder
   rewriting in the backend so callers can keep writing one dialect — pick
   `$N` as the canonical form in call sites and let the SQLite backend
   rewrite to `?N` at execute time (simpler than the reverse).
4. Update `tests/calibration_db.rs` so it exercises the new API (not raw
   `connection()` calls). Keep the SQLite-specific assertions (e.g. WAL
   pragma check) behind an `#[cfg(...)]` or a `Db::sqlite_pragma(...)`
   inspection helper used only by tests.
5. `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test --all-targets`. All must pass.

## Acceptance criteria

- [ ] No file under `src/calibration/` except `db.rs` imports `rusqlite`
      directly (verify with `rg "rusqlite" src/calibration/`).
- [ ] `Db` exposes execute / query / transaction / last-insert helpers used
      by every former `connection()` call site.
- [ ] Placeholder convention in call-site SQL is uniform (chosen dialect
      documented in a short comment at the top of `db.rs`).
- [ ] `cargo test --all-targets` passes with the same set of tests as
      before (minus any SQLite-internals tests that were intentionally
      gated, which must still run on the default feature set).
- [ ] `cargo clippy --all-targets -- -D warnings` clean.

## Files likely touched

- `src/calibration/db.rs` (major rewrite).
- `src/calibration/{record,propose,decide,analyze,changelog}.rs` (call-site
  updates).
- `tests/calibration_db.rs` (API updates).

## Pitfalls

- **Leaky `rusqlite::Row` in return types.** Don't return `Row<'_>`; copy
  data out inside the backend and yield owned values. Phase 02 cannot
  satisfy a `rusqlite::Row` return.
- **`last_insert_rowid` semantics.** SQLite returns it from the connection;
  Postgres needs `RETURNING id`. Have callers state intent ("I want the
  inserted id") via the API, not by reading a side-channel after the fact.
- **Transactions across helper calls.** If a caller currently holds a
  `rusqlite::Transaction` and calls helpers on it, the new API must let the
  same helpers run inside or outside a transaction. Keep `Tx` and `Db`
  exposing the same method surface.

## Reference

- Current schema: `data/multi-phase-plan/schema/001-initial.sql`.
- Current backend: `src/calibration/db.rs:1-175`.
