# Phase 02 — CLI: calibration record + verify

> **Recommended Codex model: GPT 5.5 medium**
>
> Wire the first two `skillnet calibration` subcommands — `record` and
> `verify` — that ingest a sidecar `.calibration.json` into the SQLite
> store. Moderate complexity: clap subcommand plumbing, JSON
> deserialization with serde, and idempotent upsert logic. Leaf-ish
> role: this phase implements one well-specified surface and a clear
> data contract; design questions are settled in the parent plan. A
> `low` tier would skip the idempotency tests and the malformed-sidecar
> handling; `high` is unnecessary.

## Working tree

`/data/nvme0/can/Projects/ai-skills`.

## Goal

`skillnet calibration record <plan-dir>` reads
`<plan-dir>/.calibration.json`, validates it, and inserts/updates rows
in the calibration database. Re-running on the same plan id is
idempotent. `skillnet calibration verify <plan-dir>` reads the
optional `verify` section of the same sidecar and inserts a row into
`verifications`, updating the `outcome` auto-tag. Both commands handle
a missing or malformed sidecar with a clear error and a non-zero exit.

## Why this matters now

Phase 01 stood up the database; without a write path it's inert.
Phases 03 and 04 (inspect, analyze) need real rows to operate on, and
Phase 06 wires the skill to invoke these commands at end-of-plan and
end-of-verify time. The sidecar contract this phase defines is what
the skill body (Phase 05) writes against — both surfaces must agree
exactly. Originating context: the design conversation specified JSON,
specified the contents of the sidecar (plan metadata, triggers,
phases, meta-heuristics-fired, tags, optional verify section), and
specified that the skill never writes SQLite directly.

## Out of scope

- Any read/inspect command (`tag`, `untag`, `show`, `query`,
  `migrate`, `vacuum`, `export`) — Phase 03.
- Any analysis/proposal command (`analyze`, `propose`, `proposals`,
  `decide`, `export-changelog`) — Phase 04.
- The skill body that *writes* the sidecar — Phase 05.
- The hook that invokes `skillnet calibration record` — Phase 06.
- Authoring the sidecar from scratch in tests beyond what's needed
  to exercise this command's surface.

## Plan

1. **Rebase before starting**: `git pull --rebase` (or local
   equivalent). Phase 02 is in Wave 1 by itself but coordinates with
   Phase 01's exact module layout.

2. **Define the sidecar schema** in `src/calibration/sidecar.rs`:
   ```rust
   #[derive(Deserialize, Serialize, Debug)]
   pub struct Sidecar {
       pub schema_version: u32,        // currently 1; reject others
       pub plan: PlanRecord,
       pub triggers: Vec<TriggerRecord>,
       pub phases: Vec<PhaseRecord>,
       pub meta_heuristics_fired: Vec<String>,
       pub tags: BTreeMap<String, String>,
       #[serde(default)]
       pub verify: Option<VerifyRecord>,
   }

   #[derive(Deserialize, Serialize, Debug)]
   pub struct PlanRecord {
       pub id: String,                  // uuid; stable across record runs
       pub name: String,
       pub flavor: String,              // codex|claude|mixed
       pub worktype: Option<String>,
       pub created_at: i64,             // unix ts
       pub phase_count: u32,
       pub wave_count: u32,
       pub max_chain_depth: u32,
       pub repo_spread: u32,
       pub routing_dist: BTreeMap<String, u32>,
       pub shape_hash: String,
   }

   #[derive(Deserialize, Serialize, Debug)]
   pub struct TriggerRecord {
       pub name: String,
       pub input_value: f64,
       pub threshold: f64,
       pub fired: bool,
       pub section_added: Option<String>,
   }

   #[derive(Deserialize, Serialize, Debug)]
   pub struct PhaseRecord {
       pub ordinal: u32,
       pub slug: String,
       pub routing_tier: String,
       pub files: Vec<String>,
   }

   #[derive(Deserialize, Serialize, Debug)]
   pub struct VerifyRecord {
       pub verified_at: i64,
       pub elapsed_seconds: Option<i64>,
       pub outcome: String,             // shipped|partial|abandoned
       pub phase_outcomes: BTreeMap<String, String>,
       pub emergency_changes: Option<serde_json::Value>,
       pub surprises: Option<String>,
   }
   ```
   Provide a `Sidecar::load(plan_dir: &Path)` helper that reads
   `.calibration.json`, parses it, and returns a clear error
   distinguishing missing-file from malformed-json from
   schema-version-mismatch.

