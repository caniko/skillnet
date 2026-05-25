# Phase 08 — Nix Home Manager module for skillnet

> **Recommended Codex model: GPT 5.5 medium**
>
> Standard Nix module work: declare options, wire the package into
> `home.packages`, optionally manage the data directory and any
> per-user config. Moderate complexity — the option surface has to
> be right the first time (option-name changes are breaking for
> downstream users), and the module needs to compose cleanly with
> other HM modules. `low` would skip the option-name review and
> ship a footgun; `high` is unnecessary for a module this small.

## Working tree

`/data/nvme0/can/Projects/skillnet`. This phase adds the HM module
inside the published crate's repo so users who add the flake input
get the module automatically. ai-skills consumes it in Phase 09.

## Goal

`programs.skillnet.enable = true;` in a Home Manager configuration
installs `skillnet` on PATH and provisions any needed runtime
state. The flake exposes the module as `hmModules.default` (and
`hmModules.skillnet` for explicit naming). Users add:

```nix
inputs.skillnet.url = "git+ssh://git@codeberg.org/caniko/skillnet.git";
# In their HM config:
imports = [ inputs.skillnet.hmModules.default ];
programs.skillnet = {
  enable = true;
  # dataDir = "${config.xdg.dataHome}/skillnet";  # default
};
```

and after `home-manager switch`, `skillnet --help` works without
nix-run ceremony.

## Why this matters now

The ai-skills `multi-phase-plan` hooks (Phase 06) call `skillnet
calibration record|verify` and prefer the binary to be on PATH. The
`nix run` fallback works but it's per-invocation overhead and
involves a registry lookup. The HM module is the clean
user-installation path; without it, every hook call eats Nix
evaluation overhead. Phase 09 wires the ai-skills flake to expose
this module so users who consume ai-skills get skillnet
transparently.

## Out of scope

- Crate publication itself — Phase 07.
- ai-skills flake input pointing at this — Phase 09.
- NixOS module (system-level rather than per-user). Could be
  added in a follow-up; HM is the priority because skillnet is a
  per-user tool.
- Per-skill configuration. The module manages skillnet's own
  install + data dir; per-skill heuristic thresholds live in the
  skill's SKILL.md (ai-skills' concern, not skillnet's).

## Plan

1. **Decide the option surface.** Conservative initial set:
   - `enable` (bool, default false) — standard HM option.
   - `package` (package, default `pkgs.skillnet` if available;
     otherwise `inputs.self.packages.${pkgs.system}.skillnet`) —
     lets advanced users swap the package.
   - `dataDir` (string, default
     `"${config.xdg.dataHome}/skillnet"`) — root data directory.
     skillnet uses `<dataDir>/<skill>/calibration.sqlite` per
     skill.
   - `extraConfig` (attrset, default `{}`) — reserved for future
     declarative config. Not used in 0.1.0 but reserves the
     namespace so adding it later isn't breaking.

   Skip these for 0.1.0 (add later if asked):
   - Auto-generated shell completions (`programs.skillnet.enableBashIntegration`
     etc.) — clap_complete supports it; document the manual
     command instead.
   - Service / timer / scheduled `calibrate` runs.

2. **Write `nix/hm-module.nix`**:

   ```nix
   { config, lib, pkgs, ... }:

   let
     cfg = config.programs.skillnet;
   in
   {
     options.programs.skillnet = {
       enable = lib.mkEnableOption "skillnet, the AI skill mirror + calibration CLI";

       package = lib.mkOption {
         type = lib.types.package;
         default = pkgs.skillnet or (throw "programs.skillnet.package not set and pkgs.skillnet unavailable; pass a package explicitly");
         description = "The skillnet package to install.";
       };

       dataDir = lib.mkOption {
         type = lib.types.str;
         default = "${config.xdg.dataHome}/skillnet";
         description = "Root data directory for skillnet; per-skill calibration databases live under <dataDir>/<skill>/.";
       };

       extraConfig = lib.mkOption {
         type = lib.types.attrs;
         default = {};
         description = "Reserved for future declarative skillnet config; unused in 0.1.0.";
       };
     };

     config = lib.mkIf cfg.enable {
       home.packages = [ cfg.package ];

       home.sessionVariables = {
         SKILLNET_DATA_DIR = cfg.dataDir;
       };

       home.activation.skillnet-data-dir = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
         $DRY_RUN_CMD mkdir -p ${lib.escapeShellArg cfg.dataDir}
       '';
     };
   }
   ```

