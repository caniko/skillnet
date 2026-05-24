# Phase 03 — HM module: declarative `settings` / `catalogSettings`

> **Recommended Codex model: GPT 5.5 medium**
>
> The interesting work is writing two TOML files out of attrset options
> using `pkgs.formats.toml`, plumbing them into XDG config, and pointing
> phase 02's env vars at them when the user opts in. Moderate complexity:
> attrset → TOML serialisation has gotchas (lists of tables, ordering),
> and the option type has to be permissive enough for users to declare
> arbitrary skillnet schema without us mirroring it in Nix.

## Working tree

Same repo: `/data/nvme0/can/Projects/skillnet`. Phase 02 must already be
merged.

## Goal

A user can write, in their HM config:

```nix
programs.skillnet = {
  enable = true;
  settings = {
    mirror_root = "/home/can/.agents/skills-mirror";
    scopes = [ { name = "global"; sources = [ ... ]; } ];
    # … etc, whatever skillnet.toml accepts
  };
  catalogSettings = { … };
  database.urlFile = "/run/agenix/skillnet-pg-url";
};
```

…and `skillnet status` works from `/tmp` without ever touching cwd. The
generated TOML files live under `$XDG_CONFIG_HOME/skillnet/` and are
pointed at by `SKILLNET_CONFIG` / `SKILLNET_CATALOG_CONFIG` automatically.

## Why this matters now

Phase 02 lets users hand-write their TOMLs and point `configFile` /
`catalogConfigFile` at them. That's the floor. The ceiling is declarative
config: the HM module renders the TOMLs from Nix, so the user's entire
skillnet setup is reproducible from a single flake.

Without this, the existing reserved no-op attrs placeholder remains a TODO,
and users have to maintain TOMLs out-of-band.

## Out of scope

- Mirroring skillnet's TOML schema as typed Nix options. The option type
  is `attrsOf anything` — we let users pass through, with a thin warning
  in the docs that schema validation happens server-side in skillnet
  itself.
- Migrating existing users of the reserved placeholder option. It is documented
  as "unused in 0.2.0", so nothing to migrate.
- Catalog regeneration logic. We only write the TOML files; `skillnet
  catalog generate` continues to run on demand.

## Plan

1. **Replace the reserved placeholder** with two real options:

   ```nix
   settings = lib.mkOption {
     type = lib.types.nullOr (pkgs.formats.toml {}).type;
     default = null;
     example = lib.literalExpression ''
       {
         mirror_root = "/home/alice/skills-mirror";
         scopes = [ { name = "global"; sources = [ "/home/alice/.claude/skills" ]; } ];
       }
     '';
     description = ''
       Declarative content of skillnet.toml, written to
       $XDG_CONFIG_HOME/skillnet/skillnet.toml and exported via
       SKILLNET_CONFIG. Pass-through: skillnet validates the schema at
       runtime.
     '';
   };

   catalogSettings = lib.mkOption {
     type = lib.types.nullOr (pkgs.formats.toml {}).type;
     default = null;
     description = "Declarative content of skillnet.catalog.toml.";
   };
   ```

   Delete the placeholder option, which is documented as unused in 0.2.0.

2. **Render to disk via `xdg.configFile`.** When the corresponding setting
   is non-null:

   ```nix
   xdg.configFile."skillnet/skillnet.toml" = lib.mkIf (cfg.settings != null) {
     source = (pkgs.formats.toml {}).generate "skillnet.toml" cfg.settings;
   };
   xdg.configFile."skillnet/skillnet.catalog.toml" = lib.mkIf (cfg.catalogSettings != null) {
     source = (pkgs.formats.toml {}).generate "skillnet.catalog.toml" cfg.catalogSettings;
   };
   ```

