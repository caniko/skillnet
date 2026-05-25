# Phase 05 — Base SKILL.md: heuristics catalog + meta-heuristics + sidecar spec

> **Recommended Codex model: GPT 5.5 high**
>
> Substantive content rewrite. The skill body is the contract every
> downstream flavor inherits and every plan author follows; getting it
> wrong creates either ceremony (the skill demands sections nobody
> needs) or silence (the skill misses real failure modes). The
> heuristic catalog with explicit thresholds, the meta-heuristic
> sampling rules, and the sidecar schema are design decisions whose
> shape was settled in the parent plan — this phase locks them into
> SKILL.md prose. Writing-heavy, design-heavy; `medium` would
> hand-wave the threshold values and miss the "section to add"
> specificity that makes the heuristics actionable. `max` is overkill
> for a writing task.

## Working tree

`/data/nvme0/can/Projects/ai-skills` (this repo). The skillnet CLI
lives in a separate repo (`/data/nvme0/can/Projects/skillnet`,
published at `ssh://git@codeberg.org/caniko/skillnet.git`); this phase
only writes ai-skills SKILL.md prose. Cross-repo coordination
matters: the sidecar schema and `surprises` convention documented
here are the contract that skillnet implements — they must agree
byte-for-byte with `src/calibration/sidecar.rs` and
`src/calibration/analyze.rs` in the skillnet repo (Phases 02 and 04).

## Goal

`global/multi-phase-plan/SKILL.md` is rewritten to:

1. **Drop the 3–8 phase cap.** Replace it with **per-phase shape
   rules** (one outcome, one rollback boundary, fits one session
   window).
2. **Add the heuristics catalog** — a list of `if X then add section Y`
   rules with explicit thresholds, grouped into four categories
   (coordination, risk, plan-shape, plan-quality lint).
3. **Add the meta-heuristics section** — the sampling rules that
   determine when a plan is recorded into the calibration dataset.
4. **Specify the `.calibration.json` sidecar schema** — the format
   the skill writes and `skillnet calibration` reads.
5. **Specify the tag conventions** — what auto-tags and user-tags
   look like, what the `signal:` tag means.
6. **Define the `surprises` text convention** — `dead-weight:` and
   `missed-signal:` prefixes that `skillnet calibration analyze`
   parses (Phase 04 documents the consumer side).

Hooks into `skillnet calibration record|verify` and the new
`calibrate` mode are added in Phase 06, not here.

## Why this matters now

The whole calibration loop is keyed off this content. The CLI
delivered by Phases 01–04 is inert without a skill body that:

- emits the right sidecar shape (Phase 02 consumes it);
- evaluates triggers (Phase 04 analyzes them);
- documents the `surprises` convention (Phase 04 parses it);
- knows when to record (meta-heuristics drive selection bias
  minimization).

Phase 06 wires the hooks; Phase 07 propagates to flavors. This phase
is the design contract everything else implements against.

Runs in parallel with Phases 01–04 in Wave 0/2. No code dependency —
SKILL.md is text. Lands before Phase 06.

## Out of scope

- The hook calls themselves (`skillnet calibration record …` at
  end-of-plan; `skillnet calibration verify …` at end-of-verify) —
  Phase 06.
- The new `calibrate` mode body — Phase 06.
- The calibration changelog footer scaffolding — Phase 06.
- Flavor skill changes — Phase 07.
- Any change to the per-phase file shape (Working tree / Goal / etc.).
  That contract stays.
- The existing dispatch / sub-layer machinery from `multi-phase-dispatch`.
  Untouched.

## Plan

