# Command Surface

Top-level commands:

- `status`: show scope divergence and catalog health.
- `doctor`: check configured scopes for invariant violations.
- `completions`: generate shell completion scripts.
- `sync`: pull from live sources, push to live targets, and inspect divergence.
- `skill`: list, inspect, and edit mirrored skill directories.
- `scope`: inspect configured mirror scopes and their live sources.
- `project`: manage configured project roots.
- `catalog`: generate and validate skill catalog metadata.
- `calibration`: record, inspect, analyze, and tune `multi-phase-plan`
  calibration data.

Examples:

```sh
skillnet sync pull --scope global
skillnet sync roundtrip --all
skillnet sync roundtrip --all --check
skillnet doctor
skillnet sync status --scope global
skillnet skill show global/rust-project-flake
skillnet project list
skillnet catalog lint
skillnet calibration heuristics list
skillnet calibration walkthrough --dry-run
```

## Sync Commands

`skillnet sync roundtrip [--scope <SCOPE>...] [--all] [--check]` pulls the
selected scopes into the mirror and then pushes that mirror state back to each
configured live destination. It is the first-class replacement for
`skillnet sync pull --then-push`; the old flag remains available but prints a
single-line deprecation warning.

By default, sync writes are newer-only at skill-directory granularity. Incoming
skills overwrite existing skills only when the incoming skill tree has a
strictly newer contained file/symlink mtime. Older incoming skills are skipped,
equal-mtime identical skills are no-ops, and equal-mtime differing skills fail.
Destination-only skills are preserved by default. Pass `--allow-older` to allow
older or equal-mtime incoming content to overwrite, and pass `--allow-delete` to
allow pruning skills that are missing from the incoming side.

`--check` stages the pull in a temporary mirror and compares the would-be mirror
contents with each live destination. It does not update the real mirror, does
not write cache metadata, and does not mutate any destination. A clean check
prints each destination as `clean`; drift prints `would change` summaries and
exits non-zero.

Sync exit codes:

| Code | Meaning                                      |
| ---- | -------------------------------------------- |
| 0    | Clean / command completed successfully       |
| 2    | Pull or temporary reconciliation failed      |
| 3    | Push to a live destination failed            |
| 4    | `doctor` parity lint findings               |
| 5    | `sync roundtrip --check` found would-change drift |

## Status JSON Schema

`skillnet status --format json` and
`skillnet sync status --format json --scope <SCOPE>` emit a stable, pretty
printed JSON document:

```json
{
  "schema": "skillnet.status.v1",
  "scopes": [
    {
      "name": "global",
      "state": "clean",
      "diverged_files": 0,
      "last_pulled_at": "2026-05-24T12:34:56Z",
      "cache_state": "fresh"
    }
  ]
}
```

Field contract:

| Field | Meaning |
| ----- | ------- |
| `schema` | Schema identifier. The current value is always `skillnet.status.v1`. |
| `scopes[].name` | Scope name, such as `global` or a configured project name. |
| `scopes[].state` | Either `clean` or `diverged`. |
| `scopes[].diverged_files` | Number of divergent files. This is `0` when `state` is `clean`. |
| `scopes[].last_pulled_at` | RFC 3339 UTC timestamp for the last successful pull, or `null` when the scope has never been pulled. |
| `scopes[].cache_state` | One of `fresh`, `stale`, or `missing`. |

The `v1` schema is additive-stable: new fields may be added within
`skillnet.status.v1`, but removing or renaming an existing field requires a new
schema value such as `skillnet.status.v2`.

## Doctor

`skillnet doctor` checks resolved global scope configuration for sync parity
risks before the first push. It exits 0 when there are no findings and 4 when
it prints any warning.

Findings:

| Kind | Meaning | Recommended fix |
| ---- | ------- | --------------- |
| `asymmetric fan-out` | Canonical agent sources include both `agents` and `claude`, but `sync_paths` does not include the corresponding `.agents/skills` or `.claude/skills` target. | Add the missing canonical target to `[global].sync_paths`. |
| `missing sync path` | A configured `sync_paths` entry is not an existing directory and cannot be created because its parent is missing, or the path exists as a non-directory. | Create the parent directory or correct the configured path. |
| `singleton global scope` | Global sources name one or more canonical agents, but no sync path is configured; or multiple canonical agents are configured with fewer than two sync paths. | Add the required canonical sync paths, or remove the extra canonical source if no fan-out is intended. |

Output is one warning per line:

```text
warn  global  asymmetric fan-out: sources include "claude" but sync_paths lacks ".claude/skills"
warn  global  missing sync path: /home/alice/.agents/skills (parent does not exist)
```

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

`skillnet sync pull` can also auto-commit selected-scope dirty mirror paths
before pulling when enabled through `skillnet.toml` or CLI flags:

```toml
[sync]
auto_commit_dirty_destination = true
codex_model = "gpt-5.4-mini"
codex_reasoning_effort = "medium"
```

```sh
skillnet sync pull --scope global --auto-commit-dirty-destination
skillnet sync pull --scope global --no-auto-commit-dirty-destination
skillnet sync pull --scope global --codex-model gpt-5.3-codex --codex-reasoning-effort high
```

The auto-commit flow only applies to `sync pull`, only stages dirty paths under
the selected mirror scopes (plus `.skillnet/cache.toml`), and requires `codex`
to be installed and logged in.

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