3. **Wire the module into the flake.** In `flake.nix` (extends
   Phase 07's flake):

   ```nix
   outputs = { self, nixpkgs, ... }: let
     systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" "x86_64-darwin" ];
     forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
   in {
     # … packages, apps, devShells from Phase 07 …

     hmModules.default = import ./nix/hm-module.nix;
     hmModules.skillnet = self.hmModules.default;
   };
   ```

   Document the dual export (`default` and `skillnet`) so users can
   pick whichever import style they prefer.

4. **Make `SKILLNET_DATA_DIR` actually do something.** The skillnet
   CLI (Phases 01–04) needs to honor `$SKILLNET_DATA_DIR` as the
   data-dir override. Update `Db::default_path()` in
   `src/calibration/db.rs`:

   ```rust
   pub fn default_path() -> PathBuf {
       std::env::var("SKILLNET_DATA_DIR")
           .map(|d| PathBuf::from(d).join("multi-phase-plan").join("calibration.sqlite"))
           .unwrap_or_else(|_| {
               // Fall back to XDG_DATA_HOME, then ~/.local/share/.
               xdg_data_home()
                   .join("skillnet")
                   .join("multi-phase-plan")
                   .join("calibration.sqlite")
           })
   }
   ```

   The compiled-in `/data/nvme0/can/Projects/ai-skills` fallback
   from Phase 01 is removed in this step (it was author-machine-
   specific).

5. **Write an integration test** at `nix/test-hm-module.nix` (or
   inline in `flake.nix`'s `checks`):
   - Use `home-manager`'s test harness or a `nixos-rebuild
test`-style invocation.
   - Build an HM activation that enables `programs.skillnet`.
   - Verify the `skillnet` binary is on PATH inside the activated
     environment.
   - Verify `$SKILLNET_DATA_DIR` is set.
   - Verify `<dataDir>` exists.
   - Run `skillnet calibration migrate` inside the test
     environment; assert exit 0.

6. **Document in README.md** (the crate's README from Phase 07):
   add a "Nix Home Manager" section after the install instructions:

   ````markdown
   ### Nix Home Manager

   Add the input and import the module:

   ```nix
   inputs.skillnet.url = "git+ssh://git@codeberg.org/caniko/skillnet.git";

   # In your home-manager config:
   imports = [ inputs.skillnet.hmModules.default ];
   programs.skillnet.enable = true;
   ```
   ````

   Options:
   - `programs.skillnet.dataDir` — defaults to
     `$XDG_DATA_HOME/skillnet`.
   - `programs.skillnet.package` — override the package.

   ```

   ```

7. **Bump the crate version to `0.1.1`** (or `0.2.0` if the
   `Db::default_path` change is breaking for any existing
   downstream — for 0.1.x it isn't, since there are no downstream
   Rust users yet). Add a CHANGELOG.md entry:

   ```markdown
   ## 0.1.1 — YYYY-MM-DD

   - Add Nix Home Manager module (`hmModules.default`,
     `hmModules.skillnet`).
   - `Db::default_path` honors `$SKILLNET_DATA_DIR`, falls back to
     `$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`.
   - Remove compiled-in fallback path (was author-machine-specific
     in 0.1.0).
   ```

8. **Tag and publish** `v0.1.1` via the Phase 07 release workflow.

9. **Validate end-to-end on a clean user** (or a fresh test
   container):
   ```nix
   # ~/.config/home-manager/home.nix
   imports = [ inputs.skillnet.hmModules.default ];
   programs.skillnet.enable = true;
   ```
   ```sh
   home-manager switch
   skillnet --help                    # works without nix-run
   echo $SKILLNET_DATA_DIR            # ~/.local/share/skillnet
   ls $SKILLNET_DATA_DIR              # dir exists
   skillnet calibration migrate       # creates <dataDir>/multi-phase-plan/
   ```

## Acceptance criteria

- [ ] `nix/hm-module.nix` exists and declares `programs.skillnet`
      with `enable`, `package`, `dataDir`, `extraConfig` options
      matching Plan step 2.
- [ ] `flake.nix` exports `hmModules.default` and `hmModules.skillnet`
      (both reference the same module).
- [ ] `Db::default_path` honors `$SKILLNET_DATA_DIR` and falls back
      to `$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`;
      the compiled-in author-machine fallback is removed.
- [ ] A `nix flake check` exercise activates the HM module in a
      test environment and confirms `skillnet --help` works and the
      data dir is provisioned.
- [ ] README.md has a Nix Home Manager section with the import
      example and the option list.
- [ ] CHANGELOG.md has a `0.1.1` entry covering the module +
      data-path changes.
- [ ] `v0.1.1` is tagged, CI passes, the crate is published, and
      `cargo install skillnet@0.1.1` works on a clean machine.

## Files likely touched

- `nix/hm-module.nix` (new)
- `flake.nix` (+ `hmModules` outputs)
- `src/calibration/db.rs` (env-var-aware `default_path`, remove
  hardcoded fallback)
- `tests/calibration_db.rs` (update if it asserted the old default
  path)
- `README.md` (+ Nix HM section)
- `CHANGELOG.md` (+ 0.1.1 entry)
- `Cargo.toml` (version bump)
- Optional: `nix/test-hm-module.nix` if the test harness is split
  out.

## Pitfalls

- **Option names are part of the public surface.** Changing
  `dataDir` to `dataDirectory` later is breaking for every user
  who imports the module. Get the names right now; lean toward
  HM conventions (`enable`, `package`, `dataDir` are all standard).
- **`pkgs.skillnet` may not exist in nixpkgs yet.** The module's
  default reads `pkgs.skillnet or (throw …)`. Users who want the
  default to work without passing `package` explicitly need to
  either install skillnet into their `pkgs` via overlay (cleanest)
  or pass `package = inputs.skillnet.packages.${pkgs.system}.skillnet`.
  Document both paths in the README; the `or throw` is friendly
  about diagnosing the missing case.
- **`xdg.dataHome` default.** This is `${config.home.homeDirectory}/.local/share`
  unless `xdg.enable = true;` is set, but `config.xdg.dataHome`
  exists either way. Use it; don't hardcode `~/.local/share`.
- **`home.activation` ordering.** The `entryAfter [ "writeBoundary" ]`
  ensures the data dir is created after HM has finished writing
  symlinks. Without it, the activation script can run too early
  in some HM versions.
- **`SKILLNET_DATA_DIR` interaction with HM's session variables.**
  `home.sessionVariables` requires the user to source their shell
  profile (`zsh`/`bash`) for the variable to appear. Document this
  if it's a footgun; most users running `home-manager switch`
  immediately followed by `exec $SHELL` will be fine.
- **Don't put `programs.skillnet` config that belongs in the skill
  itself.** Heuristic thresholds, sidecar format, etc., are
  ai-skills concerns. The HM module manages install + data dir,
  nothing else. Resist scope creep.
- **Test in a fresh container or VM, not on your dev machine.**
  Your dev machine has `$SKILLNET_DATA_DIR` from the previous
  session; you won't notice if the activation doesn't set it.
- **macOS Darwin systems.** Verify the module works on Darwin too
  (most HM users are Linux but Darwin is supported by HM). The
  `home.activation` runs on both; `xdg.dataHome` resolves
  correctly on Darwin.

## Reference

- Parent plan: `docs/planning/calibration-loop/README.md`.
- Crate publication (delivered binary): `07-skillnet-crate-publication.md`.
- Phase consuming this module in ai-skills:
  `09-ai-skills-consumption.md`.
- Phase calling the installed `skillnet` from skill hooks:
  `06-base-skill-hooks-calibrate-mode.md`.
- Home Manager options reference: <https://nix-community.github.io/home-manager/options.xhtml>.
- HM module conventions: <https://nix-community.github.io/home-manager/index.xhtml#sec-writing-modules>.