3. **Implement `record`** in `src/calibration/record.rs`:
   - `pub fn run(plan_dir: &Path, db: &mut Db) -> anyhow::Result<()>`.
   - Load sidecar.
   - Inside a transaction: upsert `plans` row by id; delete then
     re-insert child rows (`triggers`, `phases`, `tags`) — simplest
     idempotency that matches the source-of-truth-is-sidecar model.
   - Apply auto-tags from the sidecar's plan metadata:
     `flavor:<flavor>`, `worktype:<worktype>` if present,
     `scope:<derived>` (from `repo_spread`: 1→single-repo, 2→multi-repo,
     ≥3→cross-org), `risk:<derived>` (from `routing_dist`: presence of
     `max` → high; only `low`/`medium` → low; else mixed), one
     `signal:<reason>` per entry in `meta_heuristics_fired`.
   - User tags from `sidecar.tags` are also written; if a key exists
     in both auto and user, user wins (overwrite).
   - Skip the `verifications` table entirely if `sidecar.verify` is
     present (record cmd ignores it; verify cmd handles it).
   - Print one line on success: `recorded <plan_id> (<n> triggers, <m>
     phases, <k> tags)`.

4. **Implement `verify`** in `src/calibration/record.rs` (same file
   for cohesion):
   - `pub fn run_verify(plan_dir: &Path, db: &mut Db) -> anyhow::Result<()>`.
   - Load sidecar; require `verify` section present (else error:
     "no verify section in sidecar").
   - Inside a transaction: insert or replace `verifications` row by
     `plan_id`; upsert `outcome:<value>` tag (deleting any prior
     `outcome:` tag for this plan).
   - Print one line on success: `verified <plan_id>: <outcome>
     (<pass>/<total> phases passed)`.

5. **Add the clap surface** in `src/cli/args.rs`:
   ```rust
   #[derive(Subcommand)]
   pub enum Command {
       // … existing variants …
       Calibration(CalibrationArgs),
   }

   #[derive(Args)]
   pub struct CalibrationArgs {
       #[command(subcommand)]
       pub command: CalibrationCommand,
   }

   #[derive(Subcommand)]
   pub enum CalibrationCommand {
       Record { plan_dir: Utf8PathBuf },
       Verify { plan_dir: Utf8PathBuf },
       // 03 will add: Tag, Untag, Show, Query, Migrate, Vacuum, Export
       // 04 will add: Analyze, Propose, Proposals, Decide, ExportChangelog
   }
   ```
   Use a `// PHASE 03` / `// PHASE 04` placeholder comment so the
   downstream phases know exactly where to slot their additions; this
   reduces merge conflicts when 03 and 04 land in parallel.

6. **Add the dispatch** in `src/commands/calibration.rs`:
   ```rust
   pub fn run(args: CalibrationArgs) -> anyhow::Result<()> {
       let mut db = Db::open(&Db::default_path())?;
       match args.command {
           CalibrationCommand::Record { plan_dir } =>
               calibration::record::run(plan_dir.as_std_path(), &mut db),
           CalibrationCommand::Verify { plan_dir } =>
               calibration::record::run_verify(plan_dir.as_std_path(), &mut db),
           // 03/04 arms slotted here
       }
   }
   ```
   Wire `Command::Calibration` in the top-level dispatcher
   (`src/commands/mod.rs` or wherever the current pattern lives —
   check `src/main.rs` to confirm).

7. **Write integration tests** at `tests/calibration_record.rs`:
   - Fixture: a minimal valid `.calibration.json` in a `tempdir`.
   - Run `record` via `assert_cmd`; assert exit 0; open the db and
     check rows.
   - Run `record` a second time; assert no duplicate triggers/phases.
   - Mutate the sidecar (drop a trigger); re-run `record`; assert the
     dropped trigger row is gone (delete-and-reinsert idempotency).
   - Run `verify` without a verify section; assert exit non-zero with
     a clear stderr.
   - Add a verify section; re-run `verify`; assert the
     `verifications` row exists and the `outcome:` tag is updated.
   - Malformed JSON: assert clear error mentioning the parse failure
     location.
   - Wrong `schema_version`: assert clear error naming the version
     mismatch.

