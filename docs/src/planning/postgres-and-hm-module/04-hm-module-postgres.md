# Phase 04 — Home Manager module: Postgres + housekeeping

> **Recommended Codex model: GPT 5.5 medium**
>
> Nix module work with a clear shape: add a `database` submodule mirroring
> the config schema from phase 03, set env vars accordingly, and verify
> with the existing `nix/test-hm-module.nix` harness. Medium is enough; no
> systemd unit design, no service orchestration.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`, with phase 03 landed.

## Goal

Update the Home Manager module so users can declare which backend
`skillnet` should target, without editing `skillnet.toml` by hand. The
module continues to install `skillnet` on `PATH` and manage the data
directory when SQLite is used.

## Why

The README already advertises HM-based installation. Users adopting
Postgres need a first-class option, not a "drop a TOML file somewhere"
workaround.

## Out of scope

- Provisioning a Postgres server (no `services.postgresql` wiring).
- Secret management for the URL (callers use their preferred mechanism;
  the module accepts a plain string today).
- Multi-user / system-level (NixOS) module — Home Manager only.

## Plan

1. **Schema.** Extend `programs.skillnet` in `nix/hm-module.nix` with:
   ```nix
   database = {
     backend = lib.mkOption {
       type    = lib.types.enum [ "sqlite" "postgres" ];
       default = "sqlite";
       description = "Calibration storage backend.";
     };
     path = lib.mkOption {
       type        = lib.types.nullOr lib.types.str;
       default     = null;
       description = "SQLite database path; defaults to <dataDir>/multi-phase-plan/calibration.sqlite.";
     };
     url = lib.mkOption {
       type        = lib.types.nullOr lib.types.str;
       default     = null;
       description = "Postgres connection URL; required when backend = \"postgres\".";
     };
   };
   ```
   Assert via `lib.mkAssertion` (or `assertions`) that `backend ==
   "postgres" → url != null`.
2. **Env wiring.** When `backend == "postgres"`, export
   `SKILLNET_DATABASE_URL = cfg.database.url`. When `backend == "sqlite"`,
   keep the existing `SKILLNET_DATA_DIR` / `skillnet_DATA_DIR` exports.
3. **Activation script.** Only create the data directory when `backend ==
   "sqlite"`; for Postgres it's pointless. Guard the
   `home.activation.skillnet-data-dir` entry behind the backend check.
4. **Package option.** Document that the `postgres` feature is needed for
   the Postgres backend. Add a note in the module description; do not
   silently override `cfg.package` (users pick their own build). If the
   default `pkgs.skillnet` is not built with `--features postgres`, the
   runtime selector from phase 03 will produce the actionable error.
5. **Tests.** Update `nix/test-hm-module.nix` with two evaluation cases:
   one for the default SQLite path, one for `backend = "postgres"` with a
   dummy URL. Both must evaluate; the Postgres case asserts that
   `SKILLNET_DATABASE_URL` is present in `home.sessionVariables` and that
   the data-dir activation is absent.
6. **README.** Update the `### Nix Home Manager` section in `README.md`
   with a Postgres example and the option list.

## Acceptance criteria

- [ ] `nix/hm-module.nix` exposes `programs.skillnet.database.{backend,
      path, url}` with the assertions above.
- [ ] `programs.skillnet.backend = "postgres"` produces a config that
      sets `SKILLNET_DATABASE_URL` and omits data-dir creation.
- [ ] `nix flake check` (or the existing test invocation used by
      `nix/test-hm-module.nix`) passes for both backend scenarios.
- [ ] README documents the new options with a Postgres usage block.
- [ ] No existing default behaviour changes for users who don't set
      `programs.skillnet.database`.

## Files likely touched

- `nix/hm-module.nix`
- `nix/test-hm-module.nix`
- `flake.nix` (if the test attribute needs to be exposed differently)
- `README.md`

## Pitfalls

- **Backwards compatibility.** Default `database.backend = "sqlite"` and
  `path = null` must produce exactly the current behaviour. A user who
  has not opted in must see zero diff in their generated home env.
- **Assertions vs evaluation errors.** Use the module-system `assertions`
  mechanism so the message is user-readable; avoid raw `throw`s buried
  inside `mkIf` branches.
- **URL secret in the Nix store.** Setting `url = "postgres://user:pw@…"`
  puts the password in the world-readable Nix store. Mention this in the
  README and recommend a wrapper (sops-nix, agenix, or shell-sourced env)
  for production use. Do not implement secret loading in this phase.

## Reference

- Phase 03 config + env var contract.
- Current HM module: `nix/hm-module.nix`.
- Existing test scaffold: `nix/test-hm-module.nix`.
