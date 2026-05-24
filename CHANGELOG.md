# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://codeberg.org/caniko/skillnet/compare/0.3.0...HEAD
[0.3.0]: https://codeberg.org/caniko/skillnet/compare/0.2.0...0.3.0