8. **Run validation**:
   ```sh
   cargo fmt
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

## Acceptance criteria

- [ ] `src/calibration/sidecar.rs` defines the `Sidecar`,
      `PlanRecord`, `TriggerRecord`, `PhaseRecord`, and `VerifyRecord`
      types with the fields listed in Plan step 2.
- [ ] `Sidecar::load` distinguishes missing-file, malformed-JSON, and
      schema-version-mismatch errors.
- [ ] `skillnet calibration record <plan-dir>` succeeds against a
      valid sidecar and writes rows to `plans`, `triggers`, `phases`,
      `tags`.
- [ ] Re-running `record` on the same plan id is idempotent (row
      counts in `plans` unchanged; child rows match the sidecar's
      current state, not the union of all prior runs).
- [ ] Auto-tags `flavor:*`, `scope:*`, `risk:*`, `signal:*`, and
      `worktype:*` (when present) are applied.
- [ ] `skillnet calibration verify <plan-dir>` succeeds when the
      sidecar's `verify` section is present and fails clearly when it
      isn't.
- [ ] The `outcome:` tag is updated by `verify` (prior value
      replaced, not duplicated).
- [ ] `cargo test --test calibration_record` covers: happy path,
      idempotency, missing verify section, malformed JSON,
      schema_version mismatch.
- [ ] `cargo clippy --all-targets -- -D warnings` and `cargo fmt
      --check` are clean.

## Files likely touched

- `src/calibration/mod.rs` (+ `pub mod sidecar; pub mod record;`)
- `src/calibration/sidecar.rs` (new)
- `src/calibration/record.rs` (new)
- `src/cli/args.rs` (+ `Calibration(CalibrationArgs)` variant and
  `CalibrationCommand` enum with `Record` and `Verify` arms plus
  placeholder comments for 03/04)
- `src/commands/calibration.rs` (new)
- `src/commands/mod.rs` (+ `pub mod calibration;`)
- `src/main.rs` (+ dispatcher arm if needed; verify the current
  pattern in `src/commands/mod.rs`)
- `tests/calibration_record.rs` (new)

## Pitfalls

- **`src/cli/args.rs` and `src/commands/calibration.rs` are shared
  with Phases 03 and 04.** Both later phases will rebase against this
  one. Leave the placeholder comments (`// PHASE 03 commands here`,
  `// PHASE 04 commands here`) in clearly identifiable spots in both
  the enum and the dispatcher so 03/04 can insert without conflict.
- **Idempotency by upsert is fragile if child rows have no unique
  keys.** `triggers` and `phases` have surrogate `AUTOINCREMENT`
  primary keys, so an UPSERT would just append. Use the
  delete-children-then-reinsert pattern inside a transaction — the
  sidecar is the source of truth, and the database is a derivable
  index over it.
- **Transaction boundaries matter.** If you delete `triggers` rows
  then panic before reinsert, the row is gone. Use
  `conn.transaction()?` with `tx.commit()?` at the end; any error
  rolls back.
- **`Utf8PathBuf` vs `PathBuf`.** This repo uses `camino::Utf8PathBuf`
  in clap args (see existing args.rs). Convert to `std::path::Path`
  with `as_std_path()` at the boundary; don't let `camino` leak into
  the calibration module's signatures.
- **Auto-tag derivation from `routing_dist`.** The "risk" tag rule:
  any phase routed to `max` → `risk:high`; any phase routed to `high`
  but none to `max` → `risk:mixed`; only `low` and `medium` → `risk:low`.
  Get this wrong once and analysis slices will be wrong forever
  (until you re-record). Cover it with a test.
- **`signal:` tags are 1-to-many.** A plan can be captured by multiple
  meta-heuristics; emit one tag per heuristic. The tag table's
  composite PK (plan_id, key, value) makes this safe.
- **Don't print noisy debug info on success.** The skill calls this
  command from its workflow; a one-line success message is enough.
  Verbose logging should be gated behind `--verbose` (deferred — not
  in this phase's scope unless trivial).

## Reference

- Parent plan: `docs/planning/calibration-loop/README.md`.
- Schema source-of-truth: `data/multi-phase-plan/schema/001-initial.sql`
  (delivered by Phase 01).
- Sister phases that share `src/cli/args.rs`:
  `03-cli-inspect-tag-housekeeping.md`,
  `04-cli-analyze-propose-decide.md`.
- Phase that writes the sidecar this command consumes:
  `05-base-skill-heuristics-rewrite.md`.
- Phase that calls this command from the skill workflow:
  `06-base-skill-hooks-calibrate-mode.md`.
- serde + rusqlite patterns: existing `src/catalog/` modules in this
  repo are a reference for serde usage.
