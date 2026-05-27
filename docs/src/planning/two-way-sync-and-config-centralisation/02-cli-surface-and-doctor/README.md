# Phase 02 — CLI surface and doctor wiring

> **Recommended Codex model for merge/orchestration: GPT 5.5 medium**
>
> Two disjoint sub-layers consume the comparator outputs Phase 01 ships.
> Sub-01 wires the new flags, exit-code semantics, status JSON additions,
> and mutual-exclusion checks across the existing CLI plumbing. Sub-02
> adds the `NonSymlink` classification + severity matrix to
> `skillnet doctor`. They touch disjoint files and have independent
> acceptance subcriteria, so they fan out cleanly. The merge is small
> (two distinct file sets, no shared edits) and 5.5 medium is enough
> for orchestrating that merge plus running the integrated test suite.

## Sub-layers

| # | Slug | Model | Touches | Sub-layer file |
|---|------|-------|---------|----------------|
| 01 | cli-surface | 5.5 medium | `src/cli/args.rs`, `src/cli/mod.rs`, `src/commands/view.rs`, `src/commands/project.rs`, `src/commands/status.rs`, `tests/cli.rs` | [sub-01-cli-surface.md](./sub-01-cli-surface.md) |
| 02 | doctor-wiring | 5.5 medium | `src/commands/doctor.rs`, `tests/doctor.rs` (new) | [sub-02-doctor-wiring.md](./sub-02-doctor-wiring.md) |

Both sub-layers depend on Phase 01 landing first. Sub-layers within this
phase are independent of each other and can be dispatched to two
concurrent fresh Codex sessions; merge after both report green.

## Goal (phase-level)

`skillnet sync` and `skillnet doctor` expose the promotion behaviour
end-to-end. Specifically:

- `skillnet sync` accepts `--apply-promote`, `--no-promote`, `--prefer
  view|canonical`, `--adopt-new`, and the existing `--force` /
  `--allow-delete`. Mutual-exclusion combos reject at parse time.
- Default `skillnet sync` against a `ViewNewer` fixture prints would-
  promote lines, leaves canonical untouched, and exits `2`.
- `skillnet sync --apply-promote` resolves the fixture cleanly.
- `skillnet sync --no-promote` errors on non-symlink entries (parity
  with `0.5.1`).
- `skillnet sync --dry-run` collapses code `2` to code `0`.
- `skillnet status --format json` rows expose `would_promote` and
  `needs_tie_break` counts and per-entry mtime/sha fields when the
  entry is a `NonSymlink`.
- `skillnet doctor` classifies `NonSymlink` entries by comparator
  outcome and reports the design § 9 severity matrix.

## Why this matters now

Phase 01 shipped the library primitives but exposed no user-visible
surface. Without this phase, `skillnet sync` still rejects non-symlink
view entries with the same error as `0.5.1` and `skillnet doctor`
still reports `NonSymlink` as a flat error with no distinction
between safe-promote, destructive-demote, and tie-break-required
cases.

The originating user ask (a single CLI command that resolves all paths
from configuration and does the right thing on newer-view promotion)
becomes real here. The HM activation script in Phase 04 cannot reference
`--apply-promote` / `--no-promote` until they exist; the docs in
Phase 05 cannot document the new flag table until it is real.

## Out of scope

- Any change to `src/view.rs` library code beyond `pub use` shifts.
  Phase 01 ships the library; this phase consumes it.
- Any change to `src/commands/context.rs`. Phase 01 ships
  `ensure_target_clean`; this phase calls it.
- `skillnet config migrate` and the legacy-cwd deprecation warning.
  Phase 03 owns them.
- HM module additions. Phase 04 owns them.
- CHANGELOG, `commands.md`, and migration docs. Phase 05 owns them.
- Snapshot test for `skillnet sync --help`. Phase 05 adds it because
  that is the one phase guaranteed to see the final flag set.

## Merge plan

The user dispatches sub-01 and sub-02 to two fresh Codex sessions in
parallel (or sequentially if preferred). Each sub-layer commits its
work on its own and runs `cargo test --workspace` + `cargo clippy
--all-targets -- -D warnings` + `cargo fmt --check` locally before
reporting done.

Merge order:

