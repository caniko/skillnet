# Phase 02 — HM module: env wiring + four orthogonal fixes

> **Recommended Codex model: GPT 5.5 medium**
>
> Four small but non-mechanical changes in one Nix module: export new env
> vars introduced in phase 01, wire `database.path` for sqlite, soften the
> `skillsRoot` activation, and add `database.urlFile` so passwords stop
> leaking into `hm-session-vars.sh`. Each fix is small individually but the
> module's `mkMerge`/`mkIf` plumbing needs careful editing to keep the
> existing test ([nix/test-hm-module.nix](../../../../nix/test-hm-module.nix))
> green. `medium` is correct; `low` risks subtle option-merging bugs.

## Working tree

Same repo: `/data/nvme0/can/Projects/skillnet`. Phase 01 must already be
merged so the env var names `SKILLNET_CONFIG` and `SKILLNET_CATALOG_CONFIG`
are honoured by the binary.

## Goal

[nix/hm-module.nix](../../../../nix/hm-module.nix) exports
`SKILLNET_CONFIG` and `SKILLNET_CATALOG_CONFIG` (the env hooks introduced
in phase 01), honours `programs.skillnet.database.path` end-to-end for the
sqlite backend, emits a warning instead of `exit 1` when `skillsRoot` is
absent, and offers `programs.skillnet.database.urlFile` so secrets stay out
of `hm-session-vars.sh`.

## Why this matters now

Phase 01 added env-var hooks in the CLI; this phase is what makes them
visible after `home-manager switch`. The other three fixes are gaps the
README of this plan documents:

- `database.path` is declared but ignored — the SQLite branch only sets
  `SKILLNET_DATA_DIR` from `dataDir`, not from `database.path`. Anyone who
  sets `database.path` silently gets a different file location than they
  asked for.
- `skillsRoot` activation aborts with `exit 1` when the directory is
  missing, bricking the first `home-manager switch` on a fresh machine.
- The Postgres URL goes into the world-readable
  `/nix/store/.../hm-session-vars.sh`. For users who embed credentials in
  the URL (the common case for self-hosted Postgres), that's a real leak.

## Out of scope

- Adding `programs.skillnet.settings` / `catalogSettings` options that
  *write* TOML — that's phase 03. This phase only points env vars at
  whatever the user gives it (an option type `nullOr str` for each path).
- Renaming `dataDir` or `database.url`. Back-compat must be preserved.
- Removing the lowercase `skillnet_DATA_DIR` alias. Adjust the option
  description to mark it deprecated; deletion can happen in a later
  release.

## Plan

1. **Add path options for the new env vars.** Add two new options under
   `options.programs.skillnet`:

   ```nix
   configFile = lib.mkOption {
     type = lib.types.nullOr lib.types.path;
     default = null;
     description = ''
       Absolute path to skillnet.toml. When set, exported as
       SKILLNET_CONFIG so the binary can be invoked from any directory.
       Leave null to keep the cwd-based default behaviour.
     '';
   };

   catalogConfigFile = lib.mkOption {
     type = lib.types.nullOr lib.types.path;
     default = null;
     description = ''
       Absolute path to skillnet.catalog.toml. Exported as
       SKILLNET_CATALOG_CONFIG when set.
     '';
   };
   ```

   In the `config` block, conditionally export:

   ```nix
   (lib.mkIf (cfg.configFile != null) {
     home.sessionVariables.SKILLNET_CONFIG = toString cfg.configFile;
   })
   (lib.mkIf (cfg.catalogConfigFile != null) {
     home.sessionVariables.SKILLNET_CATALOG_CONFIG = toString cfg.catalogConfigFile;
   })
   ```

