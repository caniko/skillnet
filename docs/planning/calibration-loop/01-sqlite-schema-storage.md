# Phase 01 — SQLite schema and storage module

> **Recommended Codex model: GPT 5.5 medium**
>
> Foundation work: add a SQLite dependency, write the initial schema
> migration, and stand up a small Rust module that opens the database
> in WAL mode and runs pending migrations. The schema design itself is
> done (see Plan below); the work is mechanical Rust scaffolding plus
> careful migration-runner hygiene. Moderate complexity, leaf role —
> a `low` tier would skip the test fixtures and ship a broken
> migration runner; `high` is overkill for code this self-contained.

## Working tree

`/data/nvme0/can/Projects/ai-skills` (this repo). Single-repo phase;
no external coordination.

## Goal

A working `src/calibration/` module exposing `Db::open(path)` that
opens (or creates) `data/multi-phase-plan/calibration.sqlite`, enables
WAL mode, and applies any pending migrations from
`data/multi-phase-plan/schema/`. A round-trip integration test inserts
a synthetic `plans` row through raw SQL and reads it back. No CLI
surface yet — this phase only delivers the foundation that Phase 02
will build on.

## Why this matters now

Every later code phase (02, 03, 04) depends on this module. The
calibration loop's entire value proposition depends on the schema
being right the first time — schema migrations on a live database are
painful and the dataset will start accumulating real plan rows the
moment Phase 02 ships. Originating context: the design conversation
locked the storage location (`data/multi-phase-plan/calibration.sqlite`),
the per-skill scope, and the schema (see Plan step 3 below).

## Out of scope

- Any CLI subcommand. The `Calibration` clap variant is added in
  Phase 02.
- The sidecar `.calibration.json` parser. Phase 02.
- Any analysis logic. Phase 04.
- Schema for _other_ skills. This db is `multi-phase-plan`-specific by
  design; future skills get their own db.
- Cross-machine sync mechanisms. The db lives in the repo and syncs
  via git.

## Plan

1. **Add dependencies** to `Cargo.toml`:

   ```toml
   rusqlite = { version = "0.31", features = ["bundled"] }
   serde_json = "1.0"
   uuid = { version = "1.10", features = ["v4", "serde"] }
   ```

   Use `bundled` to avoid linking against system SQLite.

2. **Create the directory layout**:

   ```
   data/
   └── multi-phase-plan/
       ├── schema/
       │   └── 001-initial.sql
       └── .gitkeep       # ensure the data dir is tracked even before the db exists
   ```

   The actual `.sqlite` file is created at runtime; it is **not**
   tracked in `.gitignore` (we want the dataset committed).

3. **Write `data/multi-phase-plan/schema/001-initial.sql`** with the
   full schema from the design:

   ```sql
   CREATE TABLE schema_versions (
       version    INTEGER PRIMARY KEY,
       applied_at INTEGER NOT NULL
   );

   CREATE TABLE plans (
       id              TEXT PRIMARY KEY,
       created_at      INTEGER NOT NULL,
       name            TEXT NOT NULL,
       path            TEXT NOT NULL,
       flavor          TEXT NOT NULL,
       worktype        TEXT,
       phase_count     INTEGER NOT NULL,
       wave_count      INTEGER NOT NULL,
       max_chain_depth INTEGER NOT NULL,
       repo_spread     INTEGER NOT NULL,
       routing_dist    TEXT NOT NULL,    -- json
       shape_hash      TEXT NOT NULL,
       capture_reasons TEXT NOT NULL     -- json array
   );

   CREATE TABLE triggers (
       id            INTEGER PRIMARY KEY AUTOINCREMENT,
       plan_id       TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
       name          TEXT NOT NULL,
       input_value   REAL NOT NULL,
       threshold     REAL NOT NULL,
       fired         INTEGER NOT NULL,    -- bool
       section_added TEXT
   );
   CREATE INDEX idx_triggers_name_fired ON triggers(name, fired);

   CREATE TABLE phases (
       id           INTEGER PRIMARY KEY AUTOINCREMENT,
       plan_id      TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
       ordinal      INTEGER NOT NULL,
       slug         TEXT NOT NULL,
       routing_tier TEXT NOT NULL,
       files        TEXT NOT NULL          -- json array
   );
   CREATE INDEX idx_phases_plan ON phases(plan_id);

   CREATE TABLE verifications (
       id                INTEGER PRIMARY KEY AUTOINCREMENT,
       plan_id           TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
       verified_at       INTEGER NOT NULL,
       elapsed_seconds   INTEGER,
       outcome           TEXT NOT NULL,
       phase_outcomes    TEXT NOT NULL,   -- json
       emergency_changes TEXT,            -- json
       surprises         TEXT
   );

   CREATE TABLE tags (
       plan_id TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
       key     TEXT NOT NULL,
       value   TEXT NOT NULL,
       PRIMARY KEY (plan_id, key, value)
   );
   CREATE INDEX idx_tags_kv ON tags(key, value);

   CREATE TABLE calibration_proposals (
       id                  INTEGER PRIMARY KEY AUTOINCREMENT,
       proposed_at         INTEGER NOT NULL,
       trigger_name        TEXT NOT NULL,
       current_threshold   REAL NOT NULL,
       proposed_threshold  REAL NOT NULL,
       supporting_plan_ids TEXT NOT NULL,   -- json array
       fire_rate           REAL NOT NULL,
       signal_rate         REAL NOT NULL,
       filter_tags         TEXT,            -- json {k: v}
       decision            TEXT NOT NULL,   -- pending|accepted|rejected
       decided_at          INTEGER,
       rationale           TEXT
   );
   CREATE INDEX idx_proposals_decision ON calibration_proposals(decision);
   ```

