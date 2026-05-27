# Phase 04 — HM module additions

> **Recommended Codex model: GPT 5.5 medium**
>
> Three new Nix options plus a rewritten activation script that derives
> its flag set from those options, plus snapshot tests. The Nix module
> shape is well-known (it follows existing
> `programs.skillnet.hooks.enable` patterns). The activation script
> rewrite collapses two `skillnet sync` calls into one and adds the
> `failOnConflict` conditional, which needs careful shell-quoting and
> conditional emission of `|| true`. Moderate complexity from the
> snapshot test scaffolding more than from the Nix code itself.

## Working tree

`/data/nvme0/can/Projects/skillnet`. **Must start after Phase 02 has
landed on `main`** because the activation script references
`skillnet sync --apply-promote` and `--no-promote`, which only exist
after Phase 02. Can run in parallel with Phase 03 (no file overlap).

## Goal

`programs.skillnet.activation.{promote, failOnConflict, allowDelete}`
options exist. The activation script collapses the existing two-call
pair (`view sync --all --allow-delete && project sync --all
--allow-delete || true`) into a single `skillnet sync ...` invocation
with flags derived from the new options. Default behaviour for
existing users is preserved: no promotion, optional failure on
conflict (defaulting to true, so HM switches surface drift).

## Why this matters now

