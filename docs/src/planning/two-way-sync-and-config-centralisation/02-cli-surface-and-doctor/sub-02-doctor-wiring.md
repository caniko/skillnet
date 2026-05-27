# Phase 02 / Sub-layer 02 — Doctor severity classification

> **Recommended Codex model: GPT 5.5 medium**
>
> Single-file change with a six-row severity table plus a new test
> file. The classification rules are pinned by design § 9 and the
> comparator is shipped by Phase 01, so the work is mechanical
> wiring plus careful hint-string copy-editing (the user sees these
> strings on every doctor run, so a typo or unhelpful phrasing
> propagates). 5.5 medium is right: enough reasoning to keep the
> matrix consistent across `Warn` vs `Error` decisions and to
> craft hint strings that point at the right next command.

## Working tree

`/data/nvme0/can/Projects/skillnet`. Phase 01 must be landed on
`main` before this sub-layer starts. This sub-layer runs in parallel
with `sub-01-cli-surface`; they touch disjoint files.

## Goal

`skillnet doctor` distinguishes the six comparator outcomes
(`Identical`, `ViewNewer`, `CanonicalNewer`,
`EqualMtimeDifferentContent`, `BothAdvanced`, `AdoptCandidate`) for
every `NonSymlink` view entry and reports the design § 9 severity
+ hint matrix. Doctor's overall exit code follows the same rule as
today: zero if every reported row is `Info`, non-zero if any
`Warn`/`Error` row is present.

## Why this matters now

Today `skillnet doctor` treats any `NonSymlink` entry as a flat
error with a single hint about `--force`. After Phase 01 + sub-01
land, the user gets a richer set of next-command options
(`--apply-promote`, `--prefer`, `--adopt-new`), and doctor is the
discoverability surface for those flags. Without this sub-layer,
the new flags exist but the user has no in-CLI way to learn which
flag applies to which kind of drift.

The design's mtime-spoof mitigation in § 10 hinges on doctor being
the recommended pre-sync inspection step. That recommendation is
empty if doctor doesn't classify outcomes.

## Out of scope

- Any change to `src/view.rs`, `src/commands/sync.rs`,
  `src/cli/args.rs`, `src/cli/mod.rs`,
  `src/commands/{view,project,status}.rs`. Sub-01 owns those.
- Adding new severity levels beyond `Info`, `Warn`, `Error`.
  Whatever the existing doctor module uses, this sub-layer keeps.
- Auto-fix functionality. Doctor reports; the user runs the fix.
- Output format change (text vs JSON). Doctor today emits text;
  keep it.

## Plan

