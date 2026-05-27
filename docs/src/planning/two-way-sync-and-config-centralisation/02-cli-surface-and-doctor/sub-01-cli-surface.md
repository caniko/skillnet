# Phase 02 / Sub-layer 01 — CLI surface

> **Recommended Codex model: GPT 5.5 medium**
>
> Mechanical flag wiring + exit-code routing + JSON schema additions
> across five files. The design dossier pins every decision (flag
> names, mutual-exclusion combos, exit-code table, JSON field names),
> so no design judgement is required. The complexity sits in correctly
> mapping outcomes to actions across the existing CLI plumbing, not in
> picking what to do. 5.5 medium is the right tier: enough reasoning
> to hold the matrix without inflating cost.

## Working tree

`/data/nvme0/can/Projects/skillnet`. Phase 01 must be landed on
`main` before this sub-layer starts. This sub-layer runs in parallel
with `sub-02-doctor-wiring` — they touch disjoint files.

## Goal

`skillnet sync` exposes the full flag surface from design § 1 and
behaves per the action matrix in § 2 + exit-code table in § 3.
`skillnet status --format json` emits the new fields from § 4.
Mutual-exclusion combos reject at parse time. Doctor classification
is **not** done here — that is sub-02.

## Why this matters now

Phase 01 shipped `materialize_view_with_promotion` but nothing in the
CLI calls it. Without this sub-layer, `skillnet sync` still goes
through `materialize_view_with_options` (the old, non-promoting path),
so promotion is dead code. The user's "single CLI command" requirement
becomes false advertising until this sub-layer lands.

## Out of scope

- `src/commands/doctor.rs`. Sub-02 owns it.
- `src/view.rs` library code. Phase 01 owns it.
- `skillnet config` subcommand. Phase 03.
- Snapshot test for `--help`. Phase 05.
- Help-output wording polish. Functional CLI; copy-edit later.

## Plan

