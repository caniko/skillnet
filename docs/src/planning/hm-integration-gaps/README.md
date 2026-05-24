# Plan — Home Manager integration gaps

> **Recommended Codex model for orchestration: GPT 5.5 medium**
>
> The plan is moderately scoped: one CLI change, focused Nix module edits, a
> test extension, and docs. Single-orchestrator coordination across five
> sequential phases — no novel design work. `medium` is the right tier; bumping
> to `high` buys nothing here.

## Scope

`skillnet` 0.2.0 is installed via the Home Manager module on this user's
machine, but every command fails with `failed to read config file
skillnet.toml` because the CLI resolves its TOML config relative to cwd and
the HM module does nothing about it. This plan closes that gap and the four
related gaps in [nix/hm-module.nix](../../../../nix/hm-module.nix):

1. The CLI does not read `SKILLNET_CONFIG` / `SKILLNET_CATALOG_CONFIG`, so HM
   has no env-var hook to point the binary at user-managed TOMLs.
2. The HM module has no way to declare or write either TOML from Nix.
3. `programs.skillnet.database.path` is dropped on the floor for the SQLite
   backend.
4. `programs.skillnet.skillsRoot` hard-fails `home-manager switch` when the
   directory is absent (e.g., fresh machine), instead of warning.
5. `programs.skillnet.database.url` is exported into
   `hm-session-vars.sh` (world-readable in the Nix store), leaking any
   embedded Postgres password.
6. `nix/test-hm-module.nix` only exercises `--help` and `calibration migrate`
   — neither requires the TOML config, so the regression went uncaught.

## Current state

- CLI: [src/cli/args.rs:17](../../../../src/cli/args.rs#L17) defines
  `--config` / `--catalog-config` with literal cwd-relative defaults and no
  `env =` attribute.
- HM module: [nix/hm-module.nix](../../../../nix/hm-module.nix) exports
  `SKILLNET_DATABASE_URL`, `SKILLNET_DATA_DIR`, `AI_SKILLS_REPO` but
  nothing for config-file location; activation script `exit 1`s on missing
  `skillsRoot`.
- Tests: [nix/test-hm-module.nix:75-77](../../../../nix/test-hm-module.nix#L75-L77)
  runs only `skillnet --help` and `skillnet calibration migrate`.

## Phase table

| Phase | File | Model | Depends on | Touches | Blocking? |
|---|---|---|---|---|---|
| 01 | [01-cli-env-vars.md](./01-cli-env-vars.md) | 5.5 low | — | `src/cli/args.rs`, `src/config.rs` (env precedence note), tests | unblocks 02, 04 |
| 02 | [02-hm-env-and-fixes.md](./02-hm-env-and-fixes.md) | 5.5 medium | 01 | `nix/hm-module.nix` | unblocks 03, 04 |
| 03 | [03-hm-declarative-config.md](./03-hm-declarative-config.md) | 5.5 medium | 02 | `nix/hm-module.nix` | unblocks 04 |
| 04 | [04-test-non-cwd-status.md](./04-test-non-cwd-status.md) | 5.5 medium | 01, 02, 03 | `nix/test-hm-module.nix` | gates ship |
| 05 | [05-docs-changelog.md](./05-docs-changelog.md) | 5.5 low | 01–04 | `docs/src/commands.md`, `README.md`, `CHANGELOG.md`, `docs/src/SUMMARY.md` | none |

## Parallelism layer

This plan is fully serial. Every later phase either edits a file an earlier
phase has just modified ([nix/hm-module.nix](../../../../nix/hm-module.nix)
across phases 02 and 03) or consumes the env-var contract introduced
upstream.

- **Wave 0** — Phase 01 only. Adds the `SKILLNET_CONFIG` /
  `SKILLNET_CATALOG_CONFIG` (and `SKILLNET_MIRROR_ROOT`) clap env hooks. No
  Nix edits.
- **Wave 1** — Phase 02. Wires the new env vars into the HM module and fixes
  the four orthogonal HM gaps (`database.path`, `skillsRoot` soft-warn,
  `database.urlFile`, drop `skillnet_DATA_DIR` lowercase typo or document
  it). Must follow 01 so env var names match.
- **Wave 2** — Phase 03. Adds `programs.skillnet.settings` /
  `catalogSettings` options that serialise to TOML under
  `$XDG_CONFIG_HOME/skillnet/` and point the env vars from phase 02 at the
  generated files. Same file as phase 02 → serial.
- **Wave 3** — Phase 04. Extends the activation test to run `skillnet
  status` from a working directory that does **not** contain
  `skillnet.toml`, relying entirely on the env-var contract.
- **Wave 4** — Phase 05. Docs, CHANGELOG, SUMMARY entry.

Plan is exhausted after Wave 4.

## Whole-set acceptance criteria

- [ ] `skillnet status` succeeds when invoked from `/tmp` (or any directory
  without `skillnet.toml`) on a host whose user has the HM module enabled
  with `settings = { … }` declared.
- [ ] `nix flake check` passes, including the extended HM activation test.
- [ ] `programs.skillnet.database.urlFile` is honoured and the Postgres URL
  no longer appears in `hm-session-vars.sh`.
- [ ] `programs.skillnet.database.path` (sqlite) is honoured end-to-end.
- [ ] `home-manager switch` no longer hard-fails when `skillsRoot` is
  absent; a warning is emitted instead.
- [ ] CHANGELOG records the new env vars, new HM options, and the
  bug fixes; docs/commands.md documents the env var precedence; mdBook
  builds.

## Global constraints

- Preserve back-compat for users who already invoke `skillnet --config
  ./skillnet.toml` from a project directory. The clap `env =` hook is a
  fallback below the explicit `--config` flag, not a replacement.
- Do not break the existing `SKILLNET_DATABASE_URL` /
  `SKILLNET_DATA_DIR` env vars. The lowercase `skillnet_DATA_DIR`
  alias stays for now (phase 02 can mark it deprecated in a doc comment).
- Module options must remain optional — a user with only `enable = true;`
  and `database.url = "..."` should still get a working install (cwd-based
  TOMLs continue to work).

## Reference

- Originating diagnosis: chat transcript identifying the six gaps.
- Prior plan that introduced the module: `docs/src/planning/postgres-and-hm-module/`.
- Convention for plan layout: `multi-phase-plan` skill.