Once Phase 02 ships promotion-on-newer as a default-dry-run behaviour
with exit-code 2, the existing HM activation script
([hm-module.nix:324-325](../../../nix/hm-module.nix#L324-L325)) would
return non-zero on any host with view drift, breaking
`home-manager switch`. The masking `|| true` on the project-sync
line currently hides that, but the design (§ 8) is explicit:
activation must opt in to either "loud activation that can wedge
home-manager switch" or "quiet activation that needs skillnet doctor
for visibility". This phase makes the choice configurable.

The user's specific ask — "definable by the NM" — closes here. After
this phase, the user can declare the entire config + activation
behaviour in their Home Manager configuration.

## Out of scope

- The Rust CLI's HM-managed config detection
  ([design § 7](../two-way-sync-and-config-centralisation-research.md#7-hm-managed-config-detection)).
  That is a separate small change in `src/commands/project.rs` and
  belongs here only because it relates to HM ownership. Add it as
  part of this phase since it is closely coupled and small (see
  step 5).
- NixOS module (system-wide). Only the HM module exists today; a
  NixOS wrapper is future work not covered by this plan.
- Updating the user's actual canix HM configuration to declare
  `programs.skillnet.settings`. That is the user's environment work,
  outside this repo. Phase 05's quickstart will show the example.
- Migrating the live ai-skills `skillnet.toml` / `skillnet.catalog.toml`
  into the Nix expression. The user does that manually after the
  release ships.

## Plan

1. **Read inputs.** Open
   [design § 8 HM module additions](../two-way-sync-and-config-centralisation-research.md#8-hm-module-additions)
   and [§ 7 HM-managed config detection](../two-way-sync-and-config-centralisation-research.md#7-hm-managed-config-detection).
   Open [nix/hm-module.nix](../../../nix/hm-module.nix) and re-read
   the activation script (lines 311-327) plus the existing option
   declarations (e.g., `hooks.enable` at lines 173-193 for the
   nested-attrs pattern). Open
   [nix/test-hm-module.nix](../../../nix/test-hm-module.nix) and
   review its snapshot-test conventions before adding new
   assertions.

2. **Add the three new options.** In `nix/hm-module.nix`, add an
   `activation` nested attribute set under `programs.skillnet`,
   alongside the existing `hooks` attribute set:

   ```nix
   activation = {
     promote = lib.mkOption {
       type = lib.types.bool;
       default = false;
       description = ''
         Whether `home-manager switch` runs `skillnet sync
         --apply-promote` (true) or `skillnet sync --no-promote`
         (false). Set to true only on the host that owns the
         canonical skill store; consumer-only hosts must leave
         it false to avoid silently mutating canonical from a
         routine switch.
       '';
     };

     failOnConflict = lib.mkOption {
       type = lib.types.bool;
       default = true;
       description = ''
         Whether a non-zero exit from `skillnet sync` during
         activation fails the `home-manager switch`. Default
         true surfaces drift loudly; set false to restore the
         pre-0.6.0 silent-on-conflict behaviour.
       '';
     };

     allowDelete = lib.mkOption {
       type = lib.types.bool;
       default = true;
       description = ''
         Whether activation passes --allow-delete to skillnet
         sync. Existing default; broken out so a consumer-only
         host can disable it without rewriting the activation
         script.
       '';
     };
   };
   ```

   Place the block right after the `hooks` attrset (or in
   alphabetical order if the existing file does that). Keep the
   `description` strings exactly as above — they ship in the HM
   module's generated docs and the design dossier specifies them.

3. **Rewrite the activation script.** Replace the existing
   `home.activation.skillnet-views` block (lines 311-327) with:

   ```nix
   home.activation.skillnet-views = lib.hm.dag.entryAfter ["writeBoundary" "skillnet-skills-root"] ''
     if [ -z "''${SKILLNET_MIRROR_ROOT-}" ]; then
       mirror=${lib.escapeShellArg (
         if cfg.mirrorRoot != null
         then cfg.mirrorRoot
         else ""
       )}
     else
       mirror="$SKILLNET_MIRROR_ROOT"
     fi
     if [ -z "$mirror" ] || [ ! -d "$mirror/global" ]; then
       echo "WARNING: skillnet: mirror not found at $mirror; skipping sync" >&2
     else
       $DRY_RUN_CMD ${cfg.package}/bin/skillnet sync \
         ${lib.optionalString cfg.activation.promote "--apply-promote"} \
         ${lib.optionalString (!cfg.activation.promote) "--no-promote"} \
         ${lib.optionalString cfg.activation.allowDelete "--allow-delete"}${lib.optionalString (!cfg.activation.failOnConflict) " || true"}
     fi
   '';
   ```

   Key details:
   - Exactly one of `--apply-promote` and `--no-promote` is emitted
     per the boolean.
   - `|| true` is suffixed only when `failOnConflict == false`.
     Pay attention to whitespace before the `||` — the `lib.optionalString`
     emits its content directly, so the leading space is part of the
     string.
   - The old block called `view sync --all --allow-delete` then
     `project sync --all --allow-delete || true`. The new single
     `skillnet sync` call covers both because the top-level command
     already chains them (per Phase 02's
     `commands::sync::run_no_promote` and `run_with_promotion`).
     This is the single-command consolidation the user asked for.

4. **Add HM-managed config assertion.** The user could declare
   `programs.skillnet.settings` (XDG-managed) *and* still have a
   legacy cwd config sitting at `ai-skills/`. After their
   activation runs, both files would coexist; the CLI prefers
   XDG (rank 3 beats rank 4). The deprecation warning still fires.
   This is fine — the user runs `skillnet config migrate` to
   resolve.

   No HM-side assertion is needed for this case. **Skip step 4 in
   the Nix file**; the work belongs at step 5 in the Rust file.

5. **Add HM-managed config detection in Rust.** This is the design
   § 7 work. In `src/commands/project.rs`, at the start of
   `project_add` and `project_remove`, before any other check:

   ```rust
   if config_is_hm_managed(&ctx.config_path) {
       bail!(hm_managed_error_message(&ctx.config_path, "skillnet.toml"));
   }
   ```

   Add a small helper module or put it inline. The check:

   ```rust
   pub fn config_is_hm_managed(path: &Utf8Path) -> bool {
       path.starts_with("/nix/store/")
   }

   fn hm_managed_error_message(path: &Utf8Path, file_name: &str) -> String {
       format!(
           "{file_name} at {path} is managed by Home Manager (read-only).\n\
            hint: edit programs.skillnet.settings in your Home Manager configuration,\n\
                  then run `home-manager switch`."
       )
   }
   ```

   For `skillnet.catalog.toml` writes (none today, but be ready —
   sub-01 might have added one), the symmetric check goes wherever
   the write happens. Today, only `project add` / `project remove`
   write the config; that is the only call site to gate.

   The check covers `/nix/store/.../skillnet.toml` paths. The
   resolved path from XDG (rank 3) is normally
   `~/.config/skillnet/skillnet.toml`; if the user's HM module
   wrote it via `xdg.configFile."skillnet/skillnet.toml".source =
   ...`, that file is a symlink to a `/nix/store/...` path. **The
   check should resolve symlinks first** — `path.canonicalize()`
   returns the underlying store path. Use:

   ```rust
   let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
   Utf8PathBuf::from_path_buf(canonical)
       .map(|p| p.starts_with("/nix/store/"))
       .unwrap_or(false)
   ```

   Handle the case where canonicalization fails (config does not
   yet exist) — assume not HM-managed in that case.

6. **Tests in the HM-module test harness.** Extend
   `nix/test-hm-module.nix` with snapshot assertions on the
   activation script. The exact harness shape depends on what
   `nix/test-hm-module.nix` already does; the typical pattern is:

   - Evaluate `programs.skillnet.activation.promote = true` and
     assert the activation script contains `"--apply-promote"`
     and does not contain `"--no-promote"`.
   - Evaluate `promote = false` and assert the inverse.
   - Evaluate `failOnConflict = false` and assert the activation
     script ends with `"|| true"`.
   - Evaluate `failOnConflict = true` and assert it does **not**
     end with `"|| true"`.
   - Evaluate `allowDelete = false` and assert `"--allow-delete"`
     is absent.

   Read the existing test file before designing assertions —
   match its style.

7. **Rust tests for HM-managed config detection.** Add to
   `tests/cli.rs`:

   - `project_add_refuses_when_config_under_nix_store`: set up a
     fixture where `skillnet.toml` is a symlink to a path under a
     tempdir that *mimics* `/nix/store/` (use a tempdir whose
     path begins with `/nix/store/` — this requires running tests
     with sufficient privileges, or mocking the check). Easier
     approach: refactor `config_is_hm_managed` to take a "store
     prefix" parameter for testability, and expose a test seam.
     Skip this test entirely if the refactor adds significant
     complexity; the assertion is small and exercised by
     integration with the actual HM module.
   - At minimum, unit-test `config_is_hm_managed` directly with
     `"/nix/store/xyz/skillnet.toml"` → `true` and
     `"/home/user/.config/skillnet/skillnet.toml"` → `false`.

8. **Run local checks.** `cargo fmt`, `cargo clippy --all-targets
   -- -D warnings`, `cargo test --workspace`. For the Nix side:
   `nix flake check` (it will run the HM module tests).

9. **Commit.** One commit, message:
   `feat: HM activation toggles and HM-managed config detection`

## Acceptance criteria

- [ ] `nix/hm-module.nix` declares
      `programs.skillnet.activation.promote`,
      `programs.skillnet.activation.failOnConflict`, and
      `programs.skillnet.activation.allowDelete` with the
      defaults and descriptions specified in step 2.
- [ ] The activation script is the single rewritten block from
      step 3. It contains exactly one `skillnet sync` invocation,
      not two.
- [ ] `nix flake check` passes.
- [ ] HM-module snapshot tests cover the 4 boolean axis
      combinations enumerated in step 6.
- [ ] `config_is_hm_managed("/nix/store/abc/skillnet.toml")` is
      true; `config_is_hm_managed("/home/x/.config/skillnet/skillnet.toml")`
      is false. Unit test or integration test verifies both.
- [ ] `skillnet project add foo /tmp/foo` against a config whose
      canonicalized path begins with `/nix/store/` exits non-zero
      with the design § 7 error message.
- [ ] `cargo test --workspace`, `cargo clippy --all-targets -- -D
      warnings`, `cargo fmt --check` clean.
- [ ] `git log -1` shows the single phase commit.

## Files likely touched

- `nix/hm-module.nix` — new options + rewritten activation script.
- `nix/test-hm-module.nix` — new snapshot assertions.
- `src/commands/project.rs` — HM-managed config gate at the start
  of `project_add` and `project_remove`.
- `src/config.rs` — new `config_is_hm_managed` helper (and unit
  test in the existing `#[cfg(test)] mod tests`).
- `tests/cli.rs` — optional integration test for the gate.

## Pitfalls

- **T1: `mkIf cfg.enable` cascade.** The new options live under
  `programs.skillnet`. The whole module is gated by `cfg.enable`
  via `lib.mkIf cfg.enable`. The new options should *not* be
  inside `mkIf`; they need to be declared unconditionally so users
  can set them before enabling. Read the existing `database.url`
  pattern — it is declared outside `mkIf` and only consumed inside.
  Match the pattern.
- **T2: `lib.optionalString` whitespace.** The activation script
  template uses backslash line continuations. Each `lib.optionalString`
  emits its content directly without leading whitespace. The
  `|| true` case needs a leading space inside the
  `optionalString`, not before it. Test by running `nix-instantiate
  --eval` on the activation block and inspecting the rendered
  string.
- **T3: `home.activation` ordering.** The block runs after
  `skillnet-skills-root` per the existing
  `entryAfter ["writeBoundary" "skillnet-skills-root"]`. Keep
  that. Do not introduce a new ordering — other activation entries
  may depend on this one.
- **T4: `canonicalize` on missing file.** If
  `~/.config/skillnet/skillnet.toml` does not yet exist (fresh
  install, never run skillnet), `canonicalize` returns `ENOENT`.
  The fallback per step 5 returns the input path unmodified, which
  starts with `~/.config/...` not `/nix/store/...`, so
  `is_hm_managed` returns false. Correct.
- **T5: `path.starts_with("/nix/store/")` is a string check on a
  `Utf8Path`.** `Utf8Path::starts_with` matches *path components*
  not byte prefixes — `"/nix/store/abc"`.starts_with("/nix")` is
  true, but `"/nix/store/abc"`.starts_with("/nix/store/")` requires
  the trailing slash to align with a component boundary. Safer:
  `path.components().take(2).collect::<Vec<_>>()` against
  `[Component::RootDir, Component::Normal("nix"), Component::Normal("store")]`.
  Or simpler: `path.as_str().starts_with("/nix/store/")`. The
  string-prefix check on `Utf8Path::as_str()` is byte-exact and
  matches design § 7 verbatim.
- **T6: Test harness path mismatch.** The HM-module tests run
  in a hermetic Nix sandbox. They cannot reach into `$HOME` or
  the real `/nix/store/`. Assertions on the activation script
  content are pure string checks against the rendered Nix
  expression, not against runtime behaviour. Keep them that way.
- **T7: Old activation behaviour changes silently.** Users
  upgrading from `0.5.x` get a new activation script that:
  - Calls `skillnet sync` once instead of `view sync` + `project
    sync`.
  - Defaults to `--no-promote` (was: no equivalent — `0.5.x`
    sync errored on non-symlink).
  - Defaults to `failOnConflict = true` (was: silently masked
    project-sync errors with `|| true`).
  This is a behaviour change; document in Phase 05's CHANGELOG
  entry. Recovery for users surprised by activation failures:
  `programs.skillnet.activation.failOnConflict = false` restores
  the masked behaviour.

## Reference

- Design dossier:
  - [§ 7 HM-managed config detection](../two-way-sync-and-config-centralisation-research.md#7-hm-managed-config-detection)
  - [§ 8 HM module additions](../two-way-sync-and-config-centralisation-research.md#8-hm-module-additions)
- Existing module:
  - [nix/hm-module.nix](../../../nix/hm-module.nix) full file;
    activation script at lines 311-327.
  - [nix/test-hm-module.nix](../../../nix/test-hm-module.nix) —
    read first to match style.
- Rust call sites to gate:
  - [src/commands/project.rs](../../../src/commands/project.rs) —
    `project_add` (lines 22-48), `project_remove` (lines 50-77).
