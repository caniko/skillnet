# Phase 04 — Extend HM activation test to cover non-cwd `skillnet status`

> **Recommended Codex model: GPT 5.5 medium**
>
> Nix test scripting plus a real `skillnet status` invocation against a
> generated TOML. Easy to get wrong in subtle ways (the sandbox doesn't
> have network — `status` must not require Postgres; the TOML must declare
> at least one mirror scope with a stub directory). `medium` is the right
> tier; the failure mode is "test always passes" if you don't actually cd
> away from the config.

## Working tree

Same repo: `/data/nvme0/can/Projects/skillnet`. Phases 01–03 must be in.

## Goal

[nix/test-hm-module.nix](../../../../nix/test-hm-module.nix) exercises the
full new contract: a third HM configuration that uses
`programs.skillnet.settings` (sqlite backend), runs `skillnet status` from
`/tmp` (or any directory that is **not** the config-file's parent), and
asserts that it succeeds — proving the env-var-driven config resolution
works end-to-end.

## Why this matters now

The original test only ran `skillnet --help` and `skillnet calibration
migrate`, neither of which loads `skillnet.toml`. That blind spot let the
"errors on any real command from any directory" bug ship in 0.2.0. The
extended test makes a recurrence catchable in CI.

## Out of scope

- Postgres end-to-end testing — sandboxed Nix builds can't bring up a
  Postgres instance. The Postgres branch is still covered for *eval*
  (assertion fires correctly, env vars exported), not for runtime.
- Testing every skillnet subcommand. `status` is the canonical "needs the
  config file" smoke test; one is enough.
- Network-dependent skill mirror operations. The test points `settings`
  at a stub directory created inline.

## Plan

1. **Add a third HM config** in
   [nix/test-hm-module.nix](../../../../nix/test-hm-module.nix), parallel
   to `sqliteConfig` and `postgresConfig`:

   ```nix
   declarativeConfig = mkHmConfig {
     database.backend = "sqlite";
     settings = {
       mirror_root = "${homeDirectory}/skills-mirror";
       scopes = [
         { name = "global"; sources = [ "${homeDirectory}/.claude/skills" ]; }
       ];
       # whatever minimal shape skillnet.toml requires — verify by reading
       # src/config.rs and any example fixtures under tests/fixtures/.
     };
     catalogSettings = { skills = []; };
   };
   ```

2. **Create the stub source directories** inside the `runCommand` script,
   before sourcing the activation:

   ```bash
   mkdir -p ${homeDirectory}/skills-mirror/global
   mkdir -p ${homeDirectory}/.claude/skills
   ```

3. **Run the activation** and source its session vars:

   ```bash
   ${declarativeConfig.activationPackage}/activate
   . ${declarativeConfig.activationPackage}/home-path/etc/profile.d/hm-session-vars.sh
   test -n "$SKILLNET_CONFIG"
   test -f "$SKILLNET_CONFIG"
   ```

4. **The actual non-cwd test** — this is the load-bearing step:

   ```bash
   cd /tmp
   export PATH="${declarativeConfig.activationPackage}/home-path/bin:$PATH"
   # Must succeed despite /tmp containing no skillnet.toml:
   skillnet status
   # Bonus: confirm the env-var is what's doing the work by unsetting it
   # and asserting failure:
   ( unset SKILLNET_CONFIG; ! skillnet status 2>/dev/null )
   ```

5. **Verify the `database.urlFile` postgres path eval-only.** Add a
   fourth `mkHmConfig` with `database.urlFile = "/run/secrets/pg-url";`
   and assert:

   ```bash
   # The URL must NOT appear in hm-session-vars.sh
   ! grep -F "SKILLNET_DATABASE_URL=" ${urlFileConfig.activationPackage}/home-path/etc/profile.d/hm-session-vars.sh
   # The bash init snippet must reference the urlFile:
   grep -F "/run/secrets/pg-url" ${urlFileConfig.activationPackage}/home-files/.bashrc
   ```

   (Path of the init snippet varies; grep for `SKILLNET_DATABASE_URL` in
   the activation tree to find it.)

6. **Verify `skillsRoot` warning, not failure.** Replace the existing
   `! grep -F 'configured programs.skillnet.skillsRoot does not exist'`
   assertion (which expected the `exit 1` message) with a positive grep
   for the new "skipping for now" message, and confirm the activation
   itself does **not** abort: invoke it against a non-existent
   `skillsRoot` and check `$?` is 0.

7. **Local check:** `nix build .#checks.x86_64-linux.hm-module-test`. Iterate.

## Acceptance criteria

- [ ] New `declarativeConfig` test passes: `skillnet status` from `/tmp`
  succeeds.
- [ ] Counter-test passes: unsetting `SKILLNET_CONFIG` causes `skillnet
  status` from `/tmp` to fail (proves the env var is what's wiring it).
- [ ] `urlFile` test: `SKILLNET_DATABASE_URL=` does **not** appear in
  `hm-session-vars.sh`; the bash/zsh init contains a reference to the
  urlFile path.
- [ ] `skillsRoot`-absent test: activation completes with exit 0 and the
  expected warning string is on stderr.
- [ ] `nix flake check` passes overall.
- [ ] CI (Forgejo Actions / whatever skillnet's workflow uses) runs the
  HM module test on push.

## Files likely touched

- [nix/test-hm-module.nix](../../../../nix/test-hm-module.nix) — primary.
- Possibly [flake.nix](../../../../flake.nix) if a new check has to be
  exposed.

## Pitfalls

- **`skillnet status` requires Postgres by default.** Symptom: the test
  fails with "calibration database not reachable". Cause: the precedence
  table in [docs/src/commands.md:44-53](../../../../docs/src/commands.md#L44-L53)
  says Postgres is the default when no SQLite indicator is set. Recovery:
  ensure the `settings` block explicitly opts into sqlite via
  `database.backend = "sqlite"` *both* on the Nix side (so the activation
  sets `SKILLNET_DATA_DIR`) and inside the rendered TOML (so the binary
  agrees). Easiest path: declare `[database] backend = "sqlite"` inside
  `settings` too.
- **`skillnet status` calls out to live source dirs.** Symptom: test
  fails because `${homeDirectory}/.claude/skills` doesn't exist. Cause:
  status walks scope sources. Recovery: pre-`mkdir -p` every source dir
  listed in `settings.scopes`. Keep the test's `settings` minimal — one
  scope, one source.
- **`shell init` file paths differ across HM versions.** Symptom: the
  `urlFile` grep finds nothing. Recovery: dump
  `find ${urlFileConfig.activationPackage}/home-files -type f` in the
  test temporarily to find the right path, then pin it.
- **`/tmp` is sandbox-private during `nix build`.** Symptom: `cd /tmp`
  works but you can't observe state outside. Recovery: that's fine — we
  only care that `skillnet status` succeeds from there; observations are
  reads back inside the build.
- **Test contamination across the three configs.** Symptom: env vars
  from `sqliteConfig` leak into `declarativeConfig`'s shell. Cause: the
  existing test already `unset`s a few vars; add corresponding unsets at
  the boundary between configs.
- **Forgetting the unset-and-fail counter-test.** Symptom: the positive
  case passes only because `skillnet.toml` happens to exist somewhere in
  the build dir. Recovery: the counter-test (`unset SKILLNET_CONFIG;
  ! skillnet status`) is what guarantees the env var is load-bearing —
  do not skip it.

## Reference

- Phases 01–03 — the contract being verified end-to-end.
- HM test patterns: existing
  [nix/test-hm-module.nix](../../../../nix/test-hm-module.nix) is the
  template.
- skillnet config schema: read
  [src/config.rs](../../../../src/config.rs) and any
  `tests/fixtures/*.toml` to determine the minimal valid settings block.