3. **Auto-point the env vars** when `settings` / `catalogSettings` is set
   and the explicit `configFile` / `catalogConfigFile` from phase 02 is
   *not* set:

   ```nix
   programs.skillnet.configFile = lib.mkIf (cfg.settings != null)
     (lib.mkDefault "${config.xdg.configHome}/skillnet/skillnet.toml");
   programs.skillnet.catalogConfigFile = lib.mkIf (cfg.catalogSettings != null)
     (lib.mkDefault "${config.xdg.configHome}/skillnet/skillnet.catalog.toml");
   ```

   Use `mkDefault` so a user who explicitly sets `configFile` overrides
   this auto-wiring. The env var export from phase 02 then fires off the
   `configFile` value, which keeps the precedence chain simple:
   declarative `settings` ⇒ generated file ⇒ `configFile` value ⇒
   `SKILLNET_CONFIG` env var ⇒ binary.

4. **Document the precedence** in the option descriptions:
   - `settings` wins, generating a file under `$XDG_CONFIG_HOME`.
   - `configFile` (phase 02) takes whatever path the user gives.
   - Both null → binary falls back to cwd `./skillnet.toml`.

5. **Eval:** `nix flake check --no-build` to confirm `pkgs.formats.toml`
   round-trips the user's example. Build the activation package and
   inspect the generated `~/.config/skillnet/skillnet.toml` (this happens
   end-to-end in phase 04's extended test).

## Acceptance criteria

- [ ] `programs.skillnet.settings = { … };` causes
  `~/.config/skillnet/skillnet.toml` to exist after activation, with
  content equal to the rendered TOML.
- [ ] `SKILLNET_CONFIG` in `hm-session-vars.sh` points at that file when
  the user did not also set `configFile`.
- [ ] Explicit `configFile = "/elsewhere/skillnet.toml"` overrides the
  auto-derived path (verify via `home-manager build` and inspecting
  `hm-session-vars.sh`).
- [ ] The placeholder option is removed; users who set it see an evaluation
  error pointing them at `settings` (acceptable because the option was
  documented as unused).
- [ ] `nix flake check` passes.

## Files likely touched

- [nix/hm-module.nix](../../../../nix/hm-module.nix) — new options,
  `xdg.configFile` entries, `mkDefault` plumbing.
- (No source / test changes in this phase.)

## Pitfalls

- **`pkgs.formats.toml` ordering is unspecified.** Symptom: the file
  diff-flaps on rebuild because attrset keys serialise in different
  order. Cause: Nix attrsets are sorted alphabetically when rendered, but
  list-of-attrset ordering may surprise — confirm `scopes` (list of
  tables) serialises in declared order. Recovery: it does, by construction
  — lists preserve order in `pkgs.formats.toml`. Test by rendering twice
  and `diff`ing.
- **`(pkgs.formats.toml {}).type` rejects functions and derivations.**
  Symptom: eval error if the user puts a derivation reference into
  `settings`. Cause: TOML can't represent it. Recovery: this is the right
  behaviour — fail at eval time with a clear error.
- **`xdg.configFile` requires `xdg.enable = true` on some HM versions.**
  Symptom: file not written. Recovery: `xdg.enable` defaults to true on
  recent HM; if older versions are supported, add `xdg.enable =
  lib.mkDefault true;` when `settings != null`.
- **Removing the placeholder option breaks downstream evals.** Symptom: a
  third party who set the unused option fails to evaluate. Acceptable since
  the option was documented as a no-op, but record the removal in the
  CHANGELOG (phase 05).
- **Infinite-recursion risk with `mkDefault`-on-`configFile`.** Symptom:
  Nix complains about self-reference. Cause: writing
  `programs.skillnet.configFile = mkIf (settings != null) ...` inside the
  same `config` block that also reads `cfg.configFile`. Recovery: read
  `config.programs.skillnet.configFile` (the post-merge value) rather
  than `cfg.configFile` when exporting the env var in phase 02's block;
  this is how HM modules avoid recursion.

## Reference

- Phase 02 ([02-hm-env-and-fixes.md](./02-hm-env-and-fixes.md)) — defines
  the `configFile` / `catalogConfigFile` options this phase populates.
- HM `xdg.configFile` reference:
  https://nix-community.github.io/home-manager/options.xhtml (search
  `xdg.configFile`).
- `pkgs.formats.toml` source:
  `pkgs/pkgs-lib/formats.nix` in nixpkgs.
