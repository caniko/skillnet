# Phase 04 — CLI: analyze, propose, decide, export-changelog

> **Recommended Codex model: GPT 5.5 high**
>
> This is the calibration brain. The work is defining a small but
> consequential algorithm — per-trigger fire rate and signal rate
> from the recorded rows, plus a threshold-proposal generator with
> per-tag-band awareness — that determines whether the whole loop
> produces sensible recommendations or garbage. The CLI plumbing is
> mechanical; the algorithm and the minimum-sample-size guards are
> design content. `medium` would underspec the signal-rate
> definition and ship a brittle analyzer; `max` would be overkill
> for an algorithm whose inputs are this constrained.

## Working tree

`/data/nvme0/can/Projects/skillnet` (the new standalone crate at
`ssh://git@codeberg.org/caniko/skillnet.git`).

## Goal

Five subcommands on `skillnet calibration` that close the calibration
loop:

- `analyze [--filter-tag k=v]... [--trigger NAME] [--min-n N]` —
  compute per-trigger fire+signal rates from the recorded dataset and
  emit candidate threshold deltas as a structured report.
- `propose --trigger NAME --new-threshold N [--filter-tag k=v]...
  --rationale "..." --supporting-plan-ids id,id,...` — write a
  proposal row.
- `proposals [--pending|--accepted|--rejected]` — list proposals.
- `decide <proposal-id> accept|reject --rationale "..."` — record a
  decision.
- `export-changelog [--since DATE]` — emit a SKILL.md-shaped
  changelog block from accepted proposals.

The `analyze` algorithm and its guard rails (minimum N, per-tag-band
breakouts) are the substantive deliverable. Everything else is
straightforward CRUD on `calibration_proposals`.

## Why this matters now

Without `analyze`, the calibration loop is a one-way pipe: data goes
in, nothing comes out. Phase 06 wires the skill's `calibrate` mode
(in ai-skills) to invoke `analyze` and walk the user through
`propose` → `decide` → `export-changelog`. The algorithm choices made
here determine whether the user trusts the proposals; bad signal-rate
math leads to the user ignoring the loop, which is failure.

Public-crate context: this phase ships in the first published version
of `skillnet` (Phase 07). The `analyze` JSON schema is the contract
that the ai-skills `calibrate` mode parses (Phase 06); changing it
later is breaking. Lock it now.

Runs in parallel with Phase 03 in Wave 2. Shares
`src/cli/args.rs` and `src/commands/calibration.rs`; coordinates via
the `// PHASE 04 commands here` placeholder Phase 02 left.

## Out of scope

- Auto-applying threshold changes. The skill body (in
  `ai-skills/global/multi-phase-plan/SKILL.md`) is human-edited; the
  CLI emits a changelog block, the user decides what to do with it.