2. **Wire `database.path` for the sqlite backend.** Today
   [nix/hm-module.nix:72-81](../../../../nix/hm-module.nix#L72-L81) only
   sets `SKILLNET_DATA_DIR` from `cfg.dataDir`. Make the sqlite branch
   prefer `database.path`'s parent when set, falling back to `dataDir`.
   Concretely: if `cfg.database.path != null`, set
   `SKILLNET_DATA_DIR = dirOf cfg.database.path` (or just export both vars
   if `src/calibration/db.rs` reads `database.path` distinctly — check
   [src/calibration/db.rs:120](../../../../src/calibration/db.rs#L120)
   first; the precedence table in
   [docs/src/commands.md:44-53](../../../../docs/src/commands.md#L44-L53)
   says `[database].path` is honoured directly from the TOML, so the HM
   side only needs to ensure the parent dir exists). The simpler fix:

   ```nix
   (lib.mkIf (cfg.database.backend == "sqlite") {
     home.sessionVariables = {
       SKILLNET_DATA_DIR = cfg.dataDir;
       skillnet_DATA_DIR = cfg.dataDir;  # deprecated alias
     };
     home.activation.skillnet-data-dir = lib.hm.dag.entryAfter ["writeBoundary"] ''
       $DRY_RUN_CMD mkdir -p ${lib.escapeShellArg cfg.dataDir}
       ${lib.optionalString (cfg.database.path != null) ''
         $DRY_RUN_CMD mkdir -p ${lib.escapeShellArg (builtins.dirOf cfg.database.path)}
       ''}
     '';
   })
   ```

   Add an assertion: `cfg.database.backend == "sqlite" -> cfg.database.path == null || lib.hasPrefix "/" cfg.database.path` (must be absolute).

3. **Soften `skillsRoot` activation.** Replace the `exit 1` block at
   [nix/hm-module.nix:90-96](../../../../nix/hm-module.nix#L90-L96) with a
   warning that does not abort:

   ```nix
   home.activation.skillnet-skills-root = lib.hm.dag.entryAfter ["writeBoundary"] ''
     if [ ! -d ${lib.escapeShellArg cfg.skillsRoot} ]; then
       echo "skillnet: programs.skillnet.skillsRoot does not exist: ${cfg.skillsRoot}" >&2
       echo "skillnet: clone or restore the ai-skills checkout at that path; skipping for now." >&2
     fi
   '';
   ```

   Update the test expectation in phase 04 accordingly (no longer asserts
   failure mode).

4. **Add `database.urlFile`.** New option:

   ```nix
   urlFile = lib.mkOption {
     type = lib.types.nullOr lib.types.path;
     default = null;
     description = ''
       Path to a file containing the Postgres connection URL. Preferred
       over database.url because url ends up in the world-readable
       hm-session-vars.sh. Read at shell init via a small wrapper that
       exports SKILLNET_DATABASE_URL=$(cat <urlFile>).
     '';
   };
   ```

   Implementation: when `urlFile != null`, do **not** set
   `home.sessionVariables.SKILLNET_DATABASE_URL`. Instead, inject a
   session-init snippet:

   ```nix
   programs.bash.initExtra = lib.mkIf (cfg.database.urlFile != null) ''
     if [ -r ${lib.escapeShellArg cfg.database.urlFile} ]; then
       export SKILLNET_DATABASE_URL="$(cat ${lib.escapeShellArg cfg.database.urlFile})"
     fi
   '';
   programs.zsh.initExtra = ...; programs.fish.shellInit = ...;
   ```

   (Match whichever shells the existing module already touches. If only
   bash is targeted, that's fine; document the limitation.) Update the
   Postgres-backend assertion to allow either `url` or `urlFile`:

   ```nix
   assertion = cfg.database.backend != "postgres"
     || cfg.database.url != null
     || cfg.database.urlFile != null;
   message = "programs.skillnet.database needs `url` or `urlFile` when backend = \"postgres\".";
   ```

5. **Update the existing test stub** so it still passes (full extension
   happens in phase 04). At minimum, verify `nix flake check` evaluates
   the module; the existing
   [nix/test-hm-module.nix](../../../../nix/test-hm-module.nix) block that
   greps for the `exit 1` activation message must be relaxed (the grep on
   line 87 will fail because the new message no longer says `exit 1`). For
   now: change the grep to look for the new "skipping for now" line.

6. **Eval-check locally:** `nix flake check --no-build` (fast eval pass)
   and `nix build .#checks.x86_64-linux.hm-module-test` (full activation
   build).

## Acceptance criteria

- [ ] `programs.skillnet.configFile = "/abs/skillnet.toml"` produces
  `SKILLNET_CONFIG=/abs/skillnet.toml` in `hm-session-vars.sh`.
- [ ] `programs.skillnet.catalogConfigFile = ...` likewise sets
  `SKILLNET_CATALOG_CONFIG`.
- [ ] Sqlite branch: setting `programs.skillnet.database.path =
  "/abs/dir/db.sqlite"` causes the activation to `mkdir -p /abs/dir`.
- [ ] `programs.skillnet.skillsRoot = "/nonexistent"` no longer fails
  `home-manager switch`; warning is printed on stderr.
- [ ] Postgres branch with `database.urlFile` set: `hm-session-vars.sh`
  contains **no** `SKILLNET_DATABASE_URL` assignment, and an interactive
  shell sources `SKILLNET_DATABASE_URL` from the file at init.
- [ ] Assertion fires when `backend = "postgres"` and neither `url` nor
  `urlFile` is set.
- [ ] `nix flake check` passes (the existing
  [nix/test-hm-module.nix](../../../../nix/test-hm-module.nix) is
  minimally updated; comprehensive extension is in phase 04).

## Files likely touched

- [nix/hm-module.nix](../../../../nix/hm-module.nix) — new options,
  new sessionVariables conditionals, softened activation, urlFile shell
  init.
- [nix/test-hm-module.nix](../../../../nix/test-hm-module.nix) — relax the
  `exit 1` grep on the `skills-root` activation; do **not** add new
  positive tests here (phase 04 does that).

## Pitfalls

- **`home.sessionVariables` does not run shell logic.** Symptom: you try
  to set `SKILLNET_DATABASE_URL = "$(cat ${urlFile})"` and end up with the
  literal `$(cat /path)` in `hm-session-vars.sh`. Cause: the file is a
  static `KEY=value` dump, not a shell script. Recovery: use
  `programs.<shell>.initExtra`, not `home.sessionVariables`, for any
  command-substitution.
- **`urlFile` made required without back-compat.** Symptom: existing
  configs with only `database.url = "postgres://..."` break. Recovery:
  the assertion in step 4 accepts either source; only mutually-exclusive
  case to add to docs is "if both are set, urlFile wins" (or just refuse
  both — pick one and document).
- **Activation order with `database.path`.** Symptom: skillnet writes to
  `/abs/dir/db.sqlite` and crashes because `/abs/dir` doesn't exist.
  Cause: the `mkdir -p` lives in a separate activation block that may run
  after sourcing. Recovery: keep the `entryAfter ["writeBoundary"]`
  ordering and ensure the `mkdir -p` is in the same activation block as
  the `dataDir` one (don't fork into a second block).
- **`skillsRoot` warning suppressed in batch HM runs.** Symptom: user
  doesn't notice the missing directory. Recovery: prefix the warning with
  `>&2 echo "WARNING:"` and ensure it's visible in `home-manager switch`
  output — HM passes stderr through.
- **Lowercase `skillnet_DATA_DIR` alias.** Don't delete it in this
  phase — there's a `grep -F skillnet_DATA_DIR` in the existing test
  ([nix/test-hm-module.nix:69](../../../../nix/test-hm-module.nix#L69)).
  Add a doc-comment marking deprecation.

## Reference

- Phase 01 ([01-cli-env-vars.md](./01-cli-env-vars.md)) — env var names.
- HM activation script reference:
  https://nix-community.github.io/home-manager/options.xhtml (search for
  `home.activation`).
- Phase 04 ([04-test-non-cwd-status.md](./04-test-non-cwd-status.md))
  consumes the env-var exports introduced here.
