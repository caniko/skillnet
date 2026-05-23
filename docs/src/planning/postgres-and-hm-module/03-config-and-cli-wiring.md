# Phase 03 — Config and CLI wiring

> **Recommended Codex model: GPT 5.5 medium**
>
> Routine surface work: a new `[database]` section in `skillnet.toml`, an
> env var, a global CLI flag, and a small `select_backend()` helper that
> decides between SQLite path and Postgres URL. The design is constrained
> by phases 01–02; no novel architecture. Medium handles this without
> overspending.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`, with phases 01 and 02
landed. Can run concurrently with phase 02 once 01 lands; if it does, be
prepared to merge the `Db::open_postgres` call site at integration time.

## Goal

Let users select the calibration backend declaratively in `skillnet.toml`,
imperatively via env vars, or via a CLI flag, with a documented precedence
order. The `calibration::Db::default_path()` call site in CLI command
handlers must route through this selector instead of hard-coding SQLite.

## Why

The HM module (phase 04) needs a well-defined surface to set. CLI users
need a way to point at a non-default Postgres without editing config.

## Out of scope

- Migrating data between backends.
- A `skillnet db` admin subcommand (separate effort if requested).

## Plan

1. **Config schema.** Extend `Config` in `src/config.rs` with a
   `database` field:
   ```toml
   [database]
   backend = "sqlite"          # or "postgres"
   path    = "/abs/path/calibration.sqlite"   # sqlite only
   url     = "postgres://user@host/skillnet"  # postgres only
   ```
   Both `path` and `url` are optional; defaults match today's behaviour
   when `backend = "sqlite"` and `path` is omitted.
2. **Env var precedence.**
   - `SKILLNET_DATABASE_URL` (or `SKILLNET_DB_URL`) → Postgres URL, wins
     over config.
   - `SKILLNET_DATA_DIR` / `skillnet_DATA_DIR` → SQLite parent directory
     (existing behaviour, unchanged).
   - When both a URL and a data dir are set, the URL wins and a warning is
     logged.
3. **CLI flag.** Add a global `--database-url <URL>` option to the root
   `clap` parser; it overrides both env and config when present. No
   matching flag for SQLite path — `SKILLNET_DATA_DIR` already covers it.
4. **Selector helper.** Add `Config::resolve_db(&self) -> DbTarget` where
   `DbTarget` is:
   ```rust
   pub enum DbTarget {
       Sqlite(PathBuf),
       Postgres(String),
   }
   ```
   With precedence: CLI flag > env URL > config URL > env data dir >
   config path > `Db::default_path()` (SQLite).
5. **Db open site.** Replace each `Db::open(Db::default_path())` in
   command handlers with `Db::open_target(cfg.resolve_db_with_overrides(...))`,
   which dispatches to `open` or `open_postgres`. Surface a clear error
   when `DbTarget::Postgres` is requested but the binary was built without
   the `postgres` feature.
6. **Config doc.** Update `docs/src/commands.md` (or the nearest existing
   config section) with the new `[database]` block and precedence table.
7. **Validation.** `cargo test --all-targets`; add a unit test for
   `Config::resolve_db` covering each precedence rung.

## Acceptance criteria

- [ ] `skillnet.toml` accepts a `[database]` table with `backend`, `path`,
      `url` and rejects unknown keys (`#[serde(deny_unknown_fields)]`).
- [ ] `--database-url`, `SKILLNET_DATABASE_URL`, and the config `url`
      field all reach the Postgres backend; precedence matches the order
      documented in this phase.
- [ ] When the binary is built without `--features postgres`, requesting
      a Postgres target produces a clear, actionable error (not a panic).
- [ ] `cargo test --all-targets` passes; the precedence helper has unit
      coverage for all six rungs.
- [ ] User-facing docs name the new options and precedence.

## Files likely touched

- `src/config.rs`
- `src/cli/mod.rs` (or wherever the root `clap` parser lives)
- `src/commands/*.rs` (every site that opens the calibration DB)
- `docs/src/commands.md` (or the closest config doc)

## Pitfalls

- **Silent precedence inversion.** If both `SKILLNET_DATA_DIR` and
  `SKILLNET_DATABASE_URL` are set, users expect URL to win. Test it.
- **Feature-gated error message.** A "postgres feature not enabled" error
  shown at config-resolution time is fine; one shown deep inside `Db` is
  not. Surface it at the selector.
- **`deny_unknown_fields` on existing config.** If the current `Config`
  uses `serde(deny_unknown_fields)` at the top level, adding `[database]`
  is fine; if it doesn't, don't tighten it as a side effect.

## Reference

- Phase 02 backend entry points: `Db::open` and `Db::open_postgres`.
