# Phase 05 — Docs, CHANGELOG, snapshot, release rollup

> **Recommended Codex model: GPT 5.5 medium**
>
> Five-file rewrite plus a new mdBook page plus a snapshot test
> pinning `skillnet sync --help`. The wording must accurately
> capture the new flag matrix and the `0.6.0` framing of the
> partial reversal of `0.5.0`'s "no reconcile" stance, which is the
> one decision in this phase that needs design judgement (and the
> dossier pre-makes it). The snapshot test must be written after
> the flag table is final, which is why this phase serialises after
> 02–04. Otherwise mostly mechanical docs work. 5.5 medium is the
> right tier.

## Working tree

`/data/nvme0/can/Projects/skillnet`. **Must start after Phases 02,
03, and 04 have landed on `main`.** Final convergence phase.

## Goal

The repo is release-ready for `0.6.0`:

- `docs/src/commands.md`'s `sync` section documents the full flag
  table from design § 1, the exit-code semantics from § 3, and
  cross-references doctor's classification (§ 9).
- A new `docs/src/migration/centralised-config.md` walks users
  through `skillnet config migrate` and the HM-module declarative
  pattern.
- `docs/src/migration/option-b.md` gains a one-paragraph note
  pointing at the new doc.
- `docs/src/SUMMARY.md` lists the new migration page and this
  plan set's README.
- `CHANGELOG.md` has a `## [0.6.0]` block at the top describing
  the partial reversal of `0.5.0`'s "no reconcile" stance, the
  new flag set, the centralised config story, and the HM module
  additions, in the project's existing CHANGELOG style.
- `README.md` (project root) is updated if any user-visible
  behaviour mentioned in it changed.
- A snapshot test in `tests/cli.rs` pins `skillnet sync --help`
  output so the flag table cannot drift silently.
- `Cargo.toml` version bumped to `0.6.0`.

## Why this matters now

Phase 02 ships the flag surface but no user can find it without
docs. Phase 03 ships `skillnet config migrate` but the migration
narrative lives only in the design dossier (which is a planning
artifact, not a user doc). Phase 04 ships the HM toggles but no
HM quickstart shows how to use them. The `0.6.0` cut is the single
shipping boundary — without this phase, the work lands silently
under `0.5.2-dev` and nobody migrates.

## Out of scope

- Tagging and publishing the `0.6.0` release. That is a separate
  `cargo release`-style step the user runs by hand after this
  plan completes. The version bump lands here; the actual tag +
  publish does not.
- The `0.7.0` removal of legacy-cwd discovery. Tracked in design
  § 12; lands in a separate plan.
- Migrating the user's actual ai-skills `skillnet.toml` /
  `skillnet.catalog.toml` into their HM config. Phase 05 shows
  the pattern in the quickstart; the user executes it in their
  own canix repo.
- Rewriting the `docs/src/release.md` process doc beyond a
  one-line cross-reference to the new migration page if useful.

## Plan

1. **Read inputs.** Open the entire design dossier
   [two-way-sync-and-config-centralisation-research.md](../two-way-sync-and-config-centralisation-research.md)
   one more time to ensure the docs faithfully describe the
   landed behaviour. Open
   [docs/src/commands.md](../../commands.md),
   [docs/src/migration/option-b.md](../../migration/option-b.md),
   [docs/src/SUMMARY.md](../../SUMMARY.md),
   [CHANGELOG.md](../../../CHANGELOG.md),
   [README.md](../../../README.md), and
   [Cargo.toml](../../../Cargo.toml) to know what is being
   rewritten.

   Verify Phase 02 / 03 / 04 are landed by running:

   ```sh
   skillnet sync --help
   skillnet config --help
   nix flake check
   ```

   If any of those errors out or shows an unexpected flag set,
   stop and report back — this phase depends on the predecessor
   phases.

