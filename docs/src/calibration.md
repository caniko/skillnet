# Calibration

`skillnet calibration` stores and analyzes `multi-phase-plan` calibration data.

Core subcommands:

- `record`
- `verify`
- `init`
- `eval`
- `meta-heuristics`
- `shape-hash`
- `heuristics list`
- `heuristics show`
- `walkthrough`
- `tag`
- `untag`
- `show`
- `query`
- `migrate`
- `vacuum`
- `export`
- `analyze`
- `propose`
- `proposals`
- `decide`
- `export-changelog`

Examples:

```sh
skillnet calibration analyze --format table
skillnet calibration heuristics list
skillnet calibration walkthrough --dry-run
skillnet calibration proposals --pending
```

Since `0.4.0`, `analyze --format json` emits a SemVer-stable schema with
`schema_version: 1`, active threshold provenance, and proposal rows suitable
for automation. See [Calibration JSON schema](calibration/json-schema.md).

Verifier surprise annotations supply the false-positive and false-negative
counts used by the analyzer. See
[Verifier surprises convention](calibration/surprises.md).

## Heuristic Thresholds

Catalog heuristics ship with code defaults; runtime overrides live in
`heuristic_thresholds` and flow through migration and proposal decisions:

```sh
skillnet calibration migrate
skillnet calibration decide <proposal-id> accept --rationale "..."
```

`decide accept` persists the override, and subsequent `analyze`, `eval`, and
`heuristics` runs report whether each threshold came from the default catalog
or an override.

## Home Manager Consumers

Calibration commands use the database configured by `programs.skillnet.database`
and `programs.skillnet.settings`.

After updating the flake input, run:

```sh
home-manager switch
skillnet --version
skillnet calibration migrate
skillnet calibration heuristics list
skillnet calibration walkthrough --dry-run
```

Postgres users keep providing `programs.skillnet.database.url` or `urlFile`;
SQLite users keep `backend = "sqlite"` and an absolute `path` when overriding
the default data directory.

Schema files from `data/multi-phase-plan/schema/*.sql` (SQLite) and
`data/multi-phase-plan/schema-pg/*.sql` (Postgres) are embedded at compile time.

## Testing the Calibration DB

`tests/calibration_db.rs` runs a table-driven parity suite. `BackendFixture`
with `fn backends()` always includes SQLite and adds Postgres when the
`postgres` feature is enabled **and** `SKILLNET_TEST_PG_URL` is set. Each parity
test loops the vector and panics with the backend name on mismatch; in a build
with the `postgres` feature enabled, `log_postgres_skip_once` logs one skip
notice when `SKILLNET_TEST_PG_URL` is unset.

Round-trip parity checks **semantic** JSON equality via `serde_json::Value`, not
raw bytes, because `routing_dist` is `TEXT` in SQLite and `JSONB` in Postgres
(which normalises whitespace and key order).

Backend-specific tests live in `tests/calibration_db_backend.rs`:

- SQLite asserts `journal_mode = wal` and `PRAGMA foreign_keys = 1` (required
  for cascade-delete parity).
- Postgres asserts `RETURNING id` handles `BIGSERIAL` values above `i32::MAX`,
  migrations target the per-test `search_path` schema rather than `public`, and
  `JSONB` columns reject non-JSON input with a typed error.

Timing-sensitive SQLite WAL concurrency tests in
`tests/calibration_db_concurrency.rs` are `#[ignore]`d by default (covering
multiple readers/writers). Run them with
`cargo test --all-targets -- --ignored` when changing connection lifecycle or
pragma defaults.

Run the Postgres suite locally with:

```sh
SKILLNET_TEST_PG_URL=postgres:///skillnet_test \
  cargo test --all-targets --features postgres
```

Without `SKILLNET_TEST_PG_URL`, Postgres tests skip rather than fail so the
default suite works without a local Postgres instance.