1. **Read inputs.** Open
   [design § 9 Doctor severity matrix](../../../two-way-sync-and-config-centralisation-research.md#9-doctor-severity-matrix)
   and [§ 10 mtime-spoof mitigation](../../../two-way-sync-and-config-centralisation-research.md#10-mtime-spoof-mitigation).

   Open [src/commands/doctor.rs](../../../../src/commands/doctor.rs)
   and locate where `NonSymlink` drift is currently classified.
   Note the existing `Severity` enum (or equivalent), the report
   row struct, and the exit-code derivation. **Do not assume a
   specific shape** — read the file as it currently exists.

2. **Add a classification helper.** In `src/commands/doctor.rs`
   (or a new sibling module if the file gets large), add:

   ```rust
   use crate::view::ReconcileOutcome;

   fn classify_non_symlink(
       outcome: &ReconcileOutcome,
   ) -> (Severity, &'static str) {
       match outcome {
           ReconcileOutcome::Identical => (
               Severity::Info,
               "`skillnet sync` will silently demote to symlink",
           ),
           ReconcileOutcome::ViewNewer { .. } => (
               Severity::Warn,
               "`skillnet sync --apply-promote` will pull view → canonical and re-link",
           ),
           ReconcileOutcome::CanonicalNewer { .. } => (
               Severity::Error,
               "`skillnet sync --force` will destroy view-side edits; review before running",
           ),
           ReconcileOutcome::EqualMtimeDifferentContent { .. } => (
               Severity::Error,
               "`skillnet sync --apply-promote --prefer view|canonical` required",
           ),
           ReconcileOutcome::BothAdvanced { .. } => (
               Severity::Error,
               "`skillnet sync --apply-promote --prefer view|canonical` required; per-file merge is not supported",
           ),
           ReconcileOutcome::AdoptCandidate => (
               Severity::Info,
               "view-only skill; `skillnet sync --apply-promote --adopt-new` to promote",
           ),
       }
   }
   ```

   The hint strings are exact copies from design § 9 — do not
   paraphrase. The dossier is the source of truth; if a hint
   needs polish, edit the dossier first.

3. **Call the classifier from the doctor walk.** For every
   `DriftKind::NonSymlink` entry the existing code currently
   reports, compute the comparator outcome via
   `view::compare_view_entry(canonical_skill_path, view_entry_path)`,
   pass it through `classify_non_symlink`, and use the returned
   `(Severity, hint)` to drive the row.

   - The existing per-entry row format stays. Only the severity
     classification and the hint string change.
   - On `Err` from `compare_view_entry` (rare; race or IO error),
     fall back to the pre-existing `Severity::Error` with hint
     `"could not classify entry: {err}; rerun doctor or check
     permissions"`. Do not panic.

4. **Preserve existing doctor behaviour for other drift kinds.**
   `Missing`, `WrongTarget`, `Stale` keep whatever severity and
   hint they have today. Only `NonSymlink` rows change.

5. **Exit code.** Doctor's exit code logic stays as today:
   non-zero iff any `Warn`/`Error` row is present. Because
   `Identical` and `AdoptCandidate` map to `Info`, fixtures that
   contain only those classes return code `0`. `ViewNewer`
   fixtures return non-zero (the new `Warn`).

6. **Tests.** Add `tests/doctor.rs` (new file) using `assert_cmd`:

   - `doctor_classifies_identical_non_symlink_as_info`: fixture
     with one non-symlink view entry whose content + mtime
     match canonical. Run `skillnet doctor`. Assert exit code
     `0`, stdout contains the hint string and an `INFO`-equivalent
     marker (match whatever the existing doctor output uses).
   - `doctor_classifies_view_newer_as_warn`: fixture with view
     newer. Assert exit code non-zero and stdout contains
     `"--apply-promote"`.
   - `doctor_classifies_canonical_newer_as_error`: assert exit
     code non-zero and stdout contains `"will destroy view-side
     edits"`.
   - `doctor_classifies_equal_mtime_different_content_as_error`:
     assert stdout contains `"--prefer view|canonical"`.
   - `doctor_classifies_both_advanced_as_error`: assert stdout
     contains `"per-file merge is not supported"`.
   - `doctor_classifies_adopt_candidate_as_info`: assert exit
     code `0`, stdout contains `"--adopt-new"`.
   - `doctor_falls_through_for_other_drift_kinds`: fixture with
     `Missing` and `Stale` entries only. Assert behaviour matches
     pre-this-sub-layer doctor (i.e., the existing tests for
     these classes still pass).
   - `doctor_handles_classify_io_error_gracefully`: fixture where
     the view entry's parent dir is then made unreadable (via
     `std::os::unix::fs::PermissionsExt`); doctor reports the
     fallback error hint and exits non-zero, does not panic.

   Use `tempfile::tempdir` + `filetime::set_file_times` per the
   patterns the existing test files already use.

7. **Run the local check loop.** `cargo fmt`, `cargo clippy
   --all-targets -- -D warnings`, `cargo test --workspace`. Make
   sure existing `tests/cli.rs` doctor cases (if any) still pass
   — sub-01 may or may not have changed test counts there; this
   sub-layer should not affect those.

8. **Commit.** One commit, message:
   `feat: classify NonSymlink drift in skillnet doctor`

## Acceptance criteria

- [ ] `src/commands/doctor.rs` contains a `classify_non_symlink`
      helper (or equivalent function) that maps every
      `ReconcileOutcome` variant to the design § 9 severity +
      hint pair.
- [ ] The 8 new tests in `tests/doctor.rs` pass.
- [ ] Existing doctor tests (if any) still pass without
      modification, *or* are updated with a single-line comment
      noting the severity change and the test still pass.
- [ ] `cargo test --workspace`, `cargo clippy --all-targets -- -D
      warnings`, `cargo fmt --check` clean.
- [ ] Running `skillnet doctor` against the live host (which
      currently has zero drift; see the prior dossier's evidence)
      reports clean — no regression for the no-drift case.
- [ ] `git log -1` shows the single sub-layer commit.

## Files likely touched

- `src/commands/doctor.rs` — classification helper + wiring.
- `tests/doctor.rs` — **new file** with the 8 fixtures.

## Pitfalls

- **R1: `compare_view_entry` is called from `src/view.rs` in
  sub-01 too.** Both sub-layers consume the same library
  function. There is no file conflict (`compare_view_entry` lives
  in `src/view.rs`, which Phase 01 owns), but the two sub-layers
  will each have their own call sites. Make sure your call passes
  the correct paths (canonical skill *directory*, view entry
  *directory*) — the function expects two existing paths, one of
  which is the canonical version of the same-named skill. If
  canonical doesn't exist, `compare_view_entry` returns
  `AdoptCandidate`, which is what you want.
- **R2: Severity enum drift.** The doctor file may use
  `Severity::Note` instead of `Severity::Info`, or `Severity::Warning`
  instead of `Severity::Warn`. Match the existing enum names
  exactly; do not invent new variants.
- **R3: Doctor output format snapshot may exist.** If
  `tests/doctor.rs` exists (or doctor output is snapshotted
  anywhere), update the snapshot to include the new hint strings.
  Run `cargo test --workspace` first to discover what breaks
  before adding test cases.
- **R4: Hint strings must include backticks for code spans.** The
  user reads these in a terminal, often without colour. Backticks
  signal "this is a command you can copy-paste". Match the
  dossier's hint string formatting exactly.
- **R5: `Info` severity on `Identical` may surprise the user.**
  An `Identical` non-symlink is "view is a real directory but
  matches canonical exactly". Doctor previously called this an
  error; now it is Info. Document the change in your commit
  message body (one sentence) so the v0.6.0 CHANGELOG (Phase 05)
  has source material.

## Reference

- Design dossier:
  - [§ 9 Doctor severity matrix](../../../two-way-sync-and-config-centralisation-research.md#9-doctor-severity-matrix)
  - [§ 10 mtime-spoof mitigation](../../../two-way-sync-and-config-centralisation-research.md#10-mtime-spoof-mitigation)
- Phase 01 outputs consumed:
  `view::ReconcileOutcome`, `view::compare_view_entry`.
- [src/commands/doctor.rs](../../../../src/commands/doctor.rs) —
  the file to read first; structure and Severity enum are local
  conventions to preserve.