2. **Rewrite the `sync` section in `docs/src/commands.md`.**
   Replace the existing `sync` bullet and any subsequent
   `## View And Project Commands` section text that mentions
   `sync` with content covering:

   - One-line summary: "Top-level sync that materialises every
     configured view, promoting view → canonical when a view
     entry is a real directory newer than canonical".
   - A flag table matching design § 1 exactly. Format the table
     identically to how the existing config-precedence tables
     are formatted ([commands.md:94-100](../../commands.md#L94-L100))
     for consistency.
   - The exit-code table from design § 3. Same formatting.
   - A short worked example: a fixture with a `ViewNewer` entry,
     `skillnet sync` exits `2` and prints would-promote; rerun
     with `--apply-promote` succeeds.
   - A pointer to `skillnet doctor` for pre-sync inspection and
     to the new migration doc for the centralised-config story.

3. **Create `docs/src/migration/centralised-config.md`.** New
   mdBook page covering:

   - Why centralisation matters (the legacy cwd pickup is
     deprecated; XDG is the standard).
   - The one-shot migration: `skillnet config migrate` —
     reproduce the design § 6 decision table here in user-doc
     form (not the verbatim spec — a friendly explanation).
   - The deprecation warning users will see on legacy-cwd
     pickup and the `0.7.0` removal timeline.
   - The Home Manager declarative pattern: a worked example
     of `programs.skillnet.settings = { ... }` and
     `programs.skillnet.catalogSettings = { ... }` with the
     user's actual two existing TOML files translated to Nix
     syntax. **Use only the schema fields that already exist;
     do not invent new ones.**
   - The HM activation toggles from Phase 04 — when to set
     `programs.skillnet.activation.promote = true` (owner host
     only) and `failOnConflict = false` (rarely; for transition
     periods).
   - The HM-managed config caveat from design § 7:
     `skillnet project add` / `project remove` will refuse
     against an HM-managed config; instructions to edit the Nix
     expression instead.
   - The catalog rule fallout caveat from design § 11:
     promotion changes canonical content; `skillnet catalog
     lint` may re-classify skills based on promoted frontmatter
     status.
   - A short troubleshooting section: what to do if the
     deprecation warning fires after a successful migration
     (check breadcrumb files; ensure cwd is no longer picking
     up rank 4).

   Page length: aim for the same scale as
   [docs/src/migration/option-b.md](../../migration/option-b.md)
   (~150-300 lines). The page is the user-facing
   counterpart to the design dossier; it does not replace the
   dossier, just translates the parts users need.

4. **Update `docs/src/migration/option-b.md`.** Append a short
   "Reconcile-pull and centralised config (`0.6.0`)" section at
   the end, two paragraphs:

   - Acknowledge that `0.5.0`'s "no reconcile" stance is
     partially reversed in `0.6.0`. The reversal is narrow:
     promotion happens only on user-visible non-symlink view
     entries that newer than canonical, only when the user
     opts in via `--apply-promote`, and never silently from
     HM activation by default.
   - Link to the new `centralised-config.md` page.

5. **Update `docs/src/SUMMARY.md`.** Add:

   ```markdown
   - [Migration](migration/option-b.md)
     - [Centralised config (0.6.0)](migration/centralised-config.md)
   ```

   And at the bottom (after the existing entries), add a
   "Planning" section if not already present:

   ```markdown
   - [Planning](planning/index.md)
     - [Reconcile-pull research](planning/reconcile-pull-research.md)
     - [Two-way sync and config centralisation design](planning/two-way-sync-and-config-centralisation-research.md)
     - [Two-way sync plan set](planning/two-way-sync-and-config-centralisation/README.md)
       - [01 Library primitives](planning/two-way-sync-and-config-centralisation/01-library-primitives.md)
       - [02 CLI surface and doctor](planning/two-way-sync-and-config-centralisation/02-cli-surface-and-doctor/README.md)
       - [03 Config centralisation](planning/two-way-sync-and-config-centralisation/03-config-centralisation.md)
       - [04 HM module](planning/two-way-sync-and-config-centralisation/04-hm-module.md)
       - [05 Docs release rollup](planning/two-way-sync-and-config-centralisation/05-docs-release-rollup.md)
   ```

   Create `docs/src/planning/index.md` if SUMMARY references it
   — a single-line page is fine, mdBook just needs a target.
   The planning section in SUMMARY is informational; users
   ignore it unless they care about plan history. **If a
   `Planning` section already exists in SUMMARY (from earlier
   plan work), append rows instead of duplicating the section.**

6. **Write the `CHANGELOG.md` entry.** At the top of the file,
   under any unreleased section, add:

   ```markdown
   ## [0.6.0] — <YYYY-MM-DD on the day the user tags>

   ### Added
   - `skillnet sync` promotes a view entry's content into the
     canonical store when the view entry is a real directory
     newer than canonical. Promotion is dry-run-by-default; the
     command prints `would promote ...` lines and exits 2.
     Pass `--apply-promote` to perform the promotion; pass
     `--no-promote` to keep `0.5.x` behaviour. Tie-breaks via
     `--prefer view|canonical`; promotion of view-only skills
     via `--adopt-new`.
   - `skillnet config migrate` moves `skillnet.toml` and
     `skillnet.catalog.toml` from the legacy working-directory
     pickup to `$XDG_CONFIG_HOME/skillnet/`.
   - `skillnet doctor` classifies non-symlink view entries by
     comparator outcome (Identical / ViewNewer / CanonicalNewer
     / EqualMtimeDifferentContent / BothAdvanced / AdoptCandidate)
     with hint strings pointing at the appropriate sync flag.
   - `skillnet status --format json` rows expose
     `would_promote` and `needs_tie_break` counts; per-entry
     `view_mtime_nanos`, `canonical_mtime_nanos`, `view_sha`,
     `canonical_sha` populated for non-symlink entries.
   - HM module: `programs.skillnet.activation.{promote,
     failOnConflict, allowDelete}` toggle activation behaviour.
     Default `promote = false`, `failOnConflict = true`,
     `allowDelete = true`. Consumer hosts upgrading from
     `0.5.x` see the same behaviour they had, except activation
     now fails loudly on drift (was: silently masked via
     `|| true`).
   - Per-target `--allow-dirty-destination`: the gate now
     covers every canonical write, not just `mirror_root`.

   ### Changed
   - HM activation script collapses `view sync` + `project sync`
     calls into a single `skillnet sync` invocation.
   - `skillnet project add` / `project remove` refuse to mutate
     a config managed by Home Manager (resolved path under
     `/nix/store/`). Edit `programs.skillnet.settings` in your
     HM config instead.

   ### Deprecated
   - Legacy working-directory config discovery (`./skillnet.toml`,
     `./skillnet.catalog.toml`). The CLI prints a deprecation
     warning when it falls through to this path and will remove
     it in `0.7.0`. Run `skillnet config migrate` to move
     existing configs.

   ### Notes
   - The `0.5.0` design stance "no reconcile, canonical is the
     only writer" is partially reversed. Promotion is opt-in
     per invocation (`--apply-promote`), opt-in per host via
     the HM toggle, and dry-run-by-default in every other
     setting.
   ```

   Match the existing CHANGELOG's date format and section
   structure. Read the previous `## [0.5.x]` blocks first.

7. **Update root `README.md` if needed.** Search for any
   mention of `skillnet sync`, `0.5`, or the legacy-cwd config
   path. If the README's quickstart shows `skillnet sync`,
   it should still work as documented (the no-flag case is
   unchanged for clean-symlink configurations); add a one-line
   note pointing at `docs/src/commands.md` for the full flag
   table. If the README does not mention any of these, skip.

8. **Add the snapshot test in `tests/cli.rs`.** Append:

   ```rust
   #[test]
   fn sync_help_output_matches_snapshot() {
       let help = Cli::command()
           .find_subcommand("sync")
           .expect("sync subcommand")
           .clone()
           .render_long_help()
           .to_string();
       insta::assert_snapshot!(help);
   }
   ```

   This requires `insta` as a dev-dependency. If `insta` is not
   already in `Cargo.toml` `[dev-dependencies]`, add it (current
   stable version, no special features). On first run, accept
   the snapshot via `cargo insta accept` (or `cargo insta
   review` to inspect).

   The snapshot file lives at
   `tests/snapshots/cli__sync_help_output_matches_snapshot.snap`.
   Commit the `.snap` file alongside the test.

   **Why this matters**: any future PR that adds, removes, or
   reorders a sync flag will fail this test and require an
   explicit snapshot update. That makes flag drift visible in
   code review.

9. **Bump the version.** In `Cargo.toml`, change
   `version = "0.5.1"` to `version = "0.6.0"`. Run
   `cargo update --package skillnet` to refresh `Cargo.lock`'s
   `skillnet` entry, *not* `cargo update` (which churns
   everything).

10. **Run the full check loop.**

    ```sh
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test --workspace
    cargo doc --no-deps
    nix flake check
    mdbook build docs
    ```

    Fix every error. The snapshot test requires running
    `cargo insta accept` once after the snapshot exists. The
    mdbook build catches broken links in the new doc pages and
    SUMMARY entries.

11. **Verify the whole-set acceptance criteria from the plan
    README.** This is the integrated test: a tempdir fixture
    with mixed drift, run `skillnet sync` end-to-end and
    confirm every behaviour in the plan README's
    acceptance criteria table.

12. **Commit.** One commit, message:
    `release: 0.6.0 — promotion-aware sync and centralised config`

## Acceptance criteria

- [ ] `docs/src/commands.md` `sync` section documents every
      flag from design § 1 in the same order, the exit-code
      table from § 3, a worked example, and pointers to doctor
      + the migration doc.
- [ ] `docs/src/migration/centralised-config.md` exists and
      covers: rationale, `skillnet config migrate` walk-through,
      deprecation timeline, HM declarative pattern with an
      example translation of the user's two TOML files,
      activation toggles, HM-managed config caveat, catalog
      fallout caveat, troubleshooting.
- [ ] `docs/src/migration/option-b.md` has an appended section
      acknowledging the `0.6.0` partial reversal and linking
      the new page.
- [ ] `docs/src/SUMMARY.md` lists the new migration page and
      the plan-set entries.
- [ ] `CHANGELOG.md` has a `## [0.6.0]` block matching step 6.
- [ ] `Cargo.toml` version is `0.6.0`. `Cargo.lock` reflects
      the bump.
- [ ] `tests/cli.rs` has a snapshot test for `sync --help` and
      the snapshot file is committed.
- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D
      warnings`, `cargo test --workspace`, `cargo doc --no-deps`,
      `nix flake check`, `mdbook build docs` all clean.
- [ ] Whole-set acceptance criteria from the plan README all
      verified against a fresh tempdir fixture.
- [ ] `git log -1` shows the single phase commit.

## Files likely touched

- `docs/src/commands.md` — `sync` section rewrite.
- `docs/src/migration/centralised-config.md` — **new file**.
- `docs/src/migration/option-b.md` — appended section.
- `docs/src/SUMMARY.md` — new entries.
- `docs/src/planning/index.md` — **new file**, if SUMMARY
  references it.
- `CHANGELOG.md` — new `[0.6.0]` block.
- `README.md` — possibly one-line addition.
- `Cargo.toml` — version bump.
- `Cargo.lock` — bump propagation.
- `tests/cli.rs` — snapshot test.
- `tests/snapshots/cli__sync_help_output_matches_snapshot.snap`
  — **new file**, the snapshot.

## Pitfalls

- **U1: `mdbook build` fails on broken links.** New SUMMARY
  entries that reference non-existent files break the build.
  Verify every link in SUMMARY points to an existing file
  before committing. Recovery: `mdbook build docs 2>&1 | grep
  ERROR` lists broken links.
- **U2: `insta` requires accepting the snapshot.** On first
  CI run, the snapshot test will fail because the snapshot
  file doesn't exist. Either create the snapshot locally
  (`cargo insta accept`) and commit it, or use
  `insta::assert_snapshot!` with explicit inline content
  (less ergonomic but no `.snap` file). Prefer the file-based
  approach — it surfaces diffs in code review.
- **U3: `clap`'s help text formatting changes between
  versions.** A `cargo update` that bumps `clap` could change
  the rendered help output and break the snapshot. Pin `clap`
  in `Cargo.toml` if it isn't already (it is; check the
  existing pin). Recovery: re-accept the snapshot if the diff
  is purely cosmetic and add a note to the test.
- **U4: CHANGELOG date format mismatch.** The existing
  CHANGELOG may use a different date convention. Read the
  existing entries first and match exactly. If unsure, use
  `YYYY-MM-DD`.
- **U5: HM module test in `nix flake check` may rely on a
  network fetch.** `nix flake check` evaluates the flake and
  may try to download dependencies. Ensure the user has the
  flake's inputs cached. Recovery: run `nix flake update`
  first if the cache is stale, but be aware that updates
  `flake.lock` and should be its own commit.
- **U6: `cargo doc --no-deps` may fail on the new public
  items from Phase 01.** Phase 01 added `ReconcileOutcome`,
  `PromotionOptions`, etc. as `pub`. If any are missing
  doc comments, `#![deny(missing_docs)]` (if set) will fail.
  Recovery: add a one-line `///` to every new public item.
  Check `lib.rs` for `deny(missing_docs)` first.
- **U7: Plan-set entries in SUMMARY persist after retirement.**
  When the user runs `verify` and the plan retires (via
  `retire-docs-planning`), the SUMMARY entries here will
  dangle. That is the verify-mode skill's responsibility to
  clean up, not this phase's. Just note it.
- **U8: The 0.6.0 tag is not part of this phase.** This phase
  bumps the version in `Cargo.toml` but does not run `git
  tag` or `cargo publish`. The user runs those after
  verification. Do not auto-tag.

## Reference

- Design dossier (entire document is reference here):
  [two-way-sync-and-config-centralisation-research.md](../two-way-sync-and-config-centralisation-research.md).
- Plan-set README (acceptance criteria source):
  [README.md](./README.md).
- Existing docs to model the new pages on:
  - [docs/src/commands.md](../../commands.md)
  - [docs/src/migration/option-b.md](../../migration/option-b.md)
  - [docs/src/release.md](../../release.md)
  - [CHANGELOG.md](../../../CHANGELOG.md)
