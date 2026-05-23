# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog, and this project follows Semantic Versioning.

## [Unreleased]

## [0.1.0] - 2026-05-23

Initial release.

- Reconcile live `.agents`, `.claude`, and `.codex` skill sources into checked-in mirror directories.
- Manage mirror scopes, mirrored skill edits, catalog generation, and shell completions from the `skillnet` CLI.
- Record and analyze `multi-phase-plan` calibration data with embedded SQLite schema migrations.
- Add release metadata, crates.io packaging rules, mdBook docs, and Forgejo-ready release infrastructure.

No stable Rust library API is committed in `0.1.0`; the supported surface is the `skillnet` binary.