1. **Read the current `global/multi-phase-plan/SKILL.md`** end to end
   so you know what's preserved and what's rewritten. Especially:
   - The "Required structure per phase file" block (preserved).
   - The "Required top-level README.md" section (preserved, may be
     extended with new mandatory sections triggered by heuristics).
   - The "Workflow" section, step 1 ("Take inventory… Aim for 3–8
     phases…") — this is the cap that gets removed.
   - The anti-patterns section (preserved; may get new entries).

2. **Replace the 3–8 phase cap** in the Workflow section with
   per-phase shape rules. Suggested rewrite (adjust prose to fit the
   surrounding tone):

   > **Take inventory.** List every concrete action the work entails.
   > Group related actions into candidate phases. Each phase must
   > satisfy the per-phase shape rules:
   >
   > - **One outcome.** The Goal section names a single
   >   user-observable outcome; you can write "this phase succeeds
   >   when …" in one sentence.
   > - **One rollback boundary.** The phase's changes can be reverted
   >   as a unit without leaving the repo in a half-state. If a phase
   >   crosses two natural revert boundaries, it's two phases.
   > - **One session window.** A fresh Codex/Claude session can
   >   plausibly complete the phase in one sitting, with the model
   >   tier recommended in the routing callout. If the phase is too
   >   big for `medium` and would need `high`, that's a routing
   >   signal, not a "make the phase bigger" signal.
   >
   > Phase count is whatever falls out of these rules. There is no
   > upper cap; large efforts (10+ phases) trigger additional
   > README sections via the heuristic catalog below.

3. **Add the heuristics catalog** as a new top-level section after
   "Sequencing and parallelism guidance" (or wherever it fits the
   doc flow). Use four subsections — each heuristic specifies:
   trigger condition, exact threshold, section it adds, where the
   section goes (README vs phase file).

   ### Coordination triggers
   - **Shared-file contention.** Trigger: ≥2 phases touch the same
     file. Adds README section "Shared-file lockstep" listing each
     file, owning phase, recovery if conflict. Each affected
     phase's Plan section cross-links the others.
   - **External-repo phases.** Trigger: any phase has a non-primary
     `Working tree`. Adds README section "External repo
     coordination" with per-repo branch + push protocol.
   - **Convergence point.** Trigger: any phase has ≥3 direct
     predecessors. The convergent phase gets a "Merge-readiness
     checklist" section enumerating each predecessor's expected
     output state.
   - **Ownership-boundary spread.** Trigger: phases span ≥2
     maintainer domains (your repo + nixpkgs + third-party). Adds
     README section "PR sequencing & cross-owner coordination".

   ### Risk triggers
   - **Risk concentration.** Trigger: ≥2 phases routed to `max`.
     Adds README "Risk-tier callout" grouping them with
     rollback-blast-radius notes.
   - **Risk-late-in-plan.** Trigger: a `max` phase sits in the
     final 1/3 of waves. Adds README "Late-risk warning" and
     prompts "front-load this phase?".
   - **Infrastructure single-point-of-failure.** Trigger: ≥1 phase
     touches CI / flake.nix / lockfiles / build system AND
     downstream phases depend on it. That phase is flagged
     "infra-SPOF" in the README; downstream phases' Pitfalls
     inherit a "if infra phase regresses, this phase's smoke is
     invalid" note.
   - **Re-vendor / dependency-bump.** Trigger: phase title or files
     mention `vendor`, `bump`, lockfile, or `Cargo.lock`. Routing
     tier suggestion bumps one notch (medium → high); the phase
     gets a "Compat surface" section listing what the bump can
     break.

   ### Plan-shape triggers
   - **Long serial chain.** Trigger: dependency chain ≥4 phases
     deep. Adds README "Serial-chain recovery" section noting
     compound failure cost; each phase from chain link 2 onward
     adds a smoke-prior-phase Acceptance criterion.
   - **Mid-plan re-routing checkpoint.** Trigger: phase count ≥10.
     README mandates "after wave N, re-run gpt-plan-routing on
     remaining phases" checkpoint.
   - **Trivial-phase swamp.** Trigger: ratio of `low`/`medium` to
     `high`/`max` ≥4:1. README appendix "Cleanup batch" groups
     trivial phases under one execution note.
   - **No integrated-verification phase.** Trigger: no phase
     exercises the end-to-end outcome. Warn and prompt adding a
     closing verification phase.

   ### Plan-quality lint triggers (warn, don't add section)
   - **Routing tier inversion.** Trigger: a leaf phase routes ≥ its
     orchestrator. Require an inline justification.
   - **Mechanical streak.** Trigger: ≥3 consecutive phases at
     `5.5 low`. Suggest bundling or sharing a session.
   - **Hidden prerequisite.** Trigger: phase Plan assumes state no
     earlier phase produces and the dependency table doesn't show
     it. Block; require explicit dep edge.

   Close the section with: "These thresholds are starting values.
   They evolve via the calibration loop — see the calibration
   changelog at the bottom of this file for the audit trail."

4. **Add the meta-heuristics section** — "When a plan gets recorded".
   Place after the heuristics catalog. Body:

   > A plan is written to the calibration dataset only when at least
   > one of the following meta-heuristics fires. The goal is to
   > minimize selection bias by concentrating the dataset on
   > calibration-worthy plans, not routine ones.
   >
   > - **Threshold proximity.** Any user-facing trigger's input is
   >   within ±20% of its threshold.
   > - **Trigger absence with risk shape.** No triggers fired, but
   >   the plan has ≥1 `max` phase, OR repo spread ≥3, OR chain
   >   depth ≥3.
   > - **Novel shape signature.** The plan's (phase_count bucket,
   >   wave_count bucket, repo_spread bucket, risk-tier dist)
   >   vector has not appeared in the dataset before.
   > - **Routing tier outlier.** Any phase routes higher or lower
   >   than the median for its declared complexity class.
   > - **Verify surprise** _(verify-time only)_. The verifier
   >   reports a failure no trigger pre-empted, an emergency dep,
   >   or a phase that had to be added.
   > - **Re-routing event** _(verify-time only)_. Any phase was
   >   executed at a different tier than recommended.
   > - **High-stakes combo.** ≥1 `max` phase AND ≥1 external-repo
   >   phase.
   > - **Uniform random.** With probability 0.07, regardless of
   >   other triggers. Anti-bias floor; prevents the meta-heuristic
   >   set from drifting into a sampling monoculture.
   >
   > Each meta-heuristic that fires is recorded in the sidecar's
   > `meta_heuristics_fired` array, so calibration can later check
   > whether each meta-heuristic itself produces signal.

