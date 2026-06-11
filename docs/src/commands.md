# Command Surface

Top-level commands:

- `status`: show canonical store, view drift, destination, and catalog health.
- `doctor`: check configured views for invariant violations.
- `completions`: generate shell completion scripts.
- `sync`: materialise every configured view, promoting view to canonical when
  a view entry is a real directory newer than canonical and promotion is
  explicitly applied.
- `view`: materialise and inspect global view symlinks.
- `skill`: list, inspect, and edit canonical skill directories.
- `scope`: inspect configured canonical scopes.
- `project`: manage configured project roots and materialise project views.
- `catalog`: generate and validate skill catalog metadata.
- `calibration`: record, inspect, analyze, and tune `multi-phase-plan`
  calibration data.
- `hook`: install, remove, check, and run managed Claude Code hook ingestion.

Examples:

```sh
skillnet sync
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
skillnet hook status
```

## View And Project Commands

`skillnet sync` is the top-level materialisation command. It resolves every
selected global view and project view from `skillnet.toml`, materialises the
view symlinks, materialises project working copies, bootstraps project mirror
canonicals when possible, and reports real-directory view entries that may need
promotion back into canonical.

When a view entry is already a symlink, `sync` keeps the normal generated-view
behaviour. When a view entry is a real directory, `sync` compares that content
with the canonical skill of the same name. A view entry whose content is newer
than canonical is reported as a would-promote candidate by default; pass
`--apply-promote` to copy that view content into canonical and then replace the
view entry with a symlink.

Promotion flags:

| Flag                         | Default | Behaviour                                                                                                                                                                               |
| ---------------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--apply-promote`            | off     | Executes pending `ViewNewer` outcomes and any `--prefer`-resolved or `--adopt-new`-promoted outcomes. Without this flag, those outcomes are reported as would-promote/would-adopt only. |
| `--no-promote`               | off     | Hard-disables the promotion path entirely. Non-symlink view entries error as in `0.5.x`. Use this for CI and consumer-only hosts.                                                       |
| `--force`                    | off     | Demotes `CanonicalNewer` entries, destroying view-side content. In `0.6.0`, this is the destructive demote branch rather than the promotion branch.                                     |
| `--prefer <view\|canonical>` | unset   | Tie-breaker for `EqualMtimeDifferentContent` and `BothAdvanced`. Only consulted when `--apply-promote` is also passed.                                                                  |
| `--adopt-new`                | off     | Treats `AdoptCandidate` outcomes as promotion candidates. Only acts when `--apply-promote` is also passed.                                                                              |
| `--allow-delete`             | off     | Existing pruning semantics. Removes view entries with no canonical sibling and no `--adopt-new`.                                                                                        |
| `--scope <SCOPE>`            | unset   | Selects named scopes. Use `--scope projects` for every project, or `--scope all` for global plus every project. May be repeated.                                                        |
| `--all`                      | off     | Alias for `--scope all`.                                                                                                                                                                |
| `--link <symlink\|hardlink>` | unset   | Overrides the resolved link strategy for this materialisation run. Global scopes default to symlink; project working copies default to hardlink.                                        |
| `--dry-run`                  | off     | Global flag. Never mutates and never escalates would-promote work to exit code `2`; prints would-\* lines and exits `0`.                                                                |
| `--allow-dirty-destination`  | off     | Global flag. Allows canonical writes even when the destination Git working tree is dirty. This now gates every canonical write site, not just `mirror_root`.                            |

`--apply-promote` conflicts with `--no-promote`. `--force` also conflicts with
`--no-promote`, so a destructive demote must be requested as its own explicit
mode.

Exit codes:

| Code | Meaning                                                                                                                             |
| ---- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `0`  | All outcomes were created, updated, unchanged, removed, identical, auto-demoted, or adopt candidates left visible but not promoted. |
| `2`  | At least one would-promote, would-demote-destructive, or needs-tie-break outcome was reported and not actioned.                     |
| `1`  | Any other error, including IO errors, parse errors, dirty destinations, or parse-time flag conflicts.                               |

`--dry-run` collapses exit code `2` to `0` because it is an explicit preview.

Example:

```sh
skillnet sync
# global:claude  /home/alice/.claude/skills (+0 ~0 =1 -0)
# would promote /home/alice/.claude/skills/rust-project-flake -> /home/alice/skills/global_skills/rust-project-flake (view_mtime=..., canonical_mtime=...)
# exits 2

