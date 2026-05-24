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
