# Calibration

`skillnet calibration` stores and analyzes calibration data for the `multi-phase-plan` workflow.

Available subcommands in the current tree:

- `record`
- `verify`
- `analyze`
- `propose`
- `proposals`
- `decide`
- `export-changelog`

Examples:

```sh
skillnet calibration analyze --format table
skillnet calibration proposals --pending
```

The SQLite schema is sourced from `data/multi-phase-plan/schema/001-initial.sql` and embedded into the binary at compile time.
