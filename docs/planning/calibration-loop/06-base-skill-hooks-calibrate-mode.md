# Phase 06 — Base SKILL.md: hooks at plan/verify + `calibrate` mode

> **Recommended Codex model: GPT 5.5 medium**
>
> Closes the loop: wires the skill body (post-Phase-05) to invoke the
> CLI delivered by Phases 02 and 04, and adds the `calibrate` mode
> body that walks the user through `analyze` → `propose` → `decide`
> → `export-changelog`. Moderate complexity: the workflow hooks are
> straightforward shell-out calls, the calibrate mode is a small
> guided interaction, the changelog scaffolding is mechanical. The
> only judgment call is how prescriptive the calibrate-mode prose
> should be (suggested level: terse, the analyzer output is already
> structured enough). `low` would underspec the failure-handling for
> a missing CLI; `high` is unnecessary.

## Working tree

`/data/nvme0/can/Projects/ai-skills` (this repo). The hooks shell out
to the **installed** `skillnet` (published from a separate repo at
`ssh://git@codeberg.org/caniko/skillnet.git`, distributed via the Nix
HM module in Phase 08, or fallback `nix run` form for users who
haven't enabled the module). This phase does *not* touch the
skillnet repo.

## Goal

`global/multi-phase-plan/SKILL.md`, post-rewrite from Phase 05, gets
three additions:

1. **End-of-plan hook.** A workflow step that evaluates meta-heuristics
   (Phase 05's catalog), writes `.calibration.json` if any fire, and
   shells out to `skillnet calibration record <plan-dir>`.
2. **End-of-verify hook.** The verify workflow appends a `verify`
   section to `.calibration.json` (creating the sidecar with
   best-effort reconstruction if it doesn't exist and a verify-time
   meta-heuristic fires), then shells out to
   `skillnet calibration verify <plan-dir>`.
3. **`calibrate` mode body.** A new mode (sister to `plan` and
   `verify`) that walks the user through analyzing the dataset,
   proposing threshold changes, deciding on them, and appending the
   result to the calibration changelog at the bottom of SKILL.md.

Plus a **calibration changelog footer** section at the bottom of
SKILL.md, initially empty, that future `export-changelog` runs
append to.

## Why this matters now

This is the integration phase: the CLI is ready (02, 04, published
by 07), the skill body documents the contract (05), but nothing
actually flows yet. Without this phase, the calibration dataset
stays empty and the loop delivers nothing. Phase 09 propagates the
modes to the three flavor skills and wires the ai-skills flake to
consume the published `skillnet` crate.

This phase can run as soon as Phase 07 publishes a referenceable
crate version. It does *not* hard-depend on Phase 08 (HM module)
because the hooks can fall back to `nix run codeberg.org/caniko/skillnet#skillnet
-- ...`, but writing the prose is cleaner once the HM module's
installed-on-PATH guarantee exists.

## Out of scope

- Flavor skill updates (codex/claude/mixed) — Phase 09.
- Crate publication, Cargo metadata, Codeberg CI — Phase 07.
- Nix HM module — Phase 08.
- ai-skills flake input pointing at the published skillnet — Phase 09.
- Any change to the CLI surface — Phases 02–04 are done.
- The heuristics catalog itself — Phase 05.
- Auto-editing SKILL.md from the CLI. The `calibrate` mode emits a
  changelog block and instructs the user to paste it; the user
  remains the editor.

## Plan

1. **Read the post-Phase-05 `global/multi-phase-plan/SKILL.md`** end
   to end so you know where the heuristics catalog, meta-heuristics,
   sidecar schema, and tag conventions live. The hooks reference all
   four.

2. **Add end-of-plan workflow step.** Append a new step after the
   existing final step ("Provide a routing summary"):

   > **Record calibration data (if any meta-heuristic fires).**
   > Evaluate the meta-heuristic catalog against the plan you just
   > wrote (see "When a plan gets recorded" section). If any meta-
   > heuristic fires:
   >
   > 1. Generate a UUID for the plan (`plan_id`).
   > 2. Compute the auto-tags from plan metadata (see "Tag
   >    conventions").
   > 3. Write `<plan-dir>/.calibration.json` per the sidecar schema.
   > 4. Run `skillnet calibration record <plan-dir>`.
   >    - If the user has the skillnet Home Manager module enabled
   >      (`programs.skillnet.enable = true`), the binary is on
   >      PATH directly.
   >    - If not, fall back to
   >      `nix run codeberg.org:caniko/skillnet#skillnet --
   >      calibration record <plan-dir>`.
   >    - If both fail (no nix, no installed binary), report a
   >      one-line install hint and skip recording — the deliverable
   >      is the plan, not the data row.
   > 5. Surface the recording in the chat reply: "Recorded for
   >    calibration: <reasons>". Do not surface anything if no
   >    meta-heuristic fired.
   >
   > Calibration recording is a best-effort augment, not a blocker
   > for the plan deliverable.

3. **Add end-of-verify workflow step.** In the verify-mode workflow
   (from `multi-phase-plan` base), append:

   > **Record verify outcome (if sidecar exists OR a verify-time
   > meta-heuristic fires).**
   >
   > - If `<plan-dir>/.calibration.json` exists: append the
   >   `verify` section per the sidecar schema, including the
   >   `surprises` field using the structured prefixes
   >   (`dead-weight:`, `missed-signal:`) where applicable.
   > - If the sidecar does *not* exist but a verify-time meta-
   >   heuristic fires (verify-surprise, re-routing event):
   >   reconstruct a best-effort sidecar from the plan files (plan
   >   metadata derivable from README + phase files; trigger states
   >   reconstructed by re-evaluating the heuristics catalog
   >   against the plan as written; `meta_heuristics_fired` set to
   >   the verify-time reasons). Then append the `verify` section.
   > - Either way, run `skillnet calibration verify <plan-dir>` (or
   >   the `nix run` fallback documented in the end-of-plan hook).
   >
   > As with `record`, CLI errors are reported and don't block the
   > verify deliverable.

4. **Add the `calibrate` mode.** Insert a new top-level section
   between the verify-mode docs and the anti-patterns. Body:

   > ## Mode: `calibrate`
   >
   > When the user says "calibrate", "tune the heuristics", "review
   > calibration data", or invokes the skill with `calibrate` as the
   > first word, run this mode instead of `plan` or `verify`.
   >
   > Calibrate mode does not write phase files. It walks the user
   > through the calibration dataset and emits a proposed edit to
   > this SKILL.md's heuristic thresholds and changelog footer.
   >
   > ### Workflow
   >
   > 1. **Run analysis.** Shell out to `skillnet calibration
   >    analyze --format json` (or `nix run` fallback). The JSON
   >    output contains per-trigger fire/signal rates, candidate
   >    proposals (triggers above min-N with actionable signal),
   >    and skew warnings.
   >
   > 2. **Surface the report.** Format the JSON as a table for the
   >    user, highlighting:
   >    - Triggers with proposals (one-line summary each).
   >    - Skew warnings (one-line each).
   >    - Triggers below min-N (one-line each; informational only).
   >
   > 3. **For each candidate proposal, confirm with the user**:
   >    - Show the trigger name, current threshold, proposed
   >      threshold, fire rate, signal rate, supporting plan count.
   >    - Ask whether to formalize the proposal (`propose`), skip
   >      it, or refine the filter tags.
   >    - On confirmation, shell out to `skillnet calibration
   >      propose --trigger NAME --new-threshold N --rationale
   >      "<short rationale>" --supporting-plan-ids id1,id2,...`.
   >
   > 4. **For each pending proposal**, ask the user to accept or
   >    reject with a rationale. Shell out to `skillnet calibration
   >    decide <id> accept|reject --rationale "..."`.
   >
   > 5. **Export the changelog.** Shell out to `skillnet calibration
   >    export-changelog --since <last-changelog-date-in-SKILL.md>`.
   >    Emit the markdown blocks to the user with the instruction:
   >    *"Append these blocks to the 'Calibration changelog' section
   >    at the bottom of `global/multi-phase-plan/SKILL.md`, then
   >    edit the per-trigger thresholds in the heuristic catalog to
   >    match the accepted proposals."*
   >
   > 6. **Do not edit SKILL.md from the calibrate mode itself.** The
   >    user is the editor; the mode produces text to paste. This
   >    keeps changelog provenance auditable and lets the user catch
   >    bad proposals before they ratchet.
   >
   > ### Cadence
   >
   > Calibrate mode is user-initiated, not scheduled. Suggested
   > cadence: after every ~10 verified plans, or when the user
   > notices a heuristic firing inappropriately. The min-N guard in
   > `analyze` ensures running calibrate too early is a no-op
   > rather than a noise generator.

5. **Add the calibration changelog footer**. At the very bottom of
   SKILL.md, after the existing "Reference" section, append:

   ```markdown
   ## Calibration changelog

   Threshold changes to the heuristic catalog above are recorded
   here. Each entry is produced by the `calibrate` mode via
   `skillnet calibration export-changelog` and pasted by the user.
   The dataset backing these decisions lives at
   `data/multi-phase-plan/calibration.sqlite`.

   <!-- Newest first. Format produced by `skillnet calibration export-changelog`. -->
   ```

   Leave the section body empty (just the comment marker). The
   first real entry will be appended by a future calibrate-mode
   run.

6. **Update the anti-patterns section** with one more entry:

   - **Hand-editing the calibration changelog.** The changelog is
     the audit trail for threshold tuning. Edits should come from
     `skillnet calibration export-changelog`, not freehand prose.
     If the format needs to change, change the exporter, not the
     changelog.

7. **Cross-reference verify.** Verify mode (from
   `multi-phase-plan` base) already has its own workflow. The
   end-of-verify hook in step 3 above is an addition, not a
   rewrite. Ensure the hook step is numbered correctly relative to
   the existing verify steps.

8. **Sanity-check the hook commands.** The primary invocation is
   `skillnet calibration record <plan-dir>` assuming the HM module
   (Phase 08) has installed the binary. The fallback
   `nix run codeberg.org:caniko/skillnet#skillnet -- calibration
   record <plan-dir>` works for users without the HM module — it
   resolves the published Codeberg flake at runtime. Test both forms
   in a clean shell before declaring the prose right.

9. **Verify the mode-dispatch logic** in the existing "Modes"
   section at the top of the file (introduced by Phase 05's
   preserved scaffolding). Add `calibrate` as a third mode
   alongside `plan` and `verify`, with a one-line description.

10. **Skim-read** the post-edit file for consistency. The hooks
    reference sections that Phase 05 added; spot-check the section
    names (they may have been refined during 05's writing pass).

## Acceptance criteria

- [ ] `global/multi-phase-plan/SKILL.md` contains a new workflow
      step at the end of the plan workflow that invokes
      `skillnet calibration record` conditionally on meta-heuristic
      fires.
- [ ] The verify workflow contains a new step that invokes
      `skillnet calibration verify` and handles the
      sidecar-doesn't-exist case via best-effort reconstruction.
- [ ] A new top-level `## Mode: calibrate` section exists,
      structured per Plan step 4.
- [ ] The Modes section at the top of the file lists `plan`,
      `verify`, and `calibrate` (three modes, not two).
- [ ] A new `## Calibration changelog` section exists at the bottom
      of the file with the placeholder comment and no entries yet.
- [ ] One new anti-pattern entry covers "hand-editing the
      calibration changelog".
- [ ] The hook invocations name `skillnet …` as primary and
      `nix run codeberg.org:caniko/skillnet#skillnet --` as
      fallback, with a one-line install hint pointing at the HM
      module (Phase 08).
- [ ] All hook commands match the CLI surface delivered by Phases
      02 and 04 verbatim (`skillnet calibration record|verify|
      analyze|propose|decide|export-changelog`).
- [ ] Cross-references to Phase 05's sections ("When a plan gets
      recorded", "Tag conventions", sidecar schema) resolve to
      headings that actually exist in the post-05 file.
- [ ] CLI errors are documented as non-blocking for the plan/verify
      deliverable.

## Files likely touched

- `global/multi-phase-plan/SKILL.md` (additions; ~600 new lines
  estimated across the new workflow steps, calibrate mode body,
  changelog footer, and updated Modes header).

## Pitfalls

- **Hook command drift.** The exact CLI invocations must match what
  Phases 02 and 04 ship. Grep the CLI source after both have
  landed; spot-check every shell-out command in SKILL.md against
  the actual subcommand surface.
- **Best-effort sidecar reconstruction at verify time.** This is the
  trickiest piece. If the original plan was not recorded (no
  meta-heuristic fired at plan time), there's no `plan_id`. Use a
  fresh UUID; the dataset accepts it as a new plan row. Document
  that the reconstructed plan's "shape" is derived from the README
  + phase files as they exist *now*, not as they existed when the
  plan was generated. This drift is acknowledged and acceptable —
  the verify-surprise signal is the value, not perfect provenance.
- **`calibrate` mode doesn't auto-edit SKILL.md.** This is
  deliberate; resist the urge to add an "auto-apply" flag. The
  user-as-editor invariant keeps bad proposals from ratcheting bad
  thresholds. Document the choice in the mode body.
- **Cadence-prescriptive prose.** Don't tell the user "you must
  calibrate after every N plans". The min-N guard in `analyze`
  ensures premature calibration is a no-op. Suggested cadence is
  guidance, not a rule.
- **Conflict with Phase 05 wave acceptance.** This phase edits
  `global/multi-phase-plan/SKILL.md` after Phase 05 has already
  rewritten it. Pull the post-05 file before starting; verify the
  headings you cross-reference exist with the names you expect.
- **Mode-dispatch ordering.** When the user invokes the skill, the
  dispatcher reads the first word. If the user says "calibrate the
  thresholds for this plan", the dispatcher should pick `calibrate`,
  not `plan`. Document the mode-pick rule explicitly: first word
  match wins; default is `plan`.
- **`export-changelog --since` parsing.** The "last changelog date"
  is parsed from the existing changelog footer's first entry. If
  the footer is empty (first run), pass no `--since` flag and
  export everything (which will be the full history of accepted
  proposals — initially zero).
- **Changelog footer placement.** It must be the *last* section in
  the file, because future exports append entries newest-first
  inside it. If you put it before "Reference", future runs will
  scatter changelog entries through the file or require manual
  re-placement.

## Reference

- Parent plan: `docs/planning/calibration-loop/README.md`.
- Phases this phase wires together (CLI from external crate):
  `02-cli-record-verify.md`, `04-cli-analyze-propose-decide.md`.
- Phase this phase depends on (ai-skills SKILL.md rewrite):
  `05-base-skill-heuristics-rewrite.md`.
- Crate publication that ships the binary referenced in hooks:
  `07-skillnet-crate-publication.md`.
- HM module that puts it on PATH: `08-nix-hm-module.md`.
- Flavor propagation + ai-skills flake consumption:
  `09-ai-skills-consumption.md`.
- Existing modes scaffolding (preserved by Phase 05):
  `global/multi-phase-plan/SKILL.md` "Modes" section.