5. **Add the sidecar schema spec** — "Sidecar `.calibration.json`".
   Document the JSON shape with field-by-field explanation. Cross-
   reference: this matches `src/calibration/sidecar.rs` in the
   external **skillnet** crate (at
   `ssh://git@codeberg.org/caniko/skillnet.git`) exactly. Pin the
   skillnet crate version this schema corresponds to (e.g.,
   "matches skillnet ≥0.1.0"); future schema versions will require
   updating both sides in lockstep with a new schema migration on
   the skillnet side. Include a minimal valid example.

6. **Add the tag conventions section** — "Tag conventions". Cover:
   - Auto-tags applied by `skillnet calibration record`: `flavor`,
     `worktype`, `scope`, `risk`, `signal`, `outcome`.
   - User-tags applied via `skillnet calibration tag <plan-id>
<k=v>...`: free-form, key must match `^[a-z][a-z0-9_-]*$`.
   - Per-band analysis: tag bands let `skillnet calibration analyze
--filter-tag k=v` slice the dataset; useful when triggers behave
     differently across flavors or worktypes.

7. **Add the `surprises` text convention** — "Verifier `surprises`
   field". Define the structured prefixes the `analyze` command
   parses (Phase 04):
   - `dead-weight: <trigger-name>: <note>` — the section this
     trigger added was useless for this plan. Counts as a false
     positive for the trigger.
   - `missed-signal: <expected-trigger-name>: <note>` — a failure
     occurred that the named trigger would have caught if its
     threshold were lower; counts as a false negative.
   - Anything else — informational; not parsed.

8. **Update the anti-patterns section** with two new entries:
   - **Treating heuristic thresholds as immutable.** The
     calibration loop exists because guesses get tuned. If a
     trigger feels wrong on your plan, that's a data point — note
     it in the verifier's `surprises` field with the appropriate
     prefix so the analyzer sees it.
   - **Skipping the verifier `surprises` field.** Without
     dead-weight / missed-signal annotations, `analyze` has only
     shape data to work with. The loop degrades gracefully but
     converges slower.

