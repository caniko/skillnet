# Command Surface

Top-level commands:

- `status`: show canonical store, view drift, destination, and catalog health.
- `doctor`: check configured views for invariant violations.
- `completions`: generate shell completion scripts.
- `sync`: materialise configured views, promoting newer real-directory view
  entries to canonical when explicitly applied.
- `export`: copy repo-stored skills from `skills/` into the global canonical
  store and sync global views.
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
skillnet export
skillnet view sync --all
skillnet project sync --all
skillnet doctor
skillnet view status --all
skillnet project status --all
skillnet skill show global/rust-project-flake
skillnet project list
skillnet project list --format json
skillnet catalog lint
skillnet calibration heuristics list
skillnet calibration walkthrough --dry-run
skillnet hook status
```

## View And Project Commands

`skillnet project list` prints explicit and `project_discovery`-resolved
projects. With `--format json`, rows include `name`, `path`, `source`, and
`relative_path`. Discovery scans direct children of configured project-tree
classes only when a direct child is a Git checkout with a real `.skills`
directory. Protected roots and compatibility aliases stay explicit, and
`project add`/`project remove` never mutate discovered entries.

Project-tree schemas 1 and 2 share this confined primary-class discovery;
unknown versions fail before scanning.

## Bundle exclusions

Set `excluded_skills = ["retired-example"]` at the top level of `skillnet.toml`
to omit a skill from every generated bundle. Exclusions apply to canonical
directories, explicit manifest sources, and subscription providers (including
manifest-free stores) while preserving source files and subscription contents;
refreshing a provider cannot recreate excluded entries.

Run `skillnet view sync --all --allow-delete` to prune stale generated view
entries. Exclusions never delete authored source files, and a retained skill
that depends on an excluded skill fails validation before materialization.

Nix consumers pass `excludedSkills` to `lib.<system>.mkBundle` and match it in
`programs.skillnet.excludedSkills` for runtime composition. The producer
supplies the mechanism; consumers choose the names.

For provider-only exclusions, set `excluded_provider_skills` (or Nix
`excludedProviderSkills` option/argument); same-named authored or
external-manifest skills remain available. Feature detection is exposed via
`lib.bundleExclusionSupport` and `lib.projectTreeSchemaVersions` (and their
per-system equivalents).

Run `skillnet export` from a repository root that keeps globally useful skills
in VCS under `skills/`:

```sh
skillnet export
skillnet export --skill visual-rubric
skillnet --dry-run export --source skills
```

It exports each immediate child directory containing `SKILL.md` to the global
canonical store and syncs global views such as `~/.agents/skills`. Pass
`--source <PATH>` for another source directory, `--no-view-sync` to skip view
sync, and `--prune` to remove valid global skill directories absent from the
source listing. Known examples include `fragpipe/skills/*`, `tzu/skills/*`, and
`visual-rubric/skills/visual-rubric`. `.skills/` directories are usually
project-scoped; export them only with an explicit `--source .skills`.

`skillnet sync` resolves selected global and project views from `skillnet.toml`,
materialises view symlinks and project working copies, bootstraps project
mirror canonicals when possible, and reports real-directory view entries that
may need promotion to canonical.

Existing symlinks keep normal generated-view behaviour. Promotion applies to
non-symlink entries in promotion-enabled symlink views (global hardlink views
and generated global bundle views do not use this promotion path) and always
targets the view's own canonical store (global view to global canonical,
project view to that project's canonical; never cross-scope). For each
non-symlink view entry, the comparator checks:

- **Missing canonical sibling** -> `AdoptCandidate` (evaluated before checking
  mtimes).
- **Matching content signature** (relative paths, file bytes, and symlink
  targets; executable-bit-only diffs are not included, so mode-only edits can
  be discarded on relink) -> `Identical`, automatically relinked to a symlink.
- **Future view mtime** (newest mtime ahead of wall clock) -> downgraded to
  `EqualMtimeDifferentContent`, requiring an explicit `--prefer` tie-break.
- **Path divergence** (older side has paths absent from the newer side) ->
  `BothAdvanced`, preventing "newer wins" and requiring `--prefer`.
- **View newer / canonical newer / equal mtime with different content** ->
  `ViewNewer`, `CanonicalNewer`, or `EqualMtimeDifferentContent`.

By default, `skillnet sync` still performs clean symlink updates for other
entries and automatically relinks `Identical` entries, while printing pending
promotion/conflict lines and exiting `2` when unresolved entries remain; pass
global `--dry-run` for a non-mutating preview. With `--apply-promote`,
promotion stages and replaces one whole skill at a time (copying into staging,
removing the existing canonical directory, renaming the staged copy into place,
then relinking the view), preserving mtimes and permissions; there is no
per-file three-way merge or crash-atomic transactional rollback (an
interruption can leave canonical absent or the view unlinked). Sync diagnostics
and `skillnet status` are the inspection surface: `skillnet sync` prints both
mtimes for `would promote`, short hashes for `EqualMtimeDifferentContent` ties,
and summary counts for applied promotions (no `RECONCILIATION.md` manifest or
automatic Git commit is produced).

Promotion flags:

| Flag                         | Default | Behaviour                                                                                                                                                                                                                                                                               |
| ---------------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--apply-promote`            | off     | Executes pending `ViewNewer` outcomes, `--prefer view` tie-breaks, and `--adopt-new` promotions; without it, no would-promote/would-adopt mutation occurs (top-level `sync` prints `would promote` lines and leaves unadopted real directories untouched without a `would-adopt` line). |
| `--no-promote`               | off     | Disables promotion; non-symlink view entries error as in `0.5.x`. Use for CI and consumer-only hosts.                                                                                                                                                                                   |
| `--force`                    | off     | Destructively demotes `CanonicalNewer` entries (discarding view content), and resolves `EqualMtimeDifferentContent`/`BothAdvanced` in favor of canonical when paired with `--prefer canonical`. Since `0.6.0`, this is the destructive demote branch, not promotion.                    |
| `--prefer <view\|canonical>` | unset   | Tie-breaker for `EqualMtimeDifferentContent` and `BothAdvanced`. Pass `--apply-promote --prefer view` to promote the view, or `--force --prefer canonical` to destructively discard view edits (`--apply-promote --prefer canonical` alone leaves the conflict pending).                |
| `--adopt-new`                | off     | Treats `AdoptCandidate` entries as promotion candidates; acts only with `--apply-promote`.                                                                                                                                                                                              |
| `--allow-delete`             | off     | Prunes stale view symlinks (and entries with no canonical sibling under `--no-promote`); in promotion mode, unadopted real skill directories stay untouched unless `--no-promote --allow-delete` is passed.                                                                             |
| `--scope <SCOPE>`            | unset   | Selects scopes: named scope, `--scope projects`, or `--scope all`. Repeatable.                                                                                                                                                                                                          |
| `--all`                      | off     | Alias for `--scope all`.                                                                                                                                                                                                                                                                |
| `--link <symlink\|hardlink>` | unset   | Overrides link strategy for this run (global scopes default to symlink; project working copies default to hardlink).                                                                                                                                                                    |
| `--dry-run`                  | off     | Global flag. Never mutates and never escalates would-promote work to exit `2`; prints would-\* lines and exits `0`.                                                                                                                                                                     |
| `--allow-dirty-destination`  | off     | Global flag. Allows canonical writes into dirty Git working trees across every canonical write site, not just `mirror_root`.                                                                                                                                                            |

`--no-promote` conflicts with both `--apply-promote` and `--force`, keeping
destructive demotion an explicit mode.

Exit codes:

| Code | Meaning                                                                                                                                                                  |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `0`  | All outcomes were created, updated, unchanged, removed, identical, auto-demoted, or adopt candidates left visible but not promoted.                                      |
| `2`  | Either CLI usage / parse-time flag conflict (from argument parsing), or a parsed sync run with pending would-promote, would-demote-destructive, or needs-tie-break work. |
| `1`  | Any runtime error (IO, config parse, dirty destination, etc.).                                                                                                           |

`--dry-run` collapses sync's pending-work exit code `2` to `0`.

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

Use `skillnet doctor` before promotion when you want a read-only classification
of non-symlink entries (`Identical`, `ViewNewer`, `CanonicalNewer`,
`EqualMtimeDifferentContent`, `BothAdvanced`, `AdoptCandidate`) with matching
`sync` flag hints. For XDG config migration and Home Manager setup, see
[Centralised config (0.6.0)](migration/centralised-config.md).

`skillnet view sync --all` materialises global views from the canonical global
store and accepts `--link symlink|hardlink` (defaulting to generated symlinks).
Unlike top-level `skillnet sync`, user-facing `skillnet view sync` enables
promotion and adoption by default for ordinary global symlink views (preserving
authored directories and limiting `--allow-delete` to dangling symlinks); in
bundle mode, promotion is disabled and authored directories are preserved.

`skillnet project sync --all` materialises project working copies at
`canonical_rel` (default `.agents/skills`) from `<project>/.skills`, then
materialises project views (such as `.claude/skills`) as per-skill relative
symlinks into the working copy. Working copies default to hardlinked directory
twins; pass `--link symlink` or set `link_strategy = "symlink"` only when you
want a directory symlink.

Status-aware sync repairs hardlink working copies:

- missing or foreign working copies are replaced;
- severed, identical, or diverged files are re-linked from canonical;
- cross-filesystem hardlink failures error without copying or symlinking.

`skillnet project clone --all` clones missing configured projects that set
`origin`, then runs `skillnet project sync --all`. Existing paths are skipped,
`--dry-run` previews clones without invoking Git, and HTTPS origins are refused
unless `--ssh-strict=false` is passed.

Inspect drift without mutating files via `view status|diff` and
`project status|diff`; use `skillnet sync` to materialise all configured views
in one shot.

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

Rows include `would_promote` and `needs_tie_break` counts. Non-symlink drift
entries may also include `view_mtime_nanos`, `canonical_mtime_nanos`,
`view_sha`, and `canonical_sha` for read-only inspection of promotion
decisions.

## Doctor

`skillnet doctor` validates Option B canonical/view/working-copy invariants:

- global canonical store existence and global view symlinks;
- project `.skills` canonical stores, working copies, and committed in-repo
  view symlinks;
- orphan view entries without canonical siblings;
- broken or unexpected symlink targets.

Missing configured project repositories emit warnings so partially bootstrapped
hosts are not flagged as corrupted. Non-symlink view entries use the same
comparator as `skillnet sync`:

- `Identical` and `AdoptCandidate` report as Info (do not cause a non-zero
  exit). If a promotion run was interrupted after updating canonical and before
  relinking, the matching view directory reports as `Identical` (Info) and
  rerunning `skillnet sync` relinks it; a missing link is reported separately.
- `ViewNewer` reports as Warn (non-zero exit).
- `CanonicalNewer`, `EqualMtimeDifferentContent`, and `BothAdvanced` report as
  Error (non-zero exit).

Project working-copy checks follow the configured link strategy: symlink
strategy checks that the working copy points at `<project>/.skills`; hardlink
strategy verifies regular files are hardlinked twins of canonical files.
Missing working copies, symlink working copies under hardlink strategy, foreign
trees, and diverged files are errors; severed-but-identical files are warnings
because `skillnet project sync` re-links them without losing content.

## Config File Location

Config file discovery precedence, highest first (`skillnet --config <path>`
wins):

| Rank | Source                          | Resolves to                                                                      |
| ---- | ------------------------------- | -------------------------------------------------------------------------------- |
| 1    | `--config <path>`               | absolute or cwd-relative                                                         |
| 2    | `SKILLNET_CONFIG`               | absolute or cwd-relative                                                         |
| 3    | XDG config file, when present   | `$XDG_CONFIG_HOME/skillnet/skillnet.toml`, or `~/.config/skillnet/skillnet.toml` |
| 4    | legacy cwd config, when present | `./skillnet.toml`                                                                |
| 5    | missing-config error path       | XDG path                                                                         |

`--catalog-config` / `SKILLNET_CATALOG_CONFIG` follows the same ladder for
`skillnet.catalog.toml`. Rank 4 legacy cwd discovery remains active with a
deprecation warning (removal was targeted for `0.7.0` and is deferred to a
future release).

Destination root precedence: `--mirror-root`, `SKILLNET_MIRROR_ROOT`,
`skills_root` in `skillnet.toml`, legacy `mirror_root`, then `.`. For Git
destinations, `skillnet status` reports branch, origin, and dirty state, and
writing commands refuse dirty repositories unless `--allow-dirty-destination`
is passed.

The removed `sync pull` auto-commit flow has no replacement since `0.6.0`;
promotion writes canonical files when applied, and the user commits Git changes
directly.

## Calibration Database

Calibration defaults to Postgres. Select it in `skillnet.toml`:

```toml
[database]
url = "postgres://user@host/skillnet"
```

Select SQLite explicitly:

```toml
[database]
backend = "sqlite"
path = "/abs/path/calibration.sqlite"
```

`path` applies only to SQLite; `url` selects Postgres.

Database target precedence, highest first:

| Rank | Source                                       | Result                                                                                              |
| ---- | -------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| 1    | `--database-url <URL>`                       | Postgres                                                                                            |
| 2    | `SKILLNET_DATABASE_URL` or `SKILLNET_DB_URL` | Postgres                                                                                            |
| 3    | `[database].url`                             | Postgres                                                                                            |
| 4    | `SKILLNET_DATA_DIR` or `skillnet_DATA_DIR`   | SQLite at `<dir>/multi-phase-plan/calibration.sqlite`                                               |
| 5    | `[database].path` with `backend = "sqlite"`  | SQLite at the configured path                                                                       |
| 6    | no override                                  | Postgres, requiring `database.url`, `SKILLNET_DATABASE_URL`, `SKILLNET_DB_URL`, or `--database-url` |

If both a database URL and `SKILLNET_DATA_DIR`/`skillnet_DATA_DIR` are set, the
URL wins and `skillnet` prints a warning.