1. **Read inputs.** Open
   [design § 1](../../../two-way-sync-and-config-centralisation-research.md#1-skillnet-sync-flag-surface),
   [§ 2 action matrix](../../../two-way-sync-and-config-centralisation-research.md#2-comparator-outcomes-and-action-matrix),
   [§ 3 exit codes](../../../two-way-sync-and-config-centralisation-research.md#3-exit-codes),
   [§ 4 status JSON](../../../two-way-sync-and-config-centralisation-research.md#4-status-json-additions),
   [§ 5 per-target dirty gate](../../../two-way-sync-and-config-centralisation-research.md#5-per-target-dirty-destination-gate).

   Open [src/cli/args.rs](../../../../src/cli/args.rs) and locate the
   `Sync` variant of `Command` (lines 71-79) plus its tests
   (lines 686-727). Open [src/cli/mod.rs](../../../../src/cli/mod.rs)
   and locate the `Command::Sync` arm in `run()` (lines 85-91).
   Open [src/exit.rs](../../../../src/exit.rs) and check whether it
   already has a code-2 escape hatch (it does not — `run()` returns
   `Result` and the binary maps `Err` → code `1`).

2. **Extend `Command::Sync` in `src/cli/args.rs`.** Replace the
   existing variant with:

   ```rust
   Sync {
       #[arg(long)]
       apply_promote: bool,
       #[arg(long, conflicts_with_all = ["apply_promote", "force"])]
       no_promote: bool,
       #[arg(long)]
       force: bool,
       #[arg(long, value_enum)]
       prefer: Option<PreferenceArg>,
       #[arg(long)]
       adopt_new: bool,
       #[arg(long)]
       allow_delete: bool,
   },
   ```

   Add the `PreferenceArg` enum next to the other `ValueEnum`s in
   the file:

   ```rust
   #[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
   pub(crate) enum PreferenceArg { View, Canonical }
   ```

   The `conflicts_with_all` on `no_promote` covers both mutual-
   exclusion rules from design § 1. Clap will produce a parse-time
   error mentioning each conflicting flag.

3. **Update the existing `Sync` tests in `src/cli/args.rs`.** The
   tests at lines 686-727 pin the old flag shape; update them:

   - `sync_command_defaults_to_safe_flags` becomes
     `sync_command_defaults_record_promotion_off`. Assert all six
     new fields default to their off / `None` values:

     ```rust
     assert!(matches!(
         cli.command,
         Some(Command::Sync {
             apply_promote: false,
             no_promote: false,
             force: false,
             prefer: None,
             adopt_new: false,
             allow_delete: false,
         })
     ));
     ```

   - `sync_command_parses_mutation_flags` becomes
     `sync_command_parses_apply_promote_and_prefer`. Cover the
     happy path of every new flag in one parse.

   - Add `sync_command_rejects_apply_promote_with_no_promote` and
     `sync_command_rejects_no_promote_with_force` that use
     `Cli::try_parse_from` (not `parse_from`) and assert the error
     is non-empty with the conflicting flag names in its message.

   - `sync_command_accepts_global_dry_run_flag` stays unchanged in
     spirit but updates to the new field set.

   - `long_help_lists_sync_command` stays.

4. **Rewire the `Command::Sync` arm in `src/cli/mod.rs`.** Replace
   the existing arm (lines 85-91) with:

   ```rust
   Command::Sync {
       apply_promote,
       no_promote,
       force,
       prefer,
       adopt_new,
       allow_delete,
   } => {
       let preference = prefer.map(|p| match p {
           args::PreferenceArg::View => view::Preference::View,
           args::PreferenceArg::Canonical => view::Preference::Canonical,
       });
       let options = view::PromotionOptions {
           apply_promote,
           force_demote: force,
           prefer: preference,
           adopt_new,
           allow_delete,
           relative_links: false, // global; project sync overrides per call
       };
       let exit_code = commands::sync::run(&ctx, options, no_promote)?;
       if exit_code != 0 {
           std::process::exit(exit_code);
       }
       Ok(())
   }
   ```

   The new `commands::sync::run` function (added in the next step)
   returns the `i32` exit code so the dispatcher can route it. The
   `process::exit(2)` path is the only way to escape the existing
   `anyhow::Result<()>` plumbing without restructuring the entire
   `cli::run` return type.

   If `ctx.dry_run` is true, design § 3 mandates collapsing code 2
   to code 0. Do that in `commands::sync::run`, not here.

5. **Add `src/commands/sync.rs`.** This is the new top-level sync
   orchestrator that fans out to global view + every project view.
   Module skeleton:

   ```rust
   use anyhow::Result;
   use crate::view::{self, PromotionOptions, PromotionSummary, WouldEntry};
   use super::Context;

   pub fn run(
       ctx: &Context,
       options: PromotionOptions,
       no_promote: bool,
   ) -> Result<i32> {
       if no_promote {
           return run_no_promote(ctx, &options);
       }
       run_with_promotion(ctx, options)
   }
   ```

   `run_no_promote`: calls the existing
   `commands::view::sync(ctx, options.allow_delete, options.force_demote)`
   followed by `commands::project_sync(ctx, &[], true, options.allow_delete,
   options.force_demote)`. Returns `Ok(0)` on success and propagates errors
   as today. Exit code matches `0.5.x` behaviour. This is the
   parity escape hatch consumers and CI need.

   `run_with_promotion`: calls
   `view::materialize_view_with_promotion` once for the global
   target's single view (well, once per view on the global target —
   the global target may have multiple views like `claude` and
   `agents`), then
   `view::materialize_project_with_promotion` per configured
   project. Aggregate every `PromotionSummary` into a single
   `OverallReport`:

   ```rust
   struct OverallReport {
       totals: Totals,
       per_target: Vec<TargetReport>,
   }

   struct Totals {
       created: usize,
       updated: usize,
       unchanged: usize,
       removed: usize,
       promoted: usize,
       demoted_destructive: usize,
       adopted: usize,
       would_promote: usize,
       would_demote_destructive: usize,
       needs_tie_break: usize,
   }
   ```

   Pretty-print the report to stdout. For each `WouldEntry`:

   ```
   would promote <view-path> -> <canonical-path> (view_mtime=<ns>, canonical_mtime=<ns>)
   would destructively demote <view-path> -> <canonical-path> (view newer mtime; pass --force to discard view)
   needs tie-break <view-path> vs <canonical-path> (view_sha=<8>, canonical_sha=<8>); pass --prefer view|canonical
   ```

   Exit code derivation per design § 3:
   - If `ctx.dry_run` is true: return `Ok(0)` regardless.
   - If `totals.would_promote + totals.would_demote_destructive +
     totals.needs_tie_break > 0`: return `Ok(2)`.
   - Else: return `Ok(0)`.

6. **Wire `ensure_target_clean` into the per-target write paths.**
   The closure passed to `materialize_view_with_promotion` from
   `run_with_promotion` is:

   ```rust
   |target_path: &Utf8Path| ctx.ensure_target_clean(target_path)
   ```

   For the global view, the `project_root` field of
   `PromotionOptions` is `None`; the closure is called once on
   `ctx.mirror_root`. For project views, `project_root` is the
   project's root path; the closure is called once on
   `ctx.mirror_root` AND once on `project_root`.

7. **Register the new module.** Add `pub mod sync;` to
   [src/commands/mod.rs](../../../../src/commands/mod.rs) alongside
   the existing module declarations. Re-export `sync::run` if
   convenient.

8. **Add `would_promote` and `needs_tie_break` to status JSON.**
   In [src/commands/status.rs](../../../../src/commands/status.rs)
   (and any view/project status path that emits the
   `ViewStatusRow` / `ProjectStatusRow` structs from
   [src/commands/view.rs](../../../../src/commands/view.rs) and
   [src/commands/project.rs](../../../../src/commands/project.rs)):

   - Extend each row struct with `would_promote: usize` and
     `needs_tie_break: usize`. Both default to `0`.
   - When status walks drift entries, for each `NonSymlink` entry
     it currently produces, call
     `view::compare_view_entry(canonical, view_entry_path)`. Map
     the outcome:
     - `ViewNewer` → increment `would_promote`; populate the
       `DriftEntry` mtime fields.
     - `CanonicalNewer` → increment `would_promote` is wrong here
       — keep design semantics: this is `would_demote_destructive`
       in sync but in status JSON it folds into `would_promote` =
       0, `needs_tie_break` = 0, and the entry's severity is
       implicit in mtimes. Re-read § 4: status only counts
       `would_promote` (for `ViewNewer`) and `needs_tie_break`
       (for `EqualMtimeDifferentContent` + `BothAdvanced`). The
       `CanonicalNewer` case is visible in mtimes but not counted
       in either summary field. Match the design.
     - `EqualMtimeDifferentContent` or `BothAdvanced` →
       increment `needs_tie_break`; populate sha fields where
       available.
     - `Identical` or `AdoptCandidate` → counts stay 0; mtime/sha
       fields stay `None` (the entry classification is unchanged).

   The existing `drift_entries` count semantics are preserved.

9. **Update existing status callers.** Make sure `status::run`
   and the per-scope `view::status` / `project_status_command`
   functions populate the new fields. The plain-text output does
   not need to display them in this sub-layer (Phase 05's docs
   rewrite mentions which text-output adjustments to make, if any).
   JSON output is the load-bearing change.

10. **Tests.** Extend `tests/cli.rs` (do not create a new file):

    - `sync_default_on_view_newer_fixture_exits_2_and_does_not_mutate`:
      build a tempdir with one global view symlink replaced by a
      real directory whose mtime is set one minute in the past;
      canonical has its mtime set one hour in the past. Configure
      a tempdir skillnet config pointing at the fixture. Invoke
      `skillnet sync` via `assert_cmd`. Assert exit code is `2`,
      stdout contains `"would promote"`, and the view entry is
      still a real directory after the command.
    - `sync_apply_promote_on_view_newer_fixture_succeeds`: same
      fixture, run with `--apply-promote`. Assert exit code is
      `0`, the view entry is now a symlink, canonical content
      equals the former view content, and canonical mtime equals
      the former view mtime.
    - `sync_no_promote_on_view_newer_fixture_errors`: same fixture
      with `--no-promote`. Assert exit code is non-zero (it'll be
      `1` because the underlying error is the existing
      `"exists and is not a symlink; pass --force to replace it"`
      error). Canonical untouched.
    - `sync_dry_run_collapses_code_2_to_code_0`: same fixture with
      `--dry-run`. Assert exit code is `0` and would-promote line
      still prints.
    - `sync_per_project_dirty_gate_refuses_promotion`: tempdir
      with a project whose own git working tree is dirty
      (`git init && touch dirty && git add .`); `mirror_root`
      clean. Assert exit code is non-zero, error names the project
      path.
    - `sync_with_allow_dirty_destination_bypasses_per_project_gate`:
      same fixture with `--allow-dirty-destination`. Assert exit
      `0` or `2` (depending on whether promotion needed); the
      important thing is the per-project gate did not fire.
    - `status_json_emits_would_promote_and_tie_break_counts`:
      fixture with one `ViewNewer` and one `EqualMtimeDifferentContent`
      entry. Run `skillnet status --all --format json`. Parse the
      output with `serde_json`. Assert the global row has
      `would_promote == 1` and `needs_tie_break == 1`, and the
      relevant `DriftEntry` rows have populated mtime / sha
      fields.
    - `status_json_leaves_legacy_fields_unchanged`: fixture with
      only `Missing` and `Stale` drift. Assert
      `would_promote == 0`, `needs_tie_break == 0`, and the
      mtime/sha fields are absent or `null` in JSON.

    Use `tempfile::tempdir`, `filetime::set_file_times`, and the
    existing `assert_cmd`/`predicates` patterns. Do **not** add a
    `--help` snapshot test — that is Phase 05's responsibility.

11. **Run the local check loop.** `cargo fmt`, `cargo clippy
    --all-targets -- -D warnings`, `cargo test --workspace`.

12. **Commit.** One commit, message:
    `feat: skillnet sync promotion-aware flag surface and status JSON`

## Acceptance criteria

- [ ] `skillnet sync --help` (run from the local debug build)
      lists `--apply-promote`, `--no-promote`, `--force`,
      `--prefer <view|canonical>`, `--adopt-new`, `--allow-delete`.
- [ ] `skillnet sync --apply-promote --no-promote` rejects at parse
      time with a non-zero exit and an error message mentioning
      both flags. Same for `--force --no-promote`.
- [ ] The 7 new `tests/cli.rs` cases listed in step 10 pass.
- [ ] Existing `Sync` parse tests are updated (not deleted) and
      pass.
- [ ] `cargo test --workspace`, `cargo clippy --all-targets -- -D
      warnings`, `cargo fmt --check` clean.
- [ ] `git log -1` shows the single sub-layer commit.

## Files likely touched

- `src/cli/args.rs` — `Sync` variant rewrite, `PreferenceArg`
  enum, test rewrites.
- `src/cli/mod.rs` — `Command::Sync` arm rewrite.
- `src/commands/mod.rs` — register `pub mod sync;`.
- `src/commands/sync.rs` — **new file**, the orchestrator.
- `src/commands/status.rs` — extend rows with `would_promote` /
  `needs_tie_break`.
- `src/commands/view.rs` — extend `ViewStatusRow` similarly.
- `src/commands/project.rs` — extend `ProjectStatusRow` similarly.
- `tests/cli.rs` — 7 new test cases.

## Pitfalls

- **Q1: `process::exit(2)` swallows destructors.** Calling
  `process::exit` from `cli::run` skips the `Drop` impls of
  anything in scope. This is acceptable for skillnet — there are
  no async resources to flush — but be aware that any future
  `tokio::Runtime` or `tracing` flush hook will need to be moved
  before the exit. Symptom: future regression where logs drop
  the last few lines. Recovery: thread the exit code up through
  `main()` and use `std::process::ExitCode` instead.
- **Q2: `conflicts_with_all` on `no_promote` does not catch
  `--no-promote` passed alone with `--force`.** Re-read clap's
  docs: `conflicts_with_all` rejects when *all* named flags are
  also present, not "any". Use `conflicts_with` (singular) twice
  or rely on the action-matrix runtime check. Test both
  combinations to be sure. Recovery: switch to two
  `conflicts_with` attributes.
- **Q3: Empty `views` on global target.** The user's live
  config has two global views (`claude`, `agents`). A fixture
  with zero views is valid but the sync code must handle it
  (skip the per-view iteration cleanly, return zero counts).
  Symptom: integer underflow or `unwrap()` panic. Recovery:
  use `Vec::is_empty()` early returns where appropriate.
- **Q4: Per-project `project_root` is missing.** When iterating
  `Config::targets`, the global target's `project_root` is
  `None`. The closure pattern must not unconditionally call
  `ensure_target_clean(project_root)`; gate on the `Option`.
- **Q5: Status JSON shape break for downstream consumers.**
  Adding fields with `#[serde(default)]` semantics keeps the
  JSON backward-compatible (existing consumers ignore unknown
  fields). Make sure `#[derive(Serialize)]` on the row structs
  emits the new fields; if any field is `Option`, use
  `#[serde(skip_serializing_if = "Option::is_none")]` only if
  you want the field absent — but for `DriftEntry` mtime/sha,
  emit `null` so consumers see a uniform shape across drift
  kinds. The design § 4 expects `null` for absent values.
- **Q6: `compare_view_entry` failure on a symlink that survived
  status enumeration.** `view_status` may return a `NonSymlink`
  entry for which the underlying file disappeared between
  enumeration and comparison (race). Symptom: `compare_view_entry`
  returns an IO error. Recovery: in the status path, swallow the
  per-entry error, log a warning to stderr, and continue with
  `would_promote` / `needs_tie_break` unchanged for that entry.
  Do **not** propagate the error — status is read-only and must
  not fail the whole run on one stale entry.
- **Q7: Code 2 vs code 1 confusion in `--dry-run`.** The dry-run
  path returns code 0 per design § 3, but errors during dry-run
  (e.g., unreadable canonical) still propagate as code 1. Make
  sure the dry-run collapse only affects the "would-promote
  pending" branch, not the error branch.

## Reference

- Design dossier §§ 1-5 (linked from the phase README).
- Phase 01 outputs consumed (listed in the phase README).
- [src/cli/args.rs:686-727](../../../../src/cli/args.rs#L686-L727)
  for the existing test pattern to update.
- [src/cli/mod.rs:85-91](../../../../src/cli/mod.rs#L85-L91) for
  the existing `Sync` arm to rewrite.
- [src/exit.rs](../../../../src/exit.rs) — confirm there is no
  existing code-2 hook to reuse.
