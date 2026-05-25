# Phase 03 — CLI: inspect, tag, query, housekeeping

> **Recommended Codex model: GPT 5.5 low**
>
> Mechanical clap + SQL work: half a dozen small subcommands that
> wrap straightforward queries against the schema laid down in Phase 01. No design content, no algorithmic decisions, no log
> interpretation. The only judgment calls are output formatting
> (table vs json) and tag-key validation, both of which the Plan
> resolves explicitly. A `medium` tier would over-engineer the output
> layer; `low` matches the work.

## Working tree

`/data/nvme0/can/Projects/skillnet` (the new standalone crate at
`ssh://git@codeberg.org/caniko/skillnet.git`). All CLI work in
phases 01–04 and 07–08 happens here; the `ai-skills` repo is the
_consumer_, touched only by phases 05, 06, 09.

## Goal

Eight subcommands on `skillnet calibration` that let a human (or the
calibrate-mode workflow) inspect and manage the dataset without
touching SQLite directly:

- `tag <plan-id> <key=value>...` — add user tags.
- `untag <plan-id> <key=value>...` — remove user tags.
- `show <plan-id>` — full dump of a plan's rows (json).
- `query [--tag k=v]... [--trigger NAME] [--fired|--missed] [--limit N]` —
  slice the dataset; `--format json|table` (default table).
- `migrate` — apply pending schema migrations (exposes the
  Phase 01 runner as a manual command for ops use).
- `vacuum` — `VACUUM` the database.
- `export [--format jsonl] [--out PATH]` — dump the full dataset.

## Why this matters now

The calibration loop is only useful if the dataset is inspectable.
Without `query` and `show`, the only way to verify a recorded plan is
to open SQLite manually — which defeats the "CLI is the single
interaction surface" principle that ai-skills' `multi-phase-plan`
skill depends on (Phase 06 hooks shell out to skillnet; SKILL.md
never opens the db). `tag` exists so users can annotate plans after
the fact ("this was the auth refactor", "this was a spike"). `migrate`
and `vacuum` are operational escape hatches we will want before we
want them.

Public-crate context: these subcommands ship in the first published
version of `skillnet` (Phase 07). Their CLI surface is part of the
crate's public API — additions are easy later, removals are
breaking. Lock the surface conservatively; defer optional flags
(e.g., `--force` on export overwrite) unless they're clearly needed.

Runs in parallel with Phase 04 in Wave 2. The two phases share
`src/cli/args.rs` and `src/commands/calibration.rs` — coordinate via
the placeholder comments Phase 02 left.

## Out of scope

- Any analysis logic (per-trigger fire/signal rates,
  threshold-proposal generation) — Phase 04.
