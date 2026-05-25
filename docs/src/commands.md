# Command Surface

Top-level commands:

- `status`: show canonical store, view drift, destination, and catalog health.
- `doctor`: check configured views for invariant violations.
- `completions`: generate shell completion scripts.
- `view`: materialise and inspect global view symlinks.
- `skill`: list, inspect, and edit canonical skill directories.
- `scope`: inspect configured canonical scopes.
- `project`: manage configured project roots and materialise project views.
- `catalog`: generate and validate skill catalog metadata.
- `calibration`: record, inspect, analyze, and tune `multi-phase-plan`
  calibration data.

Examples:

```sh
skillnet view sync --all
skillnet project sync --all
skillnet doctor
skillnet view status --all
skillnet project status --all
skillnet skill show global/rust-project-flake
skillnet project list
skillnet catalog lint
skillnet calibration heuristics list
skillnet calibration walkthrough --dry-run
```

## View And Project Commands

`skillnet view sync --all` materialises configured global views from the
canonical global store.

`skillnet project sync --all` materialises configured project views and the
project aggregator symlinks under `mirror_root/projects/`.

Use `view status|diff` and `project status|diff` to inspect derived view drift
without mutating files. `skillnet sync` was removed in `0.5.0`; each scope now
has one canonical store and every other location is a generated view.

## Status JSON Schema

`skillnet status --format json --scope <SCOPE>` emits a pretty JSON array:

```json
[
  {
    "scope": "global",
    "kind": "global",
    "canonical_path": "/home/alice/skills/global",
    "skill_count": 42,
    "drift_entries": 0
  }
]
```

## Doctor

`skillnet doctor` checks resolved global view configuration. P9 expands the
doctor invariant set for Option B; in `0.5.0` it still focuses on view paths
that cannot be materialised because a parent directory is missing or the path
already exists as a non-directory.

## Config File Location

`skillnet --config <path>` is the highest-precedence source. When the flag is
omitted, the binary reads:

| Rank | Source                          | Resolves to                                                                      |
| ---- | ------------------------------- | -------------------------------------------------------------------------------- |
| 1    | `--config <path>`               | absolute or cwd-relative                                                         |
| 2    | `SKILLNET_CONFIG`               | absolute or cwd-relative                                                         |
| 3    | XDG config file, when present   | `$XDG_CONFIG_HOME/skillnet/skillnet.toml`, or `~/.config/skillnet/skillnet.toml` |
| 4    | legacy cwd config, when present | `./skillnet.toml`                                                                |
| 5    | missing-config error path       | XDG path                                                                         |

The same file precedence applies to `--catalog-config` /
`SKILLNET_CATALOG_CONFIG`, using `skillnet.catalog.toml` as the file name.

The destination root precedence is `--mirror-root`, then
`SKILLNET_MIRROR_ROOT`, then `skills_root` in `skillnet.toml`, then the legacy
`mirror_root` key, then `.`. When the destination is a Git repository,
`skillnet status` reports branch, origin, and dirty state. Commands that write
to the mirror destination refuse to run when that repository is dirty unless
`--allow-dirty-destination` is passed.

The removed `sync pull` auto-commit flow has no replacement in `0.5.0`; commit
canonical store changes with Git directly.

## Calibration Database

Calibration commands use Postgres by default. The backend can be selected in
`skillnet.toml`:

```toml
[database]
url = "postgres://user@host/skillnet"
```

SQLite remains available by selecting it explicitly:

```toml
[database]
backend = "sqlite"
path = "/abs/path/calibration.sqlite"
```

`path` is used only for SQLite. `url` selects the Postgres backend.

Database target precedence, highest first:

| Rank | Source                                       | Result                                                                                              |
| ---- | -------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| 1    | `--database-url <URL>`                       | Postgres                                                                                            |
| 2    | `SKILLNET_DATABASE_URL` or `SKILLNET_DB_URL` | Postgres                                                                                            |
| 3    | `[database].url`                             | Postgres                                                                                            |
| 4    | `SKILLNET_DATA_DIR` or `skillnet_DATA_DIR`   | SQLite at `<dir>/multi-phase-plan/calibration.sqlite`                                               |
| 5    | `[database].path` with `backend = "sqlite"`  | SQLite at the configured path                                                                       |
| 6    | no override                                  | Postgres, requiring `database.url`, `SKILLNET_DATABASE_URL`, `SKILLNET_DB_URL`, or `--database-url` |

When a database URL and `SKILLNET_DATA_DIR`/`skillnet_DATA_DIR` are both set,
the URL wins and `skillnet` prints a warning.
