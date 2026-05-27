# Phase 01 — Library primitives and canonical-side write

> **Recommended Codex model: GPT 5.5 high**
>
> Two coupled bodies of work in one phase: extending `src/view.rs` with a
> per-entry comparator over `NonSymlink` drift, and adding the per-target
> dirty-destination gate so writes into project canonicals are validated
> against that project's own git working tree. The atomicity contract
> (stage + atomic rename, mtime preservation, symlink-safe walking) is
> subtle — a smaller model can write code that builds and passes shallow
> tests but corrupts canonical content on a partial failure or pings
> mtimes back and forth on each sync. The phase is leaf-ish in the plan
> (no orchestration of other phases) but the library code itself is
> complex enough to warrant `high` reasoning.

## Working tree

`/data/nvme0/can/Projects/skillnet` (skillnet `main`, working tree must
be clean before starting; see Pitfall P1). All edits land in this repo.

## Goal

`skillnet`'s library layer exposes the comparator outcomes, the per-skill
atomic canonical-write primitive, and the per-target dirty-destination
gate that Phases 02–04 consume. No CLI surface changes yet — all new
behaviour is reachable only through library calls and existing test
fixtures. Running `cargo test --workspace` is clean at the end of the
phase; `skillnet sync` from the CLI behaves exactly as it does on
`0.5.1` today.

## Why this matters now

This is the foundation every other phase depends on:

- Phase 02 wires `--apply-promote` / `--no-promote` / `--prefer` /
  `--adopt-new` to the comparator outcomes this phase defines.
- Phase 02 wires `skillnet doctor` to classify `NonSymlink` entries by
  the same comparator.
- Phase 04 wires the HM activation script to call `skillnet sync
  --apply-promote`, which only does anything once Phase 02 + Phase 01
  are both landed.