skillnet sync --apply-promote
# promotes the view-side skill into canonical, then re-links the view
# exits 0
```

Use `skillnet doctor` before a promotion run when you want a read-only
classification of non-symlink entries. Doctor reports `Identical`,
`ViewNewer`, `CanonicalNewer`, `EqualMtimeDifferentContent`, `BothAdvanced`,
and `AdoptCandidate` with hints for the matching `sync` flag. For the
centralised XDG config migration and Home Manager pattern, see
[Centralised config (0.6.0)](migration/centralised-config.md).

`skillnet view sync --all` materialises configured global views from the
canonical global store. It accepts `--link symlink|hardlink` for parity with
the materialisation interface, but global views default to generated symlinks.

`skillnet project sync --all` materialises project-local working copies at
each project's `canonical_rel` (default `.agents/skills`) from the canonical
`<project>/.skills` store, then materialises configured project views.
Project views such as `.claude/skills` remain per-skill relative symlinks into
the project working copy. Project working copies default to hardlinked
directory twins of the canonical store. Use `--link symlink` or
`link_strategy = "symlink"` only when you intentionally want the working copy to
be a directory symlink.

Hardlink working copies are read and repaired by status-aware sync:

- missing or foreign working copies are replaced during sync;
- severed, identical, or diverged working-copy files are re-linked from
  canonical;
- cross-filesystem hardlink failures are errors, with no copy or symlink
  fallback.

`skillnet project clone --all` clones every configured project whose `path`
does not exist and whose `origin` is configured, then runs
`skillnet project sync --all`. It is idempotent for projects already present on
disk. Use `--dry-run` to print intended clones without invoking Git. By
default, HTTPS origins are refused; pass `--ssh-strict=false` only when HTTPS
clones are intended.

Use `view status|diff` and `project status|diff` to inspect derived view drift
without mutating files. `skillnet sync` is the preferred one-shot command when
you want all configured views materialised from the same config.

Scope selectors:

| Selector           | Meaning                                 |
| ------------------ | --------------------------------------- |
| `--scope global`   | Selects the global scope.               |
| `--scope <name>`   | Selects one configured project by name. |
| `--scope projects` | Selects every configured project.       |
| `--scope all`      | Selects global plus every project.      |
| `--all`            | Alias for `--scope all` on `sync`.      |

## Status JSON Schema

`skillnet status --format json --scope <SCOPE>` emits a pretty JSON array:

```json
[
  {
    "scope": "global",
    "kind": "global",
    "canonical_path": "/home/alice/skills/global",
    "skill_count": 42,
    "drift_entries": 0,
    "would_promote": 0,
    "needs_tie_break": 0
  }
]
```

Rows include `would_promote` and `needs_tie_break` counts. Individual
non-symlink drift entries may also include `view_mtime_nanos`,
`canonical_mtime_nanos`, `view_sha`, and `canonical_sha` so promotion
decisions can be inspected without mutating the filesystem.

## Doctor

`skillnet doctor` checks the Option B canonical/view/working-copy invariants:

- global canonical store existence and global view symlinks;
- project `.skills` canonical stores, project working copies, and committed
  in-repo view symlinks;
- orphan view entries that do not correspond to canonical skill names;
- broken or unexpected symlink targets.

Missing configured project repositories are warnings because a fresh host may
not have cloned every project yet. Non-symlink view entries are classified by
the same comparator used by `skillnet sync`: `Identical`, `ViewNewer`,
`CanonicalNewer`, `EqualMtimeDifferentContent`, `BothAdvanced`, or
`AdoptCandidate`.

For project working copies, doctor follows the configured link strategy.
Symlink strategy checks the configured working-copy path points at
`<project>/.skills`. Hardlink strategy verifies the project-local
working-copy directory's regular files are hardlinked twins of canonical files.
Missing working copies, symlink working copies under hardlink strategy, foreign
trees, and diverged files are errors. Severed-but-identical files are warnings
because `skillnet project sync` can re-link them without discarding content.

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

The removed `sync pull` auto-commit flow still has no replacement in `0.6.0`.
Promotion writes canonical content when explicitly applied, but the user still
commits canonical store changes with Git directly.

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
