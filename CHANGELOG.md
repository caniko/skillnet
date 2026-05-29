# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Project defaults now use `<project>/.agents/skills` as the canonical store and
  `<project>/.claude/skills` as the default per-skill symlink view. Projects
  that still rely on the old `.skills` default get a warning and can either set
  `canonical_rel = ".skills"` or follow `docs/src/migration/agents-canonical.md`;
  migration is documented only and is not performed automatically.
- Project aggregators now default to hardlinked directory twins under
  `<mirror_root>/projects/<name>`, giving the `ai-skills` checkout real
  committable files instead of directory symlinks. Severed-but-identical
  hardlinks are re-linked by sync; diverged files require `--force`,
  `--prefer canonical`, or manual reconciliation.
- Link strategy is configurable through top-level `link_strategy`, per-project
  `link_strategy`, and the `--link symlink|hardlink` flag on materialisation
  commands. Defaults are symlink for global scopes and hardlink for project
  aggregators.
- `skillnet sync` and other scope-aware commands now accept
  `--scope projects` and `--scope all`; `skillnet sync --all` is an alias for
  `--scope all`.
- `projects` and `all` are reserved selector tokens and can no longer be used
  as new project names.

### Fixed

- `skillnet view sync` now adopts view-only skill directories into the canonical
  store and back-syncs them as symlinks instead of deleting them. Previously
  `view sync --allow-delete` removed authored directories that had no canonical
  counterpart, causing data loss; the command now routes through the promotion
  path (adopt-and-back-sync), so `--allow-delete` only removes dangling view
  _symlinks_ and `--force` discards a view copy only when canonical is newer.
  The internal `--no-promote` path is unchanged.

## [0.6.0] - 2026-05-27

### Added

- `skillnet sync` promotes a view entry's content into the canonical store
  when the view entry is a real directory newer than canonical. Promotion is
  dry-run-by-default; the command prints `would promote ...` lines and exits 2. Pass `--apply-promote` to perform the promotion; pass `--no-promote` to
  keep `0.5.x` behaviour. Tie-breaks via `--prefer view|canonical`; promotion
  of view-only skills via `--adopt-new`.
- `skillnet config migrate` moves `skillnet.toml` and
  `skillnet.catalog.toml` from the legacy working-directory pickup to
  `$XDG_CONFIG_HOME/skillnet/`.
- `skillnet doctor` classifies non-symlink view entries by comparator outcome
  (Identical / ViewNewer / CanonicalNewer / EqualMtimeDifferentContent /
  BothAdvanced / AdoptCandidate) with hint strings pointing at the appropriate
  sync flag.
- `skillnet status --format json` rows expose `would_promote` and
  `needs_tie_break` counts; per-entry `view_mtime_nanos`,
  `canonical_mtime_nanos`, `view_sha`, `canonical_sha` populated for
  non-symlink entries.
- HM module: `programs.skillnet.activation.{promote,failOnConflict,allowDelete}`
  toggle activation behaviour. Default `promote = false`,
  `failOnConflict = true`, `allowDelete = true`. Consumer hosts upgrading from
  `0.5.x` see the same behaviour they had, except activation now fails loudly
  on drift (was: silently masked via `|| true`).
- Per-target `--allow-dirty-destination`: the gate now covers every canonical
  write, not just `mirror_root`.

### Changed

- HM activation script collapses `view sync` + `project sync` calls into a
  single `skillnet sync` invocation.
- `skillnet project add` / `project remove` refuse to mutate a config managed
  by Home Manager (resolved path under `/nix/store/`). Edit
  `programs.skillnet.settings` in your HM config instead.

### Deprecated

- Legacy working-directory config discovery (`./skillnet.toml`,
  `./skillnet.catalog.toml`). The CLI prints a deprecation warning when it
  falls through to this path and will remove it in `0.7.0`. Run
  `skillnet config migrate` to move existing configs.

### Notes

