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

When a database URL and `SKILLNET_DATA_DIR`/`skillnet_DATA_DIR` are both
set, the URL wins and `skillnet` prints a warning.