4. **Create `src/calibration/mod.rs`** with module declarations and
   re-exports:

   ```rust
   pub mod db;
   pub use db::Db;
   ```

5. **Create `src/calibration/db.rs`** with:
   - `pub struct Db { conn: rusqlite::Connection }`
   - `pub fn open(path: &Path) -> anyhow::Result<Db>` — creates
     parent dirs if missing, opens connection, sets `PRAGMA
journal_mode=WAL`, sets `PRAGMA foreign_keys=ON`, runs
     `migrate()`.
   - `fn migrate(&mut self) -> anyhow::Result<()>` — reads
     `data/multi-phase-plan/schema/*.sql` in sorted order, checks
     `schema_versions` table (creating it first if missing), applies
     any unseen migrations in a transaction, records each version.
   - `pub fn default_path() -> PathBuf` — returns the repo-relative
     default path. Resolves the repo root via `$AI_SKILLS_REPO` env
     var, falling back to the compiled-in path
     `/data/nvme0/can/Projects/ai-skills`.
   - Migration filenames must match `NNN-<desc>.sql`; reject any other
     filenames with a clear error.

6. **Register the module** in `src/main.rs`:

   ```rust
   mod calibration;
   ```

7. **Write the round-trip integration test** at
   `tests/calibration_db.rs`:
   - Use `tempfile::tempdir` for the db path.
   - Open with `Db::open`; assert WAL mode is active (`PRAGMA
journal_mode` returns `wal`).
   - Insert a synthetic `plans` row via raw SQL; read it back; assert
     fields round-trip including the JSON columns.
   - Run `open` a second time against the same path; assert it does
     **not** re-apply migration 001.
   - Insert a `tags` row; delete the plan; assert the tag cascaded.

8. **Run validation**:
   ```sh
   cargo fmt
   cargo clippy --all-targets -- -D warnings
   cargo test --test calibration_db
   cargo test
   ```

## Acceptance criteria

- [ ] `Cargo.toml` lists `rusqlite` (bundled), `serde_json`, and `uuid`.
- [ ] `data/multi-phase-plan/schema/001-initial.sql` exists and matches
      the schema in Plan step 3 byte-for-byte (no silent renames).
- [ ] `src/calibration/mod.rs` and `src/calibration/db.rs` exist and
      compile.
- [ ] `Db::open` creates parent directories if missing.
- [ ] `Db::open` sets `journal_mode=WAL` and `foreign_keys=ON`.
- [ ] Re-opening the db does not re-apply migration 001 (idempotent).
- [ ] `cargo test --test calibration_db` passes.
- [ ] `cargo clippy --all-targets -- -D warnings` is clean.
- [ ] `cargo fmt --check` is clean.
- [ ] `tests/calibration_db.rs` exercises the cascade delete on `tags`.

## Files likely touched

- `Cargo.toml` (+3 dependencies)
- `Cargo.lock` (auto-updated)
- `data/multi-phase-plan/schema/001-initial.sql` (new)
- `data/multi-phase-plan/.gitkeep` (new)
- `src/calibration/mod.rs` (new)
- `src/calibration/db.rs` (new)
- `src/main.rs` (+ `mod calibration;`)
- `tests/calibration_db.rs` (new)

## Pitfalls

- **Bundled vs system SQLite.** `rusqlite` defaults to dynamic linking
  against the system library. Use `features = ["bundled"]` so the
  build is reproducible across machines. Symptom if missed: build
  fails on systems without `libsqlite3-dev`. Recovery: add the
  feature flag.
- **`PRAGMA journal_mode=WAL` is per-connection only on the first
  set, persistent after.** Setting it inside `Db::open` is fine; the
  pragma persists in the file. Don't assert WAL mode without first
  forcing a write — newly created dbs read back the mode correctly,
  but cargo-test-cached temp paths sometimes don't.
- **Migration filename sort order.** Use lexical sort, not numeric.
  `001-…` and `002-…` sort correctly; `1-…` and `10-…` do not.
  Enforce the three-digit prefix in the migration runner with a
  regex; reject mismatches with a clear error.
- **Foreign keys are off by default in SQLite.** Setting `PRAGMA
foreign_keys=ON` is required for the cascade tests to work.
  Symptom if missed: cascade delete test fails silently (the tag row
  survives).
- **`data/multi-phase-plan/calibration.sqlite` should not be in
  `.gitignore`.** Check existing `.gitignore` rules. If any pattern
  like `*.sqlite` exists at repo root, add a negation for this
  specific path. The dataset is the artifact we're committing.
- **Test isolation.** Two `cargo test` invocations against the same
  default path will share state. Use `tempfile::tempdir` for every
  test; never let a test open the real `data/multi-phase-plan/`
  path.

## Reference

- Plan README (this set): `docs/planning/calibration-loop/README.md` —
  the design rationale and the schema source-of-truth.
- Next phase that consumes this module: `02-cli-record-verify.md`.
- rusqlite docs: <https://docs.rs/rusqlite>.
- SQLite WAL mode: <https://www.sqlite.org/wal.html>.