- The `0.5.0` design stance "no reconcile, canonical is the only writer" is
  partially reversed. Promotion is opt-in per invocation (`--apply-promote`),
  opt-in per host via the HM toggle, and dry-run-by-default in every other
  setting.
- Forgejo CI and publish workflows were refreshed from simit's generated
  release infrastructure before tagging.
- Release tests no longer depend on chmod-denied fixture reads, which can vary
  by CI container user.

## [0.5.1] - 2026-05-25

### Added

- `skillnet project clone --all` clones configured missing project repositories
  from optional `[[projects]].origin` values, supports `--dry-run`, refuses
  HTTPS origins by default, and runs `project sync --all` after live clones.
- `docs/src/migration/option-b.md` documents the legacy schema removal, new
  canonical/view layout, and fresh-host bootstrap workflow.

### Changed

- `skillnet doctor` now validates Option B invariants across global and
  project scopes: canonical store existence, view symlink shape, orphan view
  entries, missing canonical view entries, and, in `0.5.x`, project aggregator
  symlink freshness.
- Missing configured project repositories are reported as doctor warnings so
  partially bootstrapped hosts get actionable output without being labeled as
  corrupted state.

## [0.5.0] - 2026-05-25

`0.5.0` removes the pre-Option-B reconcile model. There is now one
canonical store per scope, and configured global/project views are derived
from that store.

### Changed

- Option B is now the only supported skill layout: `[global].views` define
  global view symlinks, projects use `canonical_rel` plus project `views`, and
  project aggregators point at each project's canonical store.
- `skillnet status` reports canonical store and view health instead of
  live-source divergence.
- The long-term plan keeps the "Updated Work That Should Survive Into The
  Long-Term Plan" items from the mirror canonical store dossier: canonical
  upstream stores, generated views, Home Manager activation materialisation,
  and downstream canix migration.

### Removed

- Removed reconcile source arbitration and deleted `src/reconcile.rs`.
- Removed the `skillnet sync` command group, including `sync pull` and
  `sync roundtrip`.
- Removed `.skillnet/cache.toml` writes; view sync status is derived directly
  from configured symlinks.
- Removed support for legacy config fields `sources`, `sync_paths`,
  `stale_codex_skill_paths`, `project_source_rules`, and `extra_sources`.

### Migration

- See `MIGRATION.md` for the planted Option B migration notes. P9 expands
  that into `docs/src/migration/option-b.md`.

## [0.4.0] - 2026-05-24

`0.4.0` adds the first-class heuristics catalog, helper commands,
walkthrough orchestrator, and SemVer-stable `analyze --format json` schema.
`ai-skills` can consume this release as the calibration backend for
`multi-phase-plan`.

### Added

- Heuristics catalog under `src/calibration/catalog/`, with user-facing
  heuristics, meta-heuristics, code defaults, and runtime threshold overrides
  through the `heuristic_thresholds` table.
- `skillnet calibration init <plan-dir>` to bootstrap `.calibration.json` from
  a plan directory.
- `skillnet calibration eval <plan-dir>` to evaluate catalog heuristics and
  emit trigger rows.
- `skillnet calibration meta-heuristics <plan-dir>` to report firing
  meta-heuristics.
- `skillnet calibration shape-hash <plan-dir>` to print a deterministic plan
  shape hash.
- `skillnet calibration heuristics list|show` to inspect the heuristic
  catalog.
- `skillnet calibration walkthrough` to run the orchestrated calibration flow:
  analyze, propose, decide, and export changelog. It supports
  `--non-interactive`, `--decisions <file>`, `--dry-run`, and
  `--skill-md <path>`.
- mdBook pages documenting the calibration JSON schema and verifier surprises
  convention.

### Changed

- `skillnet calibration analyze --format json` is now a SemVer-stable schema.
  The top-level object includes `schema_version: 1`, and trigger rows include
  `threshold_source` provenance for default and override thresholds.