9. **Verify cross-references** end-to-end. The schema spec in
   SKILL.md must match `src/calibration/sidecar.rs` in the
   _skillnet_ repo exactly (Phase 02 is the source-of-truth for
   field names; SKILL.md mirrors). The `surprises` convention here
   must match what Phase 04's `analyze` parses in the skillnet
   repo. The meta-heuristic names here must match what the auto-tag
   `signal:` values use (skillnet's `record` cmd emits the tags;
   ai-skills' hook writes them into the sidecar). Cross-repo
   verification: open `skillnet/src/calibration/sidecar.rs` side-by-
   side with this SKILL.md section while writing; fix whichever side
   is wrong deliberately and document the choice.

10. **Skim-read the post-rewrite file** for tone consistency with
    the rest of `global/`. Avoid jargon ("trigger" is fine; "signal
    rate" is OK; "skew check" is OK if the term appears in
    `analyze`'s output the user will see).

## Acceptance criteria

- [ ] `global/multi-phase-plan/SKILL.md` no longer contains the
      "Aim for 3–8 phases" guidance.
- [ ] The per-phase shape rules (one outcome / one rollback / one
      session window) appear in the Workflow section.
- [ ] The heuristics catalog appears as a top-level section with
      four subsections (coordination / risk / plan-shape /
      plan-quality lint), each entry naming a trigger, an explicit
      threshold, and what section gets added where.
- [ ] All thirteen heuristics from Plan step 3 are present with the
      thresholds as written.
- [ ] The meta-heuristics section lists all eight sampling rules
      with thresholds matching Plan step 4.
- [ ] The sidecar `.calibration.json` schema is documented with one
      minimal valid example, and field names exactly match
      `src/calibration/sidecar.rs`.
- [ ] The tag conventions section enumerates the auto-tags
      (`flavor`, `worktype`, `scope`, `risk`, `signal`, `outcome`)
      and the user-tag key regex.
- [ ] The `surprises` text convention documents `dead-weight:` and
      `missed-signal:` prefixes; these match the parser in
      `src/calibration/analyze.rs`.
- [ ] Two new anti-patterns are added.
- [ ] A `grep -n "3–8\|3-8" global/multi-phase-plan/SKILL.md` returns
      no matches.
- [ ] A `grep -n "heuristic\|trigger\|threshold" global/multi-phase-plan/SKILL.md`
      returns the expected new sections.
- [ ] The existing per-phase file shape contract is unchanged
      (Working tree / Goal / Why / Out of scope / Plan / Acceptance
      criteria / Files likely touched / Pitfalls / Reference).

## Files likely touched

- `global/multi-phase-plan/SKILL.md` (substantive rewrite; ~40% new
  content, ~60% preserved).

## Pitfalls

- **Threshold values are guesses.** Document them as starting
  points; the calibration loop's whole purpose is to refine them.
  Resist the urge to over-justify any specific number — the changelog
  footer (Phase 06) is where revisions get recorded.
- **Sidecar field-name drift across repos.** This phase writes the
  schema spec into ai-skills SKILL.md. Phase 02 implemented
  `src/calibration/sidecar.rs` in the _skillnet_ repo. Cross-repo
  drift is the most likely failure mode of this whole plan — there's
  no compiler to catch a mismatch. Mitigation: clone the skillnet
  repo locally and diff field-by-field before declaring this phase
  done. If a field name should change, change skillnet first
  (bump the crate's schema version), publish a new patch release
  (Phase 07's workflow), then update SKILL.md to match.
- **`surprises` convention drift.** Same: Phase 04's `analyze`
  parses specific prefixes in the skillnet repo. SKILL.md
  documents what the verifier should write. If they don't agree,
  the loop produces garbage signal. Verify by grepping both repos
  for `dead-weight:` and `missed-signal:`.
- **Heuristic prose vs heuristic data.** The catalog in SKILL.md is
  prescriptive (humans read it; agents follow it). The trigger rows
  in SQLite are descriptive (whatever the skill emits gets
  recorded). If SKILL.md says a trigger fires at "≥3" but the
  agent's evaluation code emits `threshold=2`, the dataset will be
  misleading. Phase 06 wires the agent's evaluation; this phase
  must produce thresholds that 06 can faithfully evaluate (i.e.,
  computable from the inventory in workflow step 1).
- **Workflow order shift.** The current Workflow step 1 ("Take
  inventory") gets the per-phase shape rules. Don't move step 1
  itself; rewrite its body. Subsequent steps (dependency table,
  parallelism layer, routing) are unchanged.
- **README content vs phase-file content boundary.** New heuristic
  sections live in the README, not in phase files. The phase-file
  contract is preserved. Don't add new mandatory phase-file
  sections — if a heuristic needs information in a phase file,
  it's a _Pitfalls_ or _Plan_ addition for that one phase, not a
  new top-level section.
- **mdBook rendering.** If the SUMMARY references this skill,
  ensure the new headings don't break navigation. (The skill
  itself isn't published; only project plans are.)

## Reference

- Parent plan: `docs/planning/calibration-loop/README.md`.
- Sidecar implementation (external crate):
  `skillnet/src/calibration/sidecar.rs` (Phase 02).
- `surprises` parser (external crate):
  `skillnet/src/calibration/analyze.rs` (Phase 04).
- Existing SKILL.md being rewritten:
  `global/multi-phase-plan/SKILL.md`.
- Hooks + calibrate mode that depend on this rewrite:
  `06-base-skill-hooks-calibrate-mode.md`.
- Crate publication delivering the installed binary:
  `07-skillnet-crate-publication.md`.
- HM module that installs it for users: `08-nix-hm-module.md`.
- ai-skills flake consumption: `09-ai-skills-consumption.md`.
- Sister skill (untouched but referenced): `global/multi-phase-dispatch/`.
