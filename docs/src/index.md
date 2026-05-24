# skillnet

`skillnet` manages a checked-in mirror of AI skills and the live directories that feed it.

The current release focuses on the CLI:

- reconcile live `.agents`, `.claude`, and `.codex` trees into mirror scopes;
- inspect, move, rename, and delete mirrored skills;
- generate catalog output from `skillnet.catalog.toml`;
- record and analyze `multi-phase-plan` calibration data;
- store calibration data in Postgres by default, with explicit SQLite support;
- install and configure the binary through the exported Nix Home Manager module.

The supported public surface is the `skillnet` binary. No stable Rust library API is promised.
