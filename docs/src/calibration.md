# Calibration

`skillnet calibration` stores and analyzes calibration data for the `multi-phase-plan` workflow.

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

`analyze --format json` emits a SemVer-stable schema starting in `0.4.0`.
The schema includes `schema_version: 1`, active threshold provenance, and
proposal rows suitable for automation. See
[Calibration JSON schema](calibration/json-schema.md).

Verifier surprise annotations feed the false-positive and false-negative
counts used by the analyzer. See
[Verifier surprises convention](calibration/surprises.md).

## Heuristic Thresholds

Catalog heuristics ship with code defaults. Runtime overrides live in the
calibration database's `heuristic_thresholds` table and are managed by the
normal migration and decision flow:

```sh
skillnet calibration migrate
skillnet calibration decide <proposal-id> accept --rationale "..."
```

`decide accept` persists the accepted threshold, and later `analyze`, `eval`,
and `heuristics` commands report whether each threshold came from the default
catalog or an override.

## Home Manager Consumers

Home Manager users do not need new module options for `0.4.0`. The new
calibration helper commands use the existing database configuration exported by
`programs.skillnet.database` and `programs.skillnet.settings`.

After updating the flake input, run:

```sh
home-manager switch
skillnet --version
skillnet calibration migrate
skillnet calibration heuristics list
skillnet calibration walkthrough --dry-run
```

Postgres users should keep providing `programs.skillnet.database.url` or
`urlFile`; SQLite users should keep `backend = "sqlite"` and an absolute
`path` when overriding the default data directory.

The SQLite schema files are sourced from `data/multi-phase-plan/schema/*.sql`;
Postgres schema files are sourced from `data/multi-phase-plan/schema-pg/*.sql`.
Both sets are embedded into the binary at compile time.

## Testing the Calibration DB

The DB abstraction is verified by a table-driven harness in
`tests/calibration_db.rs`. `BackendFixture` plus `fn backends()` yields a
vector with one entry for SQLite (always) and one for Postgres (when the
`postgres` feature is on **and** `SKILLNET_TEST_PG_URL` is set). Each
parity test loops the vector and panics with the backend name on
mismatch; the Postgres branch logs a single skip notice via
`log_postgres_skip_once` when the env var is absent.

Round-trip parity asserts **semantic** JSON equality, not byte equality:
`routing_dist` is `TEXT` in SQLite and `JSONB` in Postgres, and `JSONB`
normalises whitespace and key order. Tests parse both sides through
`serde_json::Value` before comparing.

Backend-specific floors live in `tests/calibration_db_backend.rs`:

- SQLite asserts `journal_mode = wal` and `PRAGMA foreign_keys = 1` (the
  latter is what makes the cascade-delete parity test meaningful).
- Postgres asserts `RETURNING id` survives `BIGSERIAL` values above
  `i32::MAX`, that migrations land in the per-test `search_path` schema
  rather than `public`, and that `JSONB` columns reject non-JSON input
  with a typed error.

Concurrency smoke tests in `tests/calibration_db_concurrency.rs` are
`#[ignore]`d by default — they document the SQLite WAL contract under
multiple readers/writers but are timing-sensitive. Run them explicitly
with `cargo test --all-targets -- --ignored` when changing connection
lifecycle or pragma defaults.

To run the Postgres suite locally:

```sh
SKILLNET_TEST_PG_URL=postgres:///skillnet_test \
  cargo test --all-targets --features postgres
```

Without that env var the Postgres branch is skipped (not failed) so the
default suite stays usable on machines without a local Postgres.
