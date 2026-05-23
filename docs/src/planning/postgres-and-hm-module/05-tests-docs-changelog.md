# Phase 05 — Tests, docs, changelog

> **Recommended Codex model: GPT 5.5 medium**
>
> Closeout phase: integration test parameterised across both backends,
> README/MIGRATION updates, CHANGELOG entry. The shape is largely
> mechanical once phases 01–04 are in. Medium suffices.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`, with phases 01–04 landed.

## Goal

Prove the Postgres backend behaves equivalently to SQLite on the round-trip
cases the SQLite tests already cover, and document the new feature for end
users.

## Why

Without parameterised tests, the Postgres backend can drift silently from
the SQLite one (a `JSONB` cast issue, a `RETURNING id` regression). Docs
keep the new surface discoverable.

## Out of scope

- Performance benchmarks.
- Postgres-specific features beyond schema parity (no materialised views,
  no LISTEN/NOTIFY).

## Plan

1. **Test harness.** Extract the round-trip assertions in
   `tests/calibration_db.rs` into a helper that takes a `Db`. Run it twice:
   - SQLite over `tempdir()` (current behaviour).
   - Postgres when `SKILLNET_TEST_PG_URL` is set and the binary is built
     with `--features postgres`; otherwise skip with a clear log message.
   Use a per-test schema (`CREATE SCHEMA test_<uuid>; SET search_path …`)
   or a per-test database, and drop it on completion.
2. **Cargo aliases.** Add to `.cargo/config.toml` (creating it if absent):
   ```toml
   [alias]
   test-pg = ["test", "--features", "postgres", "--all-targets"]
   ```
   Document this in `README.md` under a "Development" or "Testing"
   subsection.
3. **README.** Add a "Storage backends" section with:
   - SQLite default and config snippet.
   - Postgres setup: feature flag, env var, `[database]` block, HM module
     example.
   - The secret-in-Nix-store caveat from phase 04.
4. **MIGRATION.md.** Append a short note: "0.2.x adds optional Postgres
   support behind the `postgres` feature; SQLite remains the default and
   no migration is required."
5. **CHANGELOG.md.** New section for the upcoming release listing:
   backend abstraction, `postgres` feature, `[database]` config block,
   `--database-url` flag, `SKILLNET_DATABASE_URL`, HM module
   `database.{backend,path,url}` options.
6. **Final gates.**
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo clippy --all-targets --features postgres -- -D warnings`
   - `cargo test --all-targets`
   - `cargo test-pg` (when a local Postgres URL is available)
   - `nix flake check`

## Acceptance criteria

- [ ] Round-trip assertions in `tests/calibration_db.rs` are
      parameterised; the Postgres run executes when
      `SKILLNET_TEST_PG_URL` is set and is skipped (not failed) otherwise.
- [ ] README has a "Storage backends" section covering SQLite and
      Postgres with config + env + HM examples.
- [ ] CHANGELOG.md has an entry summarising the new surface.
- [ ] MIGRATION.md notes the addition.
- [ ] All listed gates pass locally (modulo `cargo test-pg` when no PG is
      available; the SQLite path must still pass).

## Files likely touched

- `tests/calibration_db.rs`
- `.cargo/config.toml` (new or existing)
- `README.md`
- `MIGRATION.md`
- `CHANGELOG.md`

## Pitfalls

- **Test isolation.** Two test runs against the same Postgres database
  must not race. Use distinct schemas or distinct databases per run, and
  always drop on teardown — including on panic (`Drop` impl, not just
  end-of-test code).
- **Skip vs fail.** When `SKILLNET_TEST_PG_URL` is unset and the feature
  is off, the test must produce a `Skipped` (or equivalent log) — never a
  silent pass that hides a real regression. Print a one-line reason.
- **`nix flake check` cost.** If it became expensive after phase 04, run
  the targeted attribute (e.g. `nix build .#checks.<system>.hm-default`)
  rather than the full set.

## Reference

- Earlier phase docs in this directory.
- Current test layout: `tests/calibration_db.rs`.
