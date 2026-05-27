# Plan — Two-way sync and config centralisation

> **Recommended Codex model for plan-set orchestration: GPT 5.5 high**
>
> Five phases spanning a Rust library refactor with atomicity primitives,
> a CLI/UX surface change with new flags + exit-code semantics + JSON schema
> additions, a new subcommand for config file migration, a Nix module
> extension, and a docs/release rollup. Multiple file-level overlaps require
> sequencing judgement; an orchestrator that confuses the dependency edges
> will either serialise too aggressively (slow) or overlap phases that touch
> the same `src/cli/args.rs` (merge pain). 5.5 high holds that quality bar
> without paying for `xhigh` on a 5-phase plan that has been pre-designed.

## Scope and current state

This plan turns the design in
[two-way-sync-and-config-centralisation-research.md](../two-way-sync-and-config-centralisation-research.md)
into executable phases. It implements:

1. Promotion-on-newer for `skillnet sync` (view → canonical when a view entry
   is a real directory whose content is newer than canonical), with dry-run-
   on-conflict as the default and explicit `--apply-promote` / `--no-promote`
   / `--force` / `--prefer` / `--adopt-new` opt-ins.
2. Per-target `--allow-dirty-destination` gating, so promotion into a project
   canonical is gated by *that project's* git working tree.
3. New `skillnet config migrate` subcommand that moves
   `skillnet.toml` and `skillnet.catalog.toml` from a working-directory
   pickup to `$XDG_CONFIG_HOME/skillnet/`.
4. Legacy-cwd config discovery deprecation warning (rank 4 of the discovery
   ladder), to be removed in `0.7.0`.
5. HM module additions exposing `programs.skillnet.activation.{promote,
   failOnConflict, allowDelete}`, with activation defaulting to
   `--no-promote` so HM switches never silently mutate canonical.
6. Doctor severity classification for `NonSymlink` entries that distinguishes
   `Identical`, `ViewNewer`, `CanonicalNewer`, `EqualMtimeDifferentContent`,
   `BothAdvanced`, and `AdoptCandidate` outcomes with appropriate
   hint messages.
7. Status JSON additions exposing `would_promote` and `needs_tie_break`
   counts plus the per-`DriftEntry` mtime / sha fields.
8. Docs + CHANGELOG + a help-output snapshot test pinning the new flag table.

Current repo state (skillnet `main`, working tree clean except this plan
set + the dossier): `0.5.1` baseline with `skillnet sync` already chaining
`view sync --all` and `project sync --all` but erroring on non-symlink view
entries unless `--force` is passed.
[reconcile-pull-research.md](../reconcile-pull-research.md) is the prior
research that this design builds on.

## Phase table

| Phase | File | Model | Depends on | Blocks | Touches | Parallel with |
|-------|------|-------|------------|--------|---------|---------------|
| 01 | [01-library-primitives.md](./01-library-primitives.md) | 5.5 high | — | 02, 04 | `src/fs_ops.rs`, `src/view.rs`, `src/commands/context.rs`, new fixtures | — |
| 02 | [02-cli-surface-and-doctor/](./02-cli-surface-and-doctor/README.md) | 5.5 medium (orchestration) | 01 | 03, 04, 05 | `src/cli/args.rs`, `src/cli/mod.rs`, `src/commands/{view,project,status,doctor}.rs`, `tests/cli.rs` | sub-01 and sub-02 in parallel |
| 03 | [03-config-centralisation.md](./03-config-centralisation.md) | 5.5 medium | 02 (file-level conflict on `src/cli/args.rs`) | 05 | `src/config.rs`, `src/cli/args.rs`, `src/cli/mod.rs`, new `src/commands/config.rs`, new `tests/config_migrate.rs` | — |
| 04 | [04-hm-module.md](./04-hm-module.md) | 5.5 medium | 02 | 05 | `nix/hm-module.nix`, `nix/test-hm-module.nix` | 03 |
| 05 | [05-docs-release-rollup.md](./05-docs-release-rollup.md) | 5.5 medium | 02, 03, 04 | — | `docs/src/commands.md`, new `docs/src/migration/centralised-config.md`, `docs/src/SUMMARY.md`, `CHANGELOG.md`, `README.md`, `tests/cli.rs` (snapshot) | — |

## Parallelism layer

### Wave 0 — start of plan

- **Phase 01** (library primitives + canonical write + per-target dirty
  gate). Only one phase can start because everything else either needs the
  comparator types or the per-target gate function it introduces.

### Wave 1 — after Phase 01 lands

- **Phase 02** is the only phase that can start. It has two parallel
  sub-layers inside (`sub-01-cli-surface`, `sub-02-doctor-wiring`) — the
  user can dispatch both to fresh sessions concurrently and merge after.
  See the phase `README.md` for the merge plan.

Phases 03 and 04 *cannot* start in Wave 1:

- Phase 03 touches `src/cli/args.rs` and `src/cli/mod.rs`, which Phase 02
  also rewrites. Parallelising creates merge pain on a file that is the
  CLI's single source of truth — serialise.
- Phase 04 wires the HM activation script to call `skillnet sync
  --apply-promote` / `--no-promote`, which Phase 02 introduces. Drafting
  the Nix code earlier is possible, but the snapshot tests in
  `nix/test-hm-module.nix` cannot pass until Phase 02 ships the flags.
  Serialise.

### Wave 2 — after Phase 02 lands

- **Phase 03** and **Phase 04** are independent of each other and can run
  in parallel. Phase 03 is pure Rust (`src/commands/config.rs`,
  `src/config.rs` deprecation warning, `src/cli/args.rs` subcommand
  addition). Phase 04 is pure Nix. No file conflict.

