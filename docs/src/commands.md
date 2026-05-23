# Command Surface

Top-level commands:

- `status`: show scope divergence and catalog health.
- `completions`: generate shell completion scripts.
- `sync`: pull from live sources, push to live targets, and inspect divergence.
- `skill`: list, inspect, and edit mirrored skill directories.
- `scope`: inspect configured mirror scopes and their live sources.
- `project`: manage configured project roots.
- `catalog`: generate and validate skill catalog metadata.
- `calibration`: record and analyze `multi-phase-plan` calibration data.

Examples:

```sh
skillnet sync pull --scope global
skillnet sync status --scope global
skillnet skill show global/rust-project-flake
skillnet project list
skillnet catalog lint
```

## Calibration Database

Calibration commands use SQLite by default at the existing runtime data
location. The backend can also be selected in `skillnet.toml`:

```toml
[database]
backend = "sqlite" # or "postgres"
path = "/abs/path/calibration.sqlite"
url = "postgres://user@host/skillnet"
```

`path` is used only for SQLite. `url` selects the Postgres backend. If a
Postgres target is selected with a binary built without the `postgres`
feature, `skillnet` exits with a clear error instead of opening a database.

Database target precedence, highest first:

| Rank | Source | Result |
| --- | --- | --- |
| 1 | `--database-url <URL>` | Postgres |
| 2 | `SKILLNET_DATABASE_URL` or `SKILLNET_DB_URL` | Postgres |
| 3 | `[database].url` | Postgres |
| 4 | `SKILLNET_DATA_DIR` or `skillnet_DATA_DIR` | SQLite at `<dir>/multi-phase-plan/calibration.sqlite` |
| 5 | `[database].path` | SQLite at the configured path |
| 6 | no override | SQLite at the default runtime data path |

When a database URL and `SKILLNET_DATA_DIR`/`skillnet_DATA_DIR` are both
set, the URL wins and `skillnet` prints a warning.
