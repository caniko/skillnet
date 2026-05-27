# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
  entries, missing canonical view entries, and project aggregator symlink
  freshness.
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

[Unreleased]: https://codeberg.org/caniko/skillnet/compare/0.5.1...HEAD
[0.5.1]: https://codeberg.org/caniko/skillnet/compare/0.4.0...0.5.1
[0.5.0]: https://codeberg.org/caniko/skillnet/compare/0.4.0...0.5.0
[0.4.0]: https://codeberg.org/caniko/skillnet/compare/0.3.0...0.4.0
[0.3.0]: https://codeberg.org/caniko/skillnet/compare/0.2.0...0.3.0