- Writing the sidecar (lives in ai-skills' SKILL.md hook) — Phase 06.
- New tables or schema changes. If a command would need a new column,
  it's the wrong command for this phase; punt to a future phase with
  its own migration.
- Crate publication, Cargo metadata, Codeberg CI — Phase 07.
- Nix HM module — Phase 08.
- Pretty TUI / interactive selection. CLI output only.

## Plan

1. **Rebase against Phase 02**: `git pull --rebase` (or wait for 02 to
   land in main). Read the post-02 `src/cli/args.rs` to see where the
   `// PHASE 03 commands here` placeholder lives.

2. **Add clap variants** in `src/cli/args.rs` under the placeholder:

   ```rust
   Tag {
       plan_id: String,
       #[arg(required = true, value_parser = parse_kv)]
       tags: Vec<(String, String)>,
   },
   Untag {
       plan_id: String,
       #[arg(required = true, value_parser = parse_kv)]
       tags: Vec<(String, String)>,
   },
   Show {
       plan_id: String,
   },
   Query {
       #[arg(long, value_parser = parse_kv)]
       tag: Vec<(String, String)>,
       #[arg(long)]
       trigger: Option<String>,
       #[arg(long, conflicts_with = "missed")]
       fired: bool,
       #[arg(long, conflicts_with = "fired")]
       missed: bool,
       #[arg(long, default_value = "100")]
       limit: u32,
       #[arg(long, default_value = "table")]
       format: QueryFormat,
   },
   Migrate,
   Vacuum,
   Export {
       #[arg(long, default_value = "jsonl")]
       format: ExportFormat,
       #[arg(long)]
       out: Option<Utf8PathBuf>,
   },
   ```

   Add a `parse_kv` helper that splits `key=value` and validates the
   key against `^[a-z][a-z0-9_-]*$` — rejects empty keys, mixed-case
   keys, etc. Add `QueryFormat` (`Table`, `Json`) and `ExportFormat`
   (`Jsonl`) enums (singleton enum lets us add `csv` etc. later
   without breaking the surface).

3. **Add dispatch arms** in `src/commands/calibration.rs`. Each arm
   lives in its own module under `src/calibration/`:
   - `src/calibration/tag.rs` — `add_tags`, `remove_tags`. Tags are
     just rows in the `tags` table; UPSERT for add (primary key
     (plan_id, key, value) handles dedup), DELETE for remove. Reject
     attempts to tag a non-existent plan_id with a clear error.
     **Auto-tag protection**: refuse to `untag` an auto-tag key
     (`flavor`, `scope`, `risk`, `signal`, `worktype`, `outcome`)
     with a clear error explaining these are derived; user can
     overwrite via `tag` but not delete.
   - `src/calibration/query.rs` — `show`, `query`.
     - `show` fetches the plan, all triggers, all phases, all tags,
       any verifications, and emits JSON to stdout.
     - `query` builds a SQL `SELECT plans.* FROM plans WHERE …`
       with conditions composed from the args:
       - each `--tag k=v` becomes an `EXISTS (SELECT 1 FROM tags …)`.
       - `--trigger NAME [--fired|--missed]` becomes an `EXISTS
(SELECT 1 FROM triggers WHERE name=? AND fired=?)`.
       - Use parameterized queries; never string-interpolate user
         input.
       - Table output: id, created_at (iso), name, flavor, worktype,
         tags-summary (e.g., `flavor=codex risk=mixed`). JSON
         output: full plans row + tags array.
   - `src/calibration/housekeeping.rs` — `migrate`, `vacuum`,
     `export`. `migrate` reuses the Phase 01 runner (call
     `Db::open(default_path())?` then return; `open` already runs
     migrations — but expose a separate path that runs without
     opening any other connection, for ops use). `vacuum` runs
     `VACUUM` (must be outside a transaction). `export` streams every
     `plans` row joined with its children as one JSONL line per
     plan; writes to stdout if `--out` is absent.

4. **Output formatting helpers** in `src/calibration/format.rs`:
   - Tables use a simple aligned-columns formatter (look at existing
     `src/cli/` or `src/commands/status.rs` for the repo's existing
     pattern; reuse if it exists, otherwise write a thin helper —
     don't pull in a new dep for this).
   - JSON output uses `serde_json::to_string_pretty` for `show` and
     compact for `export`.

5. **Integration tests** at `tests/calibration_inspect.rs`:
   - Seed a small dataset (2–3 plans with different flavors/risks)
     via `skillnet calibration record`.
   - `tag` adds a user tag; `show` reflects it; `untag` removes it.
   - `untag flavor=codex` fails with the auto-tag protection error.
   - `query --tag flavor=codex` returns only the codex-flavored plans.
   - `query --trigger chain-depth --fired` returns plans where that
     trigger fired (use a seed with one match).
   - `query --tag flavor=codex --tag risk=mixed` ANDs the conditions.
   - `query --format json` round-trips through `serde_json`.
   - `export --format jsonl` emits N lines for N plans, each
     deserializable as the full plan record.
   - `migrate` is a no-op on an up-to-date db (exits 0).
   - `vacuum` succeeds on a small db.

6. **Run validation**:
   ```sh
   cargo fmt
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

## Acceptance criteria

- [ ] All eight subcommands appear under `skillnet calibration --help`.
- [ ] `tag` rejects malformed `key=value` strings (empty key,
      mixed-case, etc.) with a clear error.
- [ ] `untag` refuses to remove auto-tag keys (`flavor`, `scope`,
      `risk`, `signal`, `worktype`, `outcome`).
- [ ] `show <plan-id>` returns full plan rows as JSON; returns
      non-zero with a clear error for an unknown id.
- [ ] `query --tag flavor=codex --tag risk=mixed` ANDs the conditions.
- [ ] `query` supports `--trigger NAME` with optional `--fired` /
      `--missed` (mutually exclusive).
- [ ] `query --format json` and `query --format table` both work; table
      is default.
- [ ] `export --format jsonl` emits one line per plan, each containing
      the full plan + children + tags.
- [ ] `migrate` is a no-op on an up-to-date db; exits 0.
- [ ] `vacuum` succeeds.
- [ ] `cargo test --test calibration_inspect` covers all bullets
      above.
- [ ] `cargo clippy --all-targets -- -D warnings` and `cargo fmt
    --check` are clean.

## Files likely touched

- `src/cli/args.rs` (insert at the `// PHASE 03 commands here`
  placeholder; add `parse_kv`, `QueryFormat`, `ExportFormat`)
- `src/commands/calibration.rs` (insert dispatch arms at the
  placeholder)
- `src/calibration/mod.rs` (+ `pub mod tag; pub mod query; pub mod
housekeeping; pub mod format;`)
- `src/calibration/tag.rs` (new)
- `src/calibration/query.rs` (new)
- `src/calibration/housekeeping.rs` (new)
- `src/calibration/format.rs` (new)
- `tests/calibration_inspect.rs` (new)

## Pitfalls

- **Shared file with Phase 04.** `src/cli/args.rs` and
  `src/commands/calibration.rs` are also touched by Phase 04 in the
  same wave. Coordinate placement: 03 inserts at `// PHASE 03
commands here`, 04 inserts at `// PHASE 04 commands here`. As long
  as both phases respect their placeholders, the merge is trivial.
  If you remove the other phase's placeholder, you'll break their
  workflow.
- **SQL injection via tag keys/values.** Every tag goes through a
  parameterized query. Don't compose SQL strings even for `query`'s
  multi-condition WHERE — use `rusqlite::params_from_iter` and add
  one `?` per condition.
- **`VACUUM` cannot run in a transaction.** rusqlite's default mode
  often wraps statements in implicit transactions; ensure `vacuum`
  uses `conn.execute_batch("VACUUM;")` or explicitly commits any
  open transaction first. Symptom if missed: error
  "cannot VACUUM from within a transaction".
- **Auto-tag protection is per-key, not per-tag-pair.** If user has
  manually overwritten `flavor:codex` to `flavor:claude` via `tag`,
  `untag flavor=claude` should still fail. The rule guards the _key_.
- **Table output for `query` can be very wide.** Cap the tag-summary
  column to ~40 chars; truncate with `…` on overflow. Don't try to
  pretty-print 200 columns.
- **`export --out` should refuse to overwrite an existing file
  without `--force`.** Not in the bullet list above but good
  hygiene; if you implement it, document it in `--help`. Skip if it
  adds friction — punt to a follow-up.
- **`Show` for a plan that has no verify row.** Emit `"verify":
null`, not an error. Verify is optional.

## Reference

- Parent plan (in ai-skills): `ai-skills/docs/planning/calibration-loop/README.md`.
- Foundation: `01-sqlite-schema-storage.md`,
  `02-cli-record-verify.md`.
- Parallel sister phase: `04-cli-analyze-propose-decide.md` (shares
  `src/cli/args.rs` and `src/commands/calibration.rs`).
- Crate publication that consumes this surface: `07-skillnet-crate-publication.md`.
- rusqlite `params_from_iter`:
  <https://docs.rs/rusqlite/latest/rusqlite/macro.params_from_iter.html>.