Without this phase, every later phase has to either reinvent the
comparator or fork the library. The design dossier
[§ Final Design § 2 Comparator outcomes](../two-way-sync-and-config-centralisation-research.md#2-comparator-outcomes-and-action-matrix)
defines the surface; this phase implements it.

The originating symptom that drives the whole plan: an external tool
(typically a shell pipeline or a stray `cp -r`) replaces a view symlink
at `~/.claude/skills/<skill>/` with a real directory, the user edits
inside it for a few days, then `skillnet sync` either refuses to run
(today's behaviour) or destroys the edits with `--force` (today's
escape hatch). Promotion is the recovery path for the "view became a
silent writer" failure mode that the prior research dossier
[reconcile-pull-research.md § Today's failure modes](../reconcile-pull-research.md#L94-L106)
documents.

## Out of scope

- Any new CLI flag, subcommand, or argument parsing. Phase 02 owns
  every change to `src/cli/args.rs` and `src/cli/mod.rs`.
- Any change to `src/commands/doctor.rs`. The doctor classification
  table is Phase 02's sub-layer `sub-02-doctor-wiring`.
- Any change to `src/commands/view.rs` or `src/commands/project.rs`
  except as needed to keep them compiling. The user-facing wiring is
  Phase 02's sub-layer `sub-01-cli-surface`.
- Status JSON additions (`would_promote`, `needs_tie_break`, per-entry
  mtime/sha). Phase 02 owns the JSON shape; this phase only adds the
  data on `DriftEntry` as optional fields so Phase 02 can populate
  them.
- Three-way file-level merge. `BothAdvanced` always returns the outcome
  unresolved; the resolver lives in Phase 02 behind `--prefer`.
- Catalog rule recomputation after promotion. The design notes
  ([§ 11](../two-way-sync-and-config-centralisation-research.md#11-catalog-rule-fallout-from-promotion))
  document the implicit fallout; no code change is needed here.
- `RECONCILIATION.md` manifest revival. Not happening.

## Plan

1. **Read the relevant design sections.** Open
   [two-way-sync-and-config-centralisation-research.md](../two-way-sync-and-config-centralisation-research.md)
   and skim § 2 (comparator outcomes), § 5 (per-target dirty gate),
   § 10 (mtime-spoof mitigation; the `view_mtime > now()` downgrade
   rule lives in this phase's comparator), § 14 (out of scope).
   Also skim the prior dossier
   [reconcile-pull-research.md § Candidate Next Steps Phase A and B](../reconcile-pull-research.md#L238-L286)
   for the library shape and the deleted `write_skill_set` template.

2. **Add the comparator outcome enum.** In `src/view.rs`, add a public
   `ReconcileOutcome` enum exactly matching the design § 2:

   ```rust
   pub enum ReconcileOutcome {
       Identical,
       ViewNewer            { view_mtime: u128, canonical_mtime: u128 },
       CanonicalNewer       { view_mtime: u128, canonical_mtime: u128 },
       EqualMtimeDifferentContent { view_sha: String, canonical_sha: String, mtime: u128 },
       BothAdvanced         { view_only: Vec<Utf8PathBuf>, canonical_only: Vec<Utf8PathBuf> },
       AdoptCandidate,
   }
   ```

   Place it alongside the existing `DriftKind` and `FileDelta` types.
   Derive `Debug, Clone, PartialEq, Eq, Serialize` to match the
   conventions of the surrounding types. Keep the `view_mtime` and
   `canonical_mtime` as `u128` nanoseconds so they round-trip cleanly
   through JSON without loss.

3. **Add the comparator function.** Add a public
   `compare_view_entry(canonical_skill: &Utf8Path, view_entry:
   &Utf8Path) -> Result<ReconcileOutcome>` to `src/view.rs`. The
   function:

   - Asserts the view entry exists, is not a symlink (`fs::symlink_metadata`,
     `!metadata.file_type().is_symlink()`). If the view entry is a
     symlink, return an error — callers must filter to `NonSymlink`
     drift first.
   - If `canonical_skill` does not exist: return `AdoptCandidate`.
   - Computes `newest_mtime_nanos(view_entry)` and
     `newest_mtime_nanos(canonical_skill)` via the existing
     `fs_ops::newest_mtime_nanos`.
   - Computes `content_signature` for both sides only when needed
     (see the next bullets). The mtime comparison is cheaper; do it
     first.
   - **mtime-spoof downgrade:** if `view_mtime` is greater than
     `SystemTime::now()` (converted to nanos), downgrade the outcome
     classification to `EqualMtimeDifferentContent` and use whatever
     shas you have computed. The intent (per design § 10) is to force
     a `--prefer` decision rather than auto-promote based on a future
     mtime. Use `view_sha` and `canonical_sha` populated with the
     shas you computed; the `mtime` field gets the (possibly bogus)
     view mtime.
   - If `view_sha == canonical_sha`: return `Identical`.
   - If `view_mtime > canonical_mtime`: check for `BothAdvanced` by
     enumerating files; if canonical has any file the view does not,
     return `BothAdvanced` with the lists populated. Otherwise return
     `ViewNewer { view_mtime, canonical_mtime }`.
   - If `view_mtime < canonical_mtime`: symmetrically check
     `BothAdvanced` (view has files canonical does not). Otherwise
     return `CanonicalNewer { view_mtime, canonical_mtime }`.
   - If `view_mtime == canonical_mtime` (already known shas differ at
     this branch): return `EqualMtimeDifferentContent { view_sha,
     canonical_sha, mtime }`.

   The file-set comparison for `BothAdvanced` walks each side once
   via `WalkDir::new(...).follow_links(false)` and compares relative
   paths. Reuse `walkdir` directly; no need for a new helper. Skip
   the `.skillnet-tmp` staging directories — match the existing
   `fs_ops::content_signature` filter ("file or symlink, no
   directories"). Be careful: the file lists must be of relative
   paths (`strip_prefix`), sorted, for deterministic test fixtures.

4. **Extend `DriftEntry` with optional reconcile fields.** In
   `src/view.rs`, add four optional fields to `DriftEntry`:

   ```rust
   pub view_mtime_nanos: Option<u128>,
   pub canonical_mtime_nanos: Option<u128>,
   pub view_sha: Option<String>,
   pub canonical_sha: Option<String>,
   ```

   These remain `None` for `Missing`, `WrongTarget`, and `Stale` kinds.
   They are populated by Phase 02's status classification path; this
   phase only adds the field declarations and ensures they serialise
   to JSON as `null` when absent. Existing call sites that construct
   `DriftEntry` literals get the new fields with `.. Default::default()`
   if `Default` is derivable, otherwise an explicit `None` quartet.

5. **Add `materialize_view_with_promotion`.** This is the new entry
   point that Phase 02's CLI surface will call. Signature:

   ```rust
   pub fn materialize_view_with_promotion(
       canonical_root: &Utf8Path,
       view: &ViewTarget,
       options: PromotionOptions,
   ) -> Result<PromotionSummary>;
   ```

   Where:

   ```rust
   pub struct PromotionOptions {
       pub apply_promote: bool,   // execute ViewNewer + AdoptCandidate (if adopt_new)
       pub force_demote: bool,    // demote CanonicalNewer (destructive)
       pub prefer: Option<Preference>,  // resolve EqualMtimeDifferentContent + BothAdvanced
       pub adopt_new: bool,       // promote AdoptCandidate
       pub allow_delete: bool,    // passed through to underlying view sync
       pub relative_links: bool,  // existing knob
   }

   pub enum Preference { View, Canonical }

   pub struct PromotionSummary {
       pub view: ViewSyncSummary,           // existing counts
       pub promoted: Vec<String>,           // skill names actually promoted
       pub demoted_destructive: Vec<String>,// skill names destructively demoted via force_demote
       pub adopted: Vec<String>,
       pub would_promote: Vec<WouldEntry>,
       pub would_demote_destructive: Vec<WouldEntry>,
       pub needs_tie_break: Vec<WouldEntry>,
   }

   pub struct WouldEntry {
       pub skill: String,
       pub outcome: ReconcileOutcome,
   }
   ```

   The function:
   - Enumerates view entries via the existing `expected_skill_links`
     + view directory read.
   - For each entry that is a symlink with correct target or that is
     missing, defers to the existing `materialize_view_with_options`
     code path (so the no-conflict case is unchanged).
   - For each `NonSymlink` entry, calls `compare_view_entry`. Maps
     the outcome to an action via the matrix in design § 2:
     - `Identical` → atomic-demote-to-symlink (replace view dir with
       symlink to canonical, no canonical write).
     - `ViewNewer` with `apply_promote == true` → promote (call new
       `promote_view_to_canonical` from step 6), then demote.
     - `ViewNewer` with `apply_promote == false` → record in
       `would_promote`, no mutation.
     - `CanonicalNewer` with `force_demote == true` → atomic-demote-
       to-symlink (destroys view content).
     - `CanonicalNewer` with `force_demote == false` → record in
       `would_demote_destructive`, no mutation.
     - `EqualMtimeDifferentContent` / `BothAdvanced` with
       `prefer == Some(Preference::View)` and `apply_promote` →
       promote view → canonical, demote.
     - … with `prefer == Some(Preference::Canonical)` and
       `force_demote` → demote (destroys view edits).
     - … without resolution → record in `needs_tie_break`.
     - `AdoptCandidate` with `adopt_new && apply_promote` → promote
       (no demote — there is nothing to demote since canonical did
       not exist).
     - `AdoptCandidate` otherwise → no-op (the entry stays visible in
       status / doctor).

   The function must be deterministic and per-skill atomic: a panic
   or IO error on skill N leaves skills 0..N in their final state
   and skills N+1..end untouched. Bail with the per-skill error;
   match the existing `materialize_view_with_options` contract
   ([src/view.rs:8-14](../../../src/view.rs#L8-L14)).

6. **Add `promote_view_to_canonical`.** Internal helper called from
   `materialize_view_with_promotion`. Signature:

   ```rust
   pub fn promote_view_to_canonical(
       view_entry: &Utf8Path,
       canonical_skill: &Utf8Path,
   ) -> Result<()>;
   ```

   The function:
   - Computes the staging directory:
     `<canonical_parent>/.skillnet-tmp/<skill>-<pid>`. Use the same
     `process::id()` pattern as `view::atomic_symlink`
     ([src/view.rs:434-445](../../../src/view.rs#L434-L445)).
   - Removes any stale staging dir of the same name (best-effort,
     no error if absent).
   - Calls `fs_ops::copy_dir(view_entry, staging)`. The existing
     `copy_dir` preserves mtimes via `filetime::set_file_times`
     ([src/fs_ops.rs:114-145](../../../src/fs_ops.rs#L114-L145)),
     which is load-bearing — the canonical side ends up with the
     same mtime as the winning view so the next sync sees
     `Identical` rather than ping-ponging
     ([reconcile-pull-research.md L157-160](../reconcile-pull-research.md#L157-L160)).
   - If `canonical_skill` exists, removes it (`fs::remove_dir_all`).
     This must succeed before the rename to avoid a "rename onto
     existing dir" failure on some filesystems.
   - `fs::rename(staging, canonical_skill)`. Same-filesystem rename
     is atomic; cross-filesystem will fail loudly, which is the
     right behaviour given canonical and its parent are always on
     one filesystem.

   Wrap every step in `with_context(|| ...)` so partial failures
   produce a useful error.

7. **Generalise `ensure_destination_clean` to per-target.** In
   `src/commands/context.rs`, add:

   ```rust
   impl Context {
       pub fn ensure_target_clean(&self, target: &Utf8Path) -> Result<()>;
   }
   ```

   The existing `ensure_destination_clean` becomes a thin wrapper
   that calls `ensure_target_clean(&self.mirror_root)`. The new
   function:
   - Skips the check entirely if `self.allow_dirty_destination` is
     `true`.
   - Calls `vcs::ensure_clean(target)`, which already returns
     `Ok(())` for non-git paths
     ([src/vcs.rs:51-64](../../../src/vcs.rs#L51-L64)).
   - On failure, includes the target path in the error message so
     the user knows *which* repo is dirty.

   Keep the existing call site that uses `ensure_destination_clean`
   working unchanged. Do **not** add new call sites in this phase —
   Phase 02's CLI surface adds them when wiring promotion to
   `materialize_view_with_promotion`.

8. **Wire `ensure_target_clean` from `materialize_view_with_promotion`.**
   Actually, the better split: the library function should *not*
   call `Context` methods (libraries don't know about `Context`).
   Take a closure parameter:

   ```rust
   pub fn materialize_view_with_promotion(
       canonical_root: &Utf8Path,
       view: &ViewTarget,
       options: PromotionOptions,
       ensure_clean: impl Fn(&Utf8Path) -> Result<()>,
   ) -> Result<PromotionSummary>;
   ```

   The CLI surface in Phase 02 will pass `|target| ctx.ensure_target_clean(target)`.
   Library tests pass `|_| Ok(())` (the in-memory tempdir is not a
   git repo, so the gate would no-op anyway, but the closure shape
   keeps the library decoupled).

   Call `ensure_clean(canonical_root)` once at the start of the
   function (mirror-side gate); plus, when promotion targets a
   per-project canonical, call it on the project root path. The
   `ViewTarget` does not carry the project root — extend
   `PromotionOptions` with `project_root: Option<Utf8PathBuf>` so
   the caller (which knows whether this is global or project view)
   passes the right value. `None` skips the per-project call.

9. **Add a parallel `materialize_project_with_promotion`.** In
   `src/view.rs`, alongside the existing
   `materialize_project_with_options`. Takes a `Target` and
   `PromotionOptions`, iterates `target.views`, calls
   `materialize_view_with_promotion` on each with
   `project_root: target.project_root.clone()`, then handles the
   aggregator symlink as today. Returns a
   `ProjectPromotionSummary` analogous to `ProjectSyncSummary` but
   carrying per-view `PromotionSummary`s.

10. **Tests.** Add `tests/view_sync.rs` if it does not exist, or
    extend it. One fixture per `ReconcileOutcome` variant. Use
    `tempfile::tempdir` for canonical + view trees, populate them
    with files via `std::fs::write`, set mtimes via
    `filetime::set_file_times` to bypass test flake from `sleep`.

    Required fixtures (each is one `#[test]`):

    - `identical_demotes_to_symlink`: canonical and view have
      identical content + mtime; `materialize_view_with_promotion`
      with default `PromotionOptions` demotes view to symlink;
      `PromotionSummary.promoted` is empty.
    - `view_newer_default_records_would_promote`: view content
      differs and is newer; default options leave canonical
      untouched and append to `would_promote`.
    - `view_newer_apply_promote_pulls_and_demotes`: same fixture
      with `apply_promote: true` and `prefer: None` (none required);
      canonical is updated, view becomes symlink, mtimes match.
    - `canonical_newer_default_records_would_demote_destructive`.
    - `canonical_newer_force_demote_destroys_view_content`.
    - `equal_mtime_different_content_needs_tie_break`.
    - `equal_mtime_with_prefer_view_promotes`.
    - `equal_mtime_with_prefer_canonical_demotes`.
    - `both_advanced_default_needs_tie_break`.
    - `adopt_candidate_default_noops` (view-only skill stays
      non-symlink, status reports it).
    - `adopt_candidate_with_adopt_new_promotes`.
    - `future_mtime_downgrades_to_tie_break`: view mtime set to
      `SystemTime::now() + Duration::from_secs(3600)`; outcome
      classifies as `EqualMtimeDifferentContent` not `ViewNewer`.
    - `promote_preserves_mtimes`: after promotion, canonical's
      `newest_mtime_nanos` equals the pre-promotion view's
      `newest_mtime_nanos` (within the precision the host
      filesystem supports — match the existing `copy_dir` test
      shape).
    - `partial_failure_leaves_per_skill_atomicity`: induce a
      failure on the second of three skills by making its canonical
      parent read-only; assert the first skill promoted and the
      third was untouched.

    Plus add `tests/dirty_gate.rs`:

    - `per_project_dirty_gate_refuses_promotion`: init a tempdir
      project as a git repo, leave the working tree dirty, set
      `allow_dirty: false`, assert `materialize_view_with_promotion`
      bails with the project path in the error.
    - `per_project_dirty_gate_bypassed_with_allow_dirty`: same
      fixture with `allow_dirty: true` succeeds.
    - `non_git_target_skips_gate`: tempdir that is not a git repo,
      `allow_dirty: false`, promotion succeeds (the gate is a
      no-op for non-git paths per `vcs::ensure_clean`).

    The library tests do **not** import `Context`; they pass closures
    directly. Use `assert_cmd` only if a test needs to exercise the
    full CLI — and in this phase, no test needs to.

11. **Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
    `cargo test --workspace`.** Fix any complaints before committing.
    Pay particular attention to `clippy::redundant_clone` and
    `clippy::too_many_arguments` on the new public functions — the
    `PromotionOptions` struct is the right escape hatch for the
    argument count; do not let clippy push you toward inlining its
    fields.

12. **Commit.** One commit, message:
    `feat: promotion-aware reconcile primitives + per-target dirty gate`

## Acceptance criteria

- [ ] `src/view.rs` exports `ReconcileOutcome`, `Preference`,
      `PromotionOptions`, `PromotionSummary`, `WouldEntry`,
      `ProjectPromotionSummary`, `materialize_view_with_promotion`,
      `materialize_project_with_promotion`, `promote_view_to_canonical`,
      `compare_view_entry`. `cargo doc --no-deps` succeeds.
- [ ] `src/commands/context.rs` exports `Context::ensure_target_clean`.
      The existing `ensure_destination_clean` is preserved as a wrapper
      and its existing call sites are unchanged.
- [ ] `DriftEntry` has four new optional fields:
      `view_mtime_nanos`, `canonical_mtime_nanos`, `view_sha`,
      `canonical_sha`. `serde_json::to_string(&DriftEntry { ... })`
      on a `Missing` entry emits `null` for all four.
- [ ] All 17 new tests listed in step 10 pass.
- [ ] `cargo test --workspace`, `cargo clippy --all-targets -- -D
      warnings`, `cargo fmt --check` all clean.
- [ ] `skillnet sync` (CLI, unchanged in this phase) behaves
      identically to `0.5.1` against any fixture — non-symlink view
      entries still error without `--force`. Verifiable by running
      the existing `tests/cli.rs` suite which must remain green.
- [ ] `git log -1` shows the single phase commit with the message
      above.

## Files likely touched

- `src/view.rs` — comparator, outcome enum, promotion primitives,
  `DriftEntry` field additions.
- `src/fs_ops.rs` — no edits expected; existing `newest_mtime_nanos`,
  `content_signature`, `copy_dir` are reused. If anything needs
  tightening (e.g., adding a `WalkDir` filter to skip
  `.skillnet-tmp` directories), it goes here.
- `src/commands/context.rs` — `ensure_target_clean` addition.
- `tests/view_sync.rs` — new or extended; 14 fixtures per step 10.
- `tests/dirty_gate.rs` — new; 3 fixtures per step 10.
- `Cargo.toml` — no new dependencies; `filetime` and `walkdir` are
  already in tree.

## Pitfalls

- **P1: Phase starts with a dirty working tree.** The existing
  `Context::ensure_destination_clean` would reject the initial test
  runs against `mirror_root` if `ai-skills` is dirty. This phase
  edits the skillnet repo, not `ai-skills`, so it should not trip
  the live gate; but if you test against the live mirror by
  accident, the dirty-state check will mask real failures. Use a
  tempdir mirror in every test (the existing tests already do
  this).
- **P2: `fs::rename` onto an existing directory fails on Linux.**
  Linux's `rename(2)` errors with `ENOTEMPTY` when renaming a dir
  onto a non-empty dir. The `promote_view_to_canonical` flow must
  `remove_dir_all` the destination before the rename, not the
  reverse. Symptom: promotion test fails with `rename: Directory
  not empty`. Recovery: re-order the steps in step 6.
- **P3: Cross-filesystem rename.** If a tempdir test ends up on a
  different filesystem than its parent (rare but possible with
  `/tmp` on `tmpfs` and `cargo target` on disk), `fs::rename` fails
  with `EXDEV`. Symptom: test fails with `Invalid cross-device
  link`. Recovery: stage *inside* the canonical's parent directory
  (the design specifies `<canonical_parent>/.skillnet-tmp/<skill>`
  for exactly this reason); double-check the staging path is a
  sibling of the destination, not in `/tmp/...`.
- **P4: mtime nanosecond truncation.** Some filesystems (notably
  ext4 with old kernels) round mtimes to second precision.
  Tests that compare `view_mtime_nanos == canonical_mtime_nanos`
  after a `copy_dir` round-trip may fail on the runner's `/tmp`
  even though they pass locally. The existing `fs_ops::copy_dir`
  uses `filetime::set_file_times`, which handles the precision
  the host supports. Tests should compare with the same precision
  the host filesystem produces; use the helper pattern from
  existing tests in `tests/` rather than hand-rolling timestamps.
- **P5: `WalkDir` and symlinks.** The comparator walks both sides
  with `follow_links(false)`. If a canonical skill legitimately
  contains a symlink (e.g., a relative `references -> ../shared`),
  `content_signature` already hashes the link target as the
  symlink body. Make sure `BothAdvanced` enumeration uses
  `symlink_metadata`, not `metadata`, so a broken symlink doesn't
  cause the comparator to bail.
- **P6: `process::id()` collision in parallel test runs.** Cargo
  runs tests in threads of the same process; `process::id()` is
  identical across all of them. If two tests stage to
  `<canonical_parent>/.skillnet-tmp/<skill>-<pid>` concurrently
  with the same skill name, one wins and the other gets an
  `EEXIST`. The skill names in fixtures must be unique per test
  (or each test uses its own tempdir, which already isolates).
  Use unique skill names per test to be safe.
- **P7: `clippy::too_many_arguments` on `materialize_view_with_promotion`.**
  The closure parameter pushes the arg count above clippy's
  threshold. The struct-of-options pattern fixes this; do not
  flatten `PromotionOptions` back into positional arguments to
  silence clippy.
- **P8: Promotion summary fields leak through CLI prematurely.**
  Phase 02 owns the CLI wiring. This phase exposes the
  `PromotionSummary` shape but no CLI surface consumes it yet.
  Make sure the new exports are `pub` from `src/view.rs` but the
  existing `commands::view::sync` and `commands::project_sync`
  continue to call the *old* `materialize_view_with_options`
  function path. Symptom: Phase 02 sub-layers can't both run in
  parallel because they each have to re-do the CLI wiring this
  phase already did. Recovery: leave CLI wiring entirely to
  Phase 02.

## Reference

- Design dossier sections this phase implements:
  - [§ 2 Comparator outcomes and action matrix](../two-way-sync-and-config-centralisation-research.md#2-comparator-outcomes-and-action-matrix)
  - [§ 5 Per-target dirty-destination gate](../two-way-sync-and-config-centralisation-research.md#5-per-target-dirty-destination-gate)
  - [§ 10 mtime-spoof mitigation](../two-way-sync-and-config-centralisation-research.md#10-mtime-spoof-mitigation)
  - [§ 14 Out of scope](../two-way-sync-and-config-centralisation-research.md#14-out-of-scope-deliberately)
- Prior research dossier:
  - [reconcile-pull-research.md § Library primitives](../reconcile-pull-research.md#L238-L267)
  - [reconcile-pull-research.md § Canonical-side write](../reconcile-pull-research.md#L269-L286)
- Existing code to read before starting:
  - [src/view.rs](../../../src/view.rs) — the chokepoint
    `ensure_symlink`, `materialize_view_with_options`, `view_status`.
  - [src/fs_ops.rs](../../../src/fs_ops.rs) —
    `newest_mtime_nanos`, `content_signature`, `copy_dir`.
  - [src/commands/context.rs](../../../src/commands/context.rs) —
    existing `ensure_destination_clean`.
  - [src/vcs.rs](../../../src/vcs.rs) — `ensure_clean` for git
    paths, no-op for non-git.
