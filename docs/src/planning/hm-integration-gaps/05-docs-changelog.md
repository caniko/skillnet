# Phase 05 — Docs, CHANGELOG, SUMMARY

> **Recommended Codex model: GPT 5.5 low**
>
> Pure documentation work — describe the env vars, the new HM options, and
> the removal of the unused HM placeholder option. No design content. A small model is
> enough; the only reason to promote is if the changelog needs to summarise
> the breaking removal more carefully than a one-liner.

## Working tree

Same repo: `/data/nvme0/can/Projects/skillnet`. Phases 01–04 merged.

## Goal

Three doc surfaces reflect the new contract:

1. [docs/src/commands.md](../../../../docs/src/commands.md) documents
   `SKILLNET_CONFIG` / `SKILLNET_CATALOG_CONFIG` / `SKILLNET_MIRROR_ROOT`
   and their precedence vs `--config` / cwd default.
2. [README.md](../../../../README.md) HM-snippet example uses
   `settings = { … };` rather than telling users to maintain TOMLs out of
   band.
3. [CHANGELOG.md](../../../../CHANGELOG.md) records the new env vars, new
   HM options, the bug fixes (sqlite `database.path` now honoured,
   `skillsRoot` no longer fatal, `urlFile` for Postgres secrets), and the
   removal of the unused HM placeholder option.

Plus a SUMMARY entry for this plan.

## Why this matters now

The user-visible contract has changed. Without docs, users will keep
hitting the cwd error in the wild and won't discover the env-var fallback.
The CHANGELOG entry is required for the 0.3.0 release.

## Out of scope

- Rewriting the postgres-and-hm-module plan set (already retired-ish).
- Migration scripts for users on 0.2.0 — back-compat is preserved; the
  only breaking change is removal of the unused HM placeholder option.

## Plan

1. **commands.md env-var section.** Add a new subsection under "Command
   Surface":

   ```markdown
   ## Config file location

   `skillnet --config <path>` is the highest-precedence source. When the
   flag is omitted, the binary reads:

   | Rank | Source                          | Resolves to                |
   | ---- | ------------------------------- | -------------------------- |
   | 1    | `--config <path>`               | absolute or cwd-relative   |
   | 2    | `SKILLNET_CONFIG`               | absolute path              |
   | 3    | default                         | `./skillnet.toml`          |

   The same precedence applies to `--catalog-config` /
   `SKILLNET_CATALOG_CONFIG` and `--mirror-root` /
   `SKILLNET_MIRROR_ROOT`. The Home Manager module exports these env vars
   automatically when `programs.skillnet.settings` is declared.
   ```

2. **README.md HM example.** Replace the existing snippet (which probably
   shows `database.url` only) with:

   ```nix
   programs.skillnet = {
     enable = true;
     settings = {
       mirror_root = "/home/alice/skills-mirror";
       scopes = [ { name = "global"; sources = [ "/home/alice/.claude/skills" ]; } ];
     };
     database = {
       backend = "postgres";
       urlFile = config.age.secrets.skillnet-pg-url.path;
     };
   };
   ```

   Add a one-line note: "Without `settings`, you can still drop your own
   TOMLs and point `programs.skillnet.configFile` at them."

3. **CHANGELOG.md.** Under a new `## 0.3.0` heading:

   ```markdown
   ## 0.3.0 — Home Manager integration polish

   ### Added
   - CLI: `SKILLNET_CONFIG`, `SKILLNET_CATALOG_CONFIG`,
     `SKILLNET_MIRROR_ROOT` env-var fallbacks for the corresponding
     global flags. `skillnet status` no longer requires `./skillnet.toml`
     in cwd.
   - HM module: `programs.skillnet.settings` and `catalogSettings`
     declarative options — render skillnet.toml / skillnet.catalog.toml
     from Nix and auto-export the env vars above.
   - HM module: `programs.skillnet.configFile` / `catalogConfigFile` for
     pointing at user-managed TOMLs without rendering them from Nix.
   - HM module: `programs.skillnet.database.urlFile` — read Postgres URL
     from a file at shell init, keeping secrets out of the world-readable
     `hm-session-vars.sh`.

   ### Fixed
   - HM module: `programs.skillnet.database.path` (sqlite) is now
     honoured end-to-end — the activation creates the parent directory.
   - HM module: `programs.skillnet.skillsRoot` missing on disk now warns
     instead of aborting `home-manager switch`.

   ### Removed
   - HM module: the unused placeholder option was removed; it was a no-op in
     0.2.0 and is superseded by `settings`.
   ```

4. **docs/src/SUMMARY.md.** Add the new plan section:

   ```markdown
   - [Plan — HM integration gaps](planning/hm-integration-gaps/README.md)
     - [01 — CLI env-var hooks](planning/hm-integration-gaps/01-cli-env-vars.md)
     - [02 — HM env wiring and fixes](planning/hm-integration-gaps/02-hm-env-and-fixes.md)
     - [03 — HM declarative settings](planning/hm-integration-gaps/03-hm-declarative-config.md)
     - [04 — Test non-cwd status](planning/hm-integration-gaps/04-test-non-cwd-status.md)
     - [05 — Docs and CHANGELOG](planning/hm-integration-gaps/05-docs-changelog.md)
   ```

5. **Build the book:** `nix build .#docs` (or `mdbook build docs`) and
   confirm no broken links or untitled chapters.

## Acceptance criteria

- [ ] `mdbook build docs` (or the Nix equivalent) is clean.
- [ ] commands.md documents the three new env vars with the precedence
  table.
- [ ] README HM snippet uses `settings` and `urlFile`.
- [ ] CHANGELOG.md has a 0.3.0 entry covering Added / Fixed / Removed.
- [ ] SUMMARY.md lists the README and five phase files of this plan set.
- [ ] No reference to the removed placeholder option remains except inside the
  CHANGELOG's Removed entry.

## Files likely touched

- [docs/src/commands.md](../../../../docs/src/commands.md)
- [docs/src/SUMMARY.md](../../../../docs/src/SUMMARY.md)
- [README.md](../../../../README.md)
- [CHANGELOG.md](../../../../CHANGELOG.md)

## Pitfalls

- **CHANGELOG date drift.** Symptom: the entry says "0.3.0" but
  `Cargo.toml` still says `0.2.0`. Cause: this phase doesn't bump the
  version — release prep does. Recovery: leave the heading as "0.3.0 —
  unreleased" if cutting the release is a separate task; the
  rust-crate-release-chaperone skill handles the actual bump.
- **README example references removed options.** Symptom: the example still
  shows removed option names. Cause: stale paste. Recovery: re-render the whole
  HM block from scratch using the new option set.
- **mdBook SUMMARY indentation.** Symptom: the new plan appears at the
  top level instead of nested. Cause: inconsistent leading spaces.
  Recovery: match the indentation of the existing "Plan — Postgres + HM
  module" block exactly (two-space indent for sub-entries).

## Reference

- Phases 01–04 — the changes being documented.
- Release chaperone: `rust-crate-release-chaperone` skill (separate
  from this plan) will run when cutting 0.3.0.