### Wave 3 — after Phase 03 and Phase 04 land

- **Phase 05** consolidates docs, CHANGELOG framing, SUMMARY entry, and
  the `skillnet sync --help` snapshot test. The snapshot needs the final
  flag table, so it cannot land before Phase 02; the migration doc cannot
  finalise before Phase 03; the HM quickstart cannot finalise before
  Phase 04. End-of-plan rollup.

### Wave 4 — plan exhausted

All five phases landed; `cargo test`, `cargo clippy --all-targets
-- -D warnings`, `nix flake check`, `mdbook build docs` all clean.
Version bump to `0.6.0` lives in Phase 05's plan steps; the tag and
release notes are a separate `cargo release`-style hand-off and not part
of this plan.

## Whole-set acceptance criteria

- [ ] `skillnet sync` (no flags) on a fixture where every view entry is
      either a clean symlink or a `ViewNewer` non-symlink prints would-
      promote lines and exits `2`; reruns are idempotent.
- [ ] `skillnet sync --apply-promote` on the same fixture promotes view
      → canonical (preserving mtimes) and demotes to symlink in one shot.
      Rerun returns code `0`.
- [ ] `skillnet sync --no-promote` errors on the same fixture without
      mutating canonical (parity with `0.5.x` behaviour).
- [ ] `skillnet sync` against a fixture with a non-clean per-project git
      working tree refuses to promote into that project's canonical and
      reports the gate, even when `mirror_root` is clean.
      `--allow-dirty-destination` bypasses it.
- [ ] `skillnet config migrate` against a fixture with both legacy-cwd
      and XDG config files present and content-equal deletes the cwd file
      and writes a breadcrumb. With content-different, exits `1` unless
      `--force` is passed.
- [ ] Running `skillnet` from a directory that contains a legacy-cwd
      config but no XDG config emits a deprecation warning to stderr (one
      per invocation) on the discovery fallthrough path.
- [ ] `skillnet doctor` against a fixture with one `ViewNewer` and one
      `CanonicalNewer` entry classifies them at `Warn` and `Error`
      respectively with the documented hint messages.
- [ ] `skillnet status --format json` emits non-null `would_promote` and
      `needs_tie_break` fields on every row and populates per-`DriftEntry`
      mtime / sha fields when the entry is a `NonSymlink`.
- [ ] `nix flake check` evaluates the HM module with
      `programs.skillnet.activation.promote = true` and `false`, and the
      activation script snapshot matches the expected flag set for each.
- [ ] `docs/src/commands.md` documents every flag in the §1 table from
      the design dossier, in the same order. `mdbook build docs` is
      clean.
- [ ] `cargo test --workspace`, `cargo clippy --all-targets -- -D
      warnings`, and `cargo fmt --check` all pass at HEAD of the final
      phase.

## Global constraints

- The plan ships under `0.6.0`. No `0.5.x` patch release. The CHANGELOG
  framing in Phase 05 must own the partial reversal of the `0.5.0` "no
  reconcile" stance — see the design dossier
  [§ Migration and release sequencing](../two-way-sync-and-config-centralisation-research.md#12-migration-and-release-sequencing).
- Legacy-cwd config discovery (rank 4 of the discovery ladder in
  `src/config.rs`) is **not** removed in this plan. The deprecation
  warning lands in `0.6.0`; the removal is `0.7.0` work, deliberately
  out of scope.
- Per-skill atomicity is non-negotiable. No phase should introduce
  per-file three-way merge logic. `BothAdvanced` always defers to
  `--prefer view|canonical`.
- The `RECONCILIATION.md` manifest pattern from pre-`0.5.0` reconcile is
  not revived. Telemetry stays in sync summary lines, dry-run output,
  and the calibration DB (untouched here).
- `--allow-dirty-destination` keeps its name and global-flag shape. Per-
  scope variants (`--allow-dirty <scope>`) are explicitly out of scope.

## Shared-file lockstep

Two files are touched by multiple phases. Both phases below must take
explicit care:

| File | Phases | Coordination |
|------|--------|--------------|
| `src/cli/args.rs` | 02, 03 | Phase 03 starts only after Phase 02 lands. Phase 03 adds a `Config { command: ConfigCommand }` variant to the existing `Command` enum and a `ConfigCommand::Migrate { ... }` subcommand. Phase 02 owns the `Sync { ... }` variant edits. Both phases must keep `disable_help_subcommand = true` on the enum. |
| `src/cli/mod.rs` | 02, 03 | Same sequencing. Phase 02 expands the `Command::Sync` arm in `run()`; Phase 03 adds a `Command::Config` arm. Both reuse `Context::load` for read paths, but Phase 03's `Migrate` subcommand operates on raw paths and does not require a full `Context`. |

## Reference

- Design dossier (load-bearing input for every phase):
  [two-way-sync-and-config-centralisation-research.md](../two-way-sync-and-config-centralisation-research.md).
  Each phase file cross-references the dossier section it implements.
- Prior research dossier (background and primitives recovery):
  [reconcile-pull-research.md](../reconcile-pull-research.md).
- Migration source (the `0.5.0` "no reconcile" stance that 0.6.0
  partially reverses): [docs/src/migration/option-b.md](../../migration/option-b.md).
- Current command surface: [docs/src/commands.md](../../commands.md).

## Dispatch reminder

Run each phase in a fresh Codex session pointed at its markdown file. For
Phase 02, you can fan the two sub-layers out to two concurrent sessions
and merge after — see the phase `README.md` for the merge plan. Prompt
`verify` when the plan is done to audit acceptance criteria against the
repo state.