- Crate publication, Cargo metadata, Codeberg CI — Phase 07.
- Nix HM module — Phase 08.
- Cross-trigger correlation analysis (e.g., "trigger A and trigger B
  always co-fire, drop one"). Useful future work; not this phase.
- Anomaly detection beyond simple boundary cases. The min-N guard is
  the only statistical machinery in this phase.
- Per-flavor separate thresholds. The data carries `flavor:*` tags
  (Phase 02) and `analyze` can filter by them, but the *skill itself*
  uses one threshold per trigger today. Per-band thresholds are a
  later phase if/when the data justifies them.

## Plan

1. **Rebase against Phase 02**: read the post-02 `src/cli/args.rs` to
   see where the `// PHASE 04 commands here` placeholder is. If
   Phase 03 has also landed, rebase on top of that too.

2. **Define the signal-rate algorithm** in
   `src/calibration/analyze.rs`. Document it in module-level
   rustdoc so the rationale is preserved next to the code:

   For each trigger `T`:
   - `N_fires` = number of plan rows where `T.fired = true`.
   - `N_misses` = number of plan rows where `T.fired = false`.
   - **Fire rate** = `N_fires / (N_fires + N_misses)`. Tells us how
     often the trigger captures something; pure descriptive.
   - **Signal rate** = the fraction of `N_fires` where the section
     `T` added correlates with a verified-good outcome, *minus* the
     fraction of `N_misses` where the corresponding failure mode
     appeared anyway.

     Concretely: define `helpful(T, plan)` and `failure_mode(T,
     plan)`:
     - `helpful(T, plan)` = `T.fired` AND `plan.verification.outcome
       ∈ {shipped, partial}` AND the verifier did not list `T.name`
       in `surprises` as dead weight.
     - `failure_mode(T, plan)` = the verifier listed `T.name` in
       `surprises` as a missed signal (i.e., trigger should have
       fired but didn't), OR the `emergency_changes` JSON contains a
       hint matching `T`'s scope.

     Then:
     ```
     true_positives  = count(fired AND helpful)
     false_positives = count(fired AND NOT helpful)
     false_negatives = count(NOT fired AND failure_mode)
     true_negatives  = count(NOT fired AND NOT failure_mode)
     signal_rate = (true_positives - false_positives - false_negatives)
                   / max(1, true_positives + false_positives + false_negatives)
     ```
     This is a custom score, not a standard precision/recall — chosen
     because both false-positive (dead-weight sections) and
     false-negative (missed failure modes) are equally bad here, and
     because rewarding only true positives would let a trigger that
     fires constantly look great.

     Range: `[-1, +1]`. Negative means the trigger is net-harmful
     (more dead weight or missed signal than helpful firings).
     Around 0 means the trigger is wasted ceremony. Positive means
     it earns its place.

3. **Threshold-proposal generation** in the same module:
   - Only propose for triggers with `N_fires >= MIN_N` (default 10;
     `--min-n` overrides).
   - If `signal_rate >= 0.3`: propose **lowering** the threshold by
     one notch (more inclusive) — but only if false-negative count
     ≥ 2 within the dataset, indicating real missed cases.
   - If `signal_rate <= 0.0` AND fire rate ≥ 0.5: propose
     **raising** the threshold by one notch (less inclusive) — the
     trigger fires often but doesn't pay for itself.
   - If `signal_rate` is between 0 and 0.3: hold; emit a "monitor"
     line, no proposal.
   - "One notch" is trigger-specific. For numeric thresholds (chain
     depth, phase count), notch = ±1. For ratio thresholds (e.g.,
     trivial:non-trivial 4:1), notch = ±0.5. The trigger's
     declaration in the dataset (`triggers.threshold` column) plus a
     small per-trigger `notch_size()` lookup table handle this; the
     table lives in `analyze.rs` and is initially keyed by trigger
     name with a default of `1.0`.

4. **Per-tag-band breakouts**:
   - `analyze` accepts `--filter-tag k=v` (repeatable). When
     present, it restricts the dataset to plans matching all
     specified tags before computing rates.
   - Additionally, when run without filter tags, `analyze` emits a
     "skew check" section that re-runs the rates separately for
     each value of `flavor` and `worktype`. If any band's signal
     rate differs from the global by more than ±0.3 *and* the band
     has at least 30 fires, emit a warning: "trigger T shows skew
     across <tag>: consider per-band thresholds".

5. **CLI surface** in `src/cli/args.rs` under the `// PHASE 04
   commands here` placeholder:
   ```rust
   Analyze {
       #[arg(long, value_parser = parse_kv)]
       filter_tag: Vec<(String, String)>,
       #[arg(long)]
       trigger: Option<String>,       // restrict to one trigger
       #[arg(long, default_value = "10")]
       min_n: u32,
       #[arg(long, default_value = "table")]
       format: AnalyzeFormat,         // table|json
   },
   Propose {
       #[arg(long)]
       trigger: String,
       #[arg(long)]
       new_threshold: f64,
       #[arg(long, value_parser = parse_kv)]
       filter_tag: Vec<(String, String)>,
       #[arg(long)]
       rationale: String,
       #[arg(long, value_delimiter = ',')]
       supporting_plan_ids: Vec<String>,
   },
   Proposals {
       #[arg(long, conflicts_with_all = ["accepted", "rejected"])]
       pending: bool,
       #[arg(long, conflicts_with_all = ["pending", "rejected"])]
       accepted: bool,
       #[arg(long, conflicts_with_all = ["pending", "accepted"])]
       rejected: bool,
   },
   Decide {
       proposal_id: i64,
       #[arg(value_parser = parse_decision)]
       decision: Decision,            // Accept | Reject
       #[arg(long)]
       rationale: String,
   },
   ExportChangelog {
       #[arg(long)]
       since: Option<String>,         // ISO date
   },
   ```
   `parse_kv` from Phase 03 is reused; if Phase 03 hasn't landed,
   bring a local copy (and Phase 03's coordination note will dedupe
   it).

6. **Module layout** under `src/calibration/`:
   - `analyze.rs` — the rate algorithm + proposal generator, plus
     formatters for table and json output.
   - `propose.rs` — wraps the `calibration_proposals` insert.
   - `decide.rs` — updates a proposal's `decision`, `decided_at`,
     `rationale` columns.
   - `changelog.rs` — formats accepted proposals as a SKILL.md
     changelog block (see Acceptance criteria for the exact
     format).

7. **Output format for `analyze`** (table mode):
   ```
   TRIGGER             FIRES  MISSES  FIRE%   SIGNAL  VERDICT
   chain-depth            18      42  30.0%   +0.42   hold
   shared-file-contention 27      33  45.0%   -0.15   raise threshold (4→5)
   risk-concentration      8      52  13.3%   n/a     n=8 < min-n=10
   …
   PROPOSALS (1):
     - shared-file-contention: 4 → 5
       fire% 45.0% / signal -0.15
       supporting plans: 9 (run `skillnet calibration query ...`)
       run `skillnet calibration propose ...` to formalize
   SKEW WARNINGS (0):
   ```
   JSON mode emits the same data as a structured object so the
   skill's `calibrate` mode can consume it programmatically.

8. **Changelog export format** for `export-changelog`. The output is
   intended to be pasted into the bottom of
   `global/multi-phase-plan/SKILL.md`:
   ```markdown
   ### 2026-MM-DD — <trigger-name>: <old> → <new>

   - **Rationale**: <user-supplied rationale from the decide step>
   - **Fire rate at decision**: <pct>
   - **Signal rate at decision**: <signed float>
   - **Supporting plans**: <count>, ids: <comma list>
   - **Filter tags (if any)**: <k=v list>
   ```
   One block per accepted proposal, newest first. `--since YYYY-MM-DD`
   limits to proposals decided on or after the given date.

9. **Integration tests** at `tests/calibration_analyze.rs`:
   - Seed dataset where trigger `T1` fires 15 times, all helpful,
     no failure modes → `analyze` reports positive signal, may
     propose lowering threshold if false-negative count is set up
     to ≥ 2 in the fixture.
   - Seed where `T2` fires 20 times, all dead-weight (verifier marks
     all as surprises) → signal rate strongly negative; `analyze`
     proposes raising threshold.
   - Seed where `T3` fires 5 times → `analyze` does not propose
     (under min-n), emits "n=5 < min-n=10".
   - Seed mixed flavors where `T4` has signal rate +0.5 for codex
     and −0.4 for claude → skew warning fires when run without
     filter tags.
   - `--filter-tag flavor=codex` restricts the dataset correctly.
   - `propose` inserts a row with `decision=pending`.
   - `proposals --pending` lists it; `decide <id> accept --rationale
     "..."` updates it; `proposals --accepted` lists it.
   - `decide` on an already-decided proposal errors clearly.
   - `export-changelog` emits the expected markdown for one accepted
     proposal; `--since` filters correctly.

10. **Run validation**:
    ```sh
    cargo fmt
    cargo clippy --all-targets -- -D warnings
    cargo test
    ```

## Acceptance criteria

- [ ] `src/calibration/analyze.rs` documents the signal-rate formula
      in module-level rustdoc with the same math as Plan step 2.
- [ ] `analyze` emits per-trigger fires, misses, fire%, signal,
      verdict columns in table mode and equivalent JSON in json mode.
- [ ] `analyze` honors `--min-n` and refuses to propose for triggers
      below it.
- [ ] `analyze` proposal rules: positive signal + missed-cases ≥ 2 →
      lower threshold; non-positive signal + fire ≥ 50% → raise
      threshold; otherwise hold.
- [ ] `analyze` "skew check" section runs when no `--filter-tag` is
      passed and emits warnings for tag bands diverging by >±0.3
      with ≥30 fires.
- [ ] `propose` inserts a `calibration_proposals` row with
      `decision=pending`; `proposals --pending` lists it.
- [ ] `decide <id> accept|reject --rationale "..."` updates the row;
      cannot be applied twice.
- [ ] `export-changelog` emits the markdown format from Plan step 8;
      `--since YYYY-MM-DD` filters correctly.
- [ ] `cargo test --test calibration_analyze` covers the eight
      scenarios in Plan step 9.
- [ ] `cargo clippy --all-targets -- -D warnings` and `cargo fmt
      --check` are clean.

## Files likely touched

- `src/cli/args.rs` (insert at the `// PHASE 04 commands here`
  placeholder; add `AnalyzeFormat`, `Decision` enums)
- `src/commands/calibration.rs` (insert dispatch arms at the
  placeholder)
- `src/calibration/mod.rs` (+ `pub mod analyze; pub mod propose; pub
  mod decide; pub mod changelog;`)
- `src/calibration/analyze.rs` (new — the substantive deliverable)
- `src/calibration/propose.rs` (new)
- `src/calibration/decide.rs` (new)
- `src/calibration/changelog.rs` (new)
- `tests/calibration_analyze.rs` (new)

## Pitfalls

- **Coordinating clap insertions with Phase 03.** Use the `// PHASE
  04 commands here` placeholder Phase 02 left. If Phase 03 has
  already landed and removed the placeholder, restore your own
  insertion location and resolve the merge by adding your variants
  after 03's.
- **Defining "helpful" and "failure_mode" from `surprises` text.**
  The `surprises` column is free text from the verifier. Don't try
  to NLP-parse it. The skill convention (defined in Phase 05) is
  that surprises use a structured prefix: `dead-weight:
  <trigger-name>: <note>` for false positives and
  `missed-signal: <expected-trigger-name>: <note>` for false
  negatives. Anything else is treated as informational only.
  Document this convention in `analyze.rs`'s rustdoc so the
  contract is explicit.
- **Min-N too aggressive too early.** With 10 fires as the floor,
  early on the analyzer will emit nothing. That's correct — we
  shouldn't propose threshold changes from 3 data points. Document
  the silence as expected in the analyzer's own help text.
- **Signal rate of `n/a` vs `0.0`.** Distinguish in output. `n/a`
  means "below min-n, don't trust"; `0.0` means "computed but
  neutral". Conflating them confuses readers.
- **`decide` on a missing or already-decided proposal.** Both error
  cases need clear messages. Don't silently update or insert.
- **Floating-point thresholds in `triggers.threshold`.** The column
  is `REAL`. Some triggers will use integer thresholds (phase count
  = 10), some ratio (4.0). Use `f64` everywhere in the analyzer;
  format integers as `4` not `4.0` in output for readability (a
  small `pretty_threshold` helper).
- **Don't ship the analyzer without min-n enforcement.** Without
  it, a single misbehaving plan can drive a "threshold raise"
  proposal. Min-N is the bullshit filter.

## Reference

- Parent plan (in ai-skills): `ai-skills/docs/planning/calibration-loop/README.md`.
- Foundation phases: `01-sqlite-schema-storage.md`,
  `02-cli-record-verify.md`.
- Parallel sister phase (shared files): `03-cli-inspect-tag-housekeeping.md`.
- The skill body that consumes `analyze` output (in ai-skills):
  `06-base-skill-hooks-calibrate-mode.md`.
- Crate publication that ships this surface: `07-skillnet-crate-publication.md`.
- Rust integer/float formatting: `std::fmt::Display`,
  `format!("{:.1}%", value * 100.0)`.