1. Whichever sub-layer finishes first commits to `main` directly.
2. The second sub-layer rebases its branch onto `main` (or
   incorporates the first's changes if it was edited on `main` too).
   Because the sub-layers touch disjoint files, the rebase is
   conflict-free.
3. Run the integrated check from this phase's acceptance criteria
   list (the `skillnet sync` and `skillnet doctor` end-to-end test
   in `tests/cli.rs`) against the merged tree.

If the user dispatches both sub-layers to the same branch
sequentially, the merge step collapses to a no-op — just run the
integrated check at the end.

## Phase-level acceptance criteria

These are verified after both sub-layers land. Per-sub-layer
acceptance criteria live in each sub-layer file.

- [ ] `skillnet sync --help` lists `--apply-promote`, `--no-promote`,
      `--force`, `--prefer <view|canonical>`, `--adopt-new`,
      `--allow-delete` in that order. Existing global flags
      (`--dry-run`, `--allow-dirty-destination`, `--config`,
      `--catalog-config`, `--mirror-root`, `--database-url`) still
      appear in the global-flags section.
- [ ] `skillnet sync --apply-promote --no-promote` rejects at parse
      time with a clap error mentioning both flags.
- [ ] `skillnet sync --force --no-promote` rejects at parse time
      with a clap error.
- [ ] `skillnet sync` against a fixture where one global view entry
      is a directory with content newer than canonical exits `2`,
      prints a `would promote <path> -> <canonical> (view_mtime=...,
      canonical_mtime=...)` line, and leaves canonical untouched.
- [ ] The same fixture under `skillnet sync --apply-promote` exits
      `0`, promotes the entry (canonical content equals former view
      content; canonical mtime equals former view mtime), and
      demotes the view entry to a symlink. Rerun returns code `0`
      with no further changes.
- [ ] `skillnet sync --no-promote` against the same fixture exits
      non-zero with the existing `0.5.1` error message ("`{link}`
      exists and is not a symlink; pass --force to replace it").
- [ ] `skillnet sync --dry-run` against the `ViewNewer` fixture
      exits `0` and prints would-promote lines.
- [ ] `skillnet sync` against a fixture where the project repo is
      dirty (independent of `mirror_root`) refuses to promote and
      names the project path in the error.
      `--allow-dirty-destination` bypasses it.
- [ ] `skillnet status --all --format json` against a fixture with
      mixed drift kinds emits rows with non-null `would_promote`,
      `needs_tie_break` integer fields, and per-`DriftEntry`
      `view_mtime_nanos` / `canonical_mtime_nanos` / `view_sha` /
      `canonical_sha` populated for `NonSymlink` entries only.
- [ ] `skillnet doctor` against a `ViewNewer` fixture reports the
      entry at `Warn` severity with the design § 9 hint string
      `"next 'skillnet sync --apply-promote' will pull view → canonical
      and re-link"`.
- [ ] `skillnet doctor` against a `CanonicalNewer` fixture reports
      at `Error` severity with the design § 9 hint.
- [ ] `skillnet doctor` against an `Identical` non-symlink fixture
      reports at `Info` severity; doctor's overall exit code is `0`
      because no `Warn`/`Error` row is present.
- [ ] `cargo test --workspace`, `cargo clippy --all-targets -- -D
      warnings`, `cargo fmt --check` all clean.
- [ ] Existing CLI snapshot tests in `tests/cli.rs` that pin the
      `0.5.x` `Sync` command shape still pass *or* are updated to
      reflect the new flag set with a comment marking the upgrade.

## Reference

- Design dossier:
  - [§ 1 Sync flag surface](../../two-way-sync-and-config-centralisation-research.md#1-skillnet-sync-flag-surface)
  - [§ 3 Exit codes](../../two-way-sync-and-config-centralisation-research.md#3-exit-codes)
  - [§ 4 Status JSON additions](../../two-way-sync-and-config-centralisation-research.md#4-status-json-additions)
  - [§ 9 Doctor severity matrix](../../two-way-sync-and-config-centralisation-research.md#9-doctor-severity-matrix)
- Phase 01 outputs consumed:
  - `view::ReconcileOutcome`, `view::compare_view_entry`,
    `view::materialize_view_with_promotion`,
    `view::materialize_project_with_promotion`,
    `view::PromotionOptions`, `view::PromotionSummary`,
    `view::WouldEntry`.
  - `commands::context::Context::ensure_target_clean`.
- Existing CLI code to read first:
  - [src/cli/args.rs](../../../../src/cli/args.rs) (Cli struct,
    Command enum, the `Sync` variant, the existing tests at lines
    686-727 that pin the current default flag shape).
  - [src/cli/mod.rs](../../../../src/cli/mod.rs) (the `run()`
    function, the `Command::Sync` arm at lines 85-91).
  - [src/commands/view.rs](../../../../src/commands/view.rs) and
    [src/commands/project.rs](../../../../src/commands/project.rs).
  - [src/commands/status.rs](../../../../src/commands/status.rs).
  - [src/commands/doctor.rs](../../../../src/commands/doctor.rs).