- `skillnet calibration decide accept` writes accepted thresholds to
  `heuristic_thresholds`, closing the calibration loop.

### Schema migrations

- Added `data/multi-phase-plan/schema/002-heuristic-thresholds.sql` and
  `data/multi-phase-plan/schema-pg/002-heuristic-thresholds.sql`. The
  migrations create the override table and seed code defaults idempotently.

## [0.3.0] - 2026-05-24

### Added

- CLI: `SKILLNET_CONFIG`, `SKILLNET_CATALOG_CONFIG`, `SKILLNET_MIRROR_ROOT` env-var fallbacks for the corresponding global flags. `skillnet status` no longer requires `./skillnet.toml` in cwd.
- HM module: `programs.skillnet.settings` and `catalogSettings` declarative options render `skillnet.toml` / `skillnet.catalog.toml` from Nix and auto-export the env vars above.
- HM module: `programs.skillnet.configFile` / `catalogConfigFile` for pointing at user-managed TOMLs without rendering them from Nix.
- HM module: `programs.skillnet.database.urlFile` reads the Postgres URL from a file at shell init, keeping secrets out of the world-readable `hm-session-vars.sh`.

### Fixed

- HM module: `programs.skillnet.database.path` for SQLite is now honoured end-to-end; activation creates the parent directory.
- HM module: `programs.skillnet.skillsRoot` missing on disk now warns instead of aborting `home-manager switch`.

### Removed

- HM module: `programs.skillnet.extraConfig`, a no-op placeholder in 0.2.0 superseded by `settings`.

## [0.2.0] - 2026-05-23

- Add a backend abstraction for calibration storage and an optional `postgres` Cargo feature.
- Add `[database]` config support for selecting `sqlite` or `postgres`, including SQLite `path` and Postgres `url` settings.
- Add the global `--database-url` flag plus `SKILLNET_DATABASE_URL` and `SKILLNET_DB_URL` environment-variable overrides.
- Extend the Home Manager module with `programs.skillnet.database.{backend,path,url}` and Postgres session-variable wiring.
- Add backend parity coverage for calibration database round trips.
- Raise the crate MSRV and Forgejo release containers to Rust 1.88 for dependencies and release tooling that require a newer Cargo.

## [0.1.1] - 2026-05-23

- Add a Nix Home Manager module exported as `hmModules.default` and `hmModules.skillnet`.
- Install `skillnet` through `programs.skillnet.enable`, provision its data directory, and expose runtime data-dir session variables.
- Make `Db::default_path` honor `$skillnet_DATA_DIR` and `$SKILLNET_DATA_DIR`, falling back to `$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`.
- Remove the compiled-in author-machine fallback through `$AI_SKILLS_REPO`.

## [0.1.0] - 2026-05-23

Initial release.

- Reconcile live `.agents`, `.claude`, and `.codex` skill sources into checked-in mirror directories.
- Manage mirror scopes, mirrored skill edits, catalog generation, and shell completions from the `skillnet` CLI.
- Record and analyze `multi-phase-plan` calibration data with embedded SQLite schema migrations.
- Add release metadata, crates.io packaging rules, mdBook docs, and Forgejo-ready release infrastructure.

No stable Rust library API is committed in `0.1.0`; the supported surface is the `skillnet` binary.

[Unreleased]: https://codeberg.org/caniko/skillnet/compare/0.6.0...HEAD
[0.6.0]: https://codeberg.org/caniko/skillnet/compare/0.5.1...0.6.0
[0.5.1]: https://codeberg.org/caniko/skillnet/compare/0.4.0...0.5.1
[0.5.0]: https://codeberg.org/caniko/skillnet/compare/0.4.0...0.5.0
[0.4.0]: https://codeberg.org/caniko/skillnet/compare/0.3.0...0.4.0
[0.3.0]: https://codeberg.org/caniko/skillnet/compare/0.2.0...0.3.0
