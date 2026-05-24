# Phase 02 — Remaining parity tests (A2–A12 minus the hotspots)

> **Recommended Codex model: GPT 5.5 medium**
>
> Nine independent tests, all built on the harness from phase 01. Each
> one is small but the JSONB normalisation (A10) and `unix_date_expr`
> (A11) tests need careful contract decisions. `low` would likely punt
> on A10's "byte-stable vs semantic equality" choice and produce a
> Postgres-flaky test; `high` is overkill — the design surface is small
> once phase 01's harness exists.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`, with phase 01 landed.

## Goal

All remaining A-series parity tests (A2, A3, A5, A6, A7, A9, A10, A11,
A12) implemented as harness-driven `#[test]` functions, each iterating
`backends()` and asserting identical behaviour with the backend name in
panic messages.

## Why

Each test pins one contract on the `Db` API that the abstraction
implicitly promises but does not currently verify. Adding them in one
phase keeps the contract decisions consistent (e.g. one JSONB stance
applied across A6/A7/A10).

## Out of scope

- Backend-specific tests (phase 03 covers B1–B5).
- Error paths (phase 04 covers C1–C6).
- Production-code changes. If a test reveals a contract gap, document it
  in this phase's chat reply; do not patch `Db` here.

## Plan

For each test below, write a `#[test]` function in
`tests/calibration_db.rs` that loops `for (name, mut fx) in backends()`
and asserts behaviour. Reuse `assert_round_trip`'s fixture inserts where
helpful; otherwise insert minimal `tags` or fresh `plans` rows.

1. **A2 — `query_optional_returns_none_for_missing_and_some_for_present`.**
   Query `plans` for a known-absent uuid → `None`. Insert a row, query
   again → `Some(_)` carrying the expected name.

2. **A3 — `query_all_returns_rows_in_order_by_clause`.** Insert N=4
   phases with ordinals `[3, 1, 4, 2]`; `SELECT … ORDER BY ordinal DESC`
   and verify the closure is invoked exactly 4 times in the order `[4,
   3, 2, 1]`.

3. **A5 — `nullable_params_round_trip_through_optional_accessors`.**
   Two sub-cases per backend:
   - Insert a `plans` row with `worktype = NULL` (via
     `DbParam::nullable_text(None)`), read back through
     `DbRow::get_optional_string`.
   - Insert a `verifications` row with `elapsed_seconds = NULL` (via
     `nullable_i64(None)`), read back through `get_optional_i64`.
   Assert both yield `None` and that the non-null counterparts (`Some`)
   round-trip unchanged.

4. **A6 — `placeholder_reuse_binds_same_value_twice`.** Run
   `SELECT $1, $1` (or, more realistically, `SELECT * FROM tags WHERE
   key = $1 OR value = $1`) and confirm both backends bind the single
   parameter into both positions without ordinal confusion.

5. **A7 — `placeholder_in_string_literal_is_not_rewritten`.** Insert a
   `tags` row with `value = '$1 is a literal'`; `SELECT value FROM tags
   WHERE key = $1` and assert the returned string is exactly `$1 is a
   literal`. Catches an over-eager dialect rewriter in the SQLite
   bridge.

6. **A9 — `execute_returns_rows_affected_count`.** Insert three `tags`
   rows; `UPDATE tags SET value = 'x' WHERE key = $1` returns 3; `DELETE
   FROM tags WHERE key = $1` with a non-matching key returns 0. Assert
   the `usize` return value matches.

7. **A10 — `json_blob_round_trips_semantic_equality`.** Insert a
   `plans` row whose `routing_dist` is `{"medium":2,"high":1}` (note key
   ordering and absence of spaces). Read it back as a String, parse via
   `serde_json::from_str::<serde_json::Value>`, and assert
   `serde_json::Value` equality with the input. *Do not* assert byte
   equality — Postgres `JSONB` normalises whitespace and key order; the
   contract is semantic equality. Add a one-line comment in the test
   explaining this and pointing to phase 02's design note.

8. **A11 — `unix_date_expr_groups_timestamps_into_calendar_days`.**
   Insert three `plans` rows with `created_at` values that cross a
   UTC midnight: e.g. `1_717_171_700` and `1_717_171_705` (same day)
   and `1_717_258_100` (next day). Run
   ```sql
   SELECT {unix_date_expr("created_at")} AS day, COUNT(*)
   FROM plans
   GROUP BY 1
   ORDER BY 1
   ```
   (substitute via `db.unix_date_expr(...)`). Assert exactly two
   groups with counts `[2, 1]`. Catches divergent date-truncation SQL
   between backends.

9. **A12 — `migration_is_applied_exactly_once_across_reopens`.** Open
   the DB, record `applied_migration_count`. Drop, re-open, re-open
   again. Each re-open must keep the count identical. Already implicit
   in the original round-trip; promote to its own test so a regression
   doesn't take down the larger one.

## Acceptance criteria

- [ ] Nine new `#[test]` functions exist with the names listed above
      (or close paraphrases).
- [ ] Each iterates `backends()` and includes the backend name in any
      panic message.
- [ ] A10 documents the semantic-equality stance inline.
- [ ] A11 passes on both backends; if it fails on Postgres because
      `unix_date_expr` produces a different SQL fragment than expected,
      surface the divergence in the chat reply rather than relaxing the
      assertion.
- [ ] `cargo test --all-targets` and `cargo test --all-targets
      --features postgres` (with and without `SKILLNET_TEST_PG_URL`)
      pass.
- [ ] `cargo clippy` clean on both feature configurations.

## Files likely touched

- `tests/calibration_db.rs`

## Pitfalls

- **A10 against `JSONB`.** Don't compare strings. Parse both sides into
  `serde_json::Value` and use that as the equality predicate.
- **A11 timezone.** `unix_date_expr` likely truncates to UTC days; if
  the chosen timestamps land near a TZ boundary in the test runner's
  local time, the grouping flips. Use UTC-anchored values.
- **A7 dialect rewriter.** If the SQLite bridge naively replaces every
  `$N` with `?N`, A7 will fail. That's the test's job — the bridge
  needs to be string-literal-aware. If currently broken, file the gap
  and (separately) fix it in `db.rs`; this plan doesn't authorise
  production patches.
- **A3 closure-invocation count.** Use a `Cell<usize>` or an
  accumulator `Vec` inside the closure to count invocations; don't
  rely solely on the returned `Vec` length.
- **A9 affected-rows on `UPDATE` no-ops.** `UPDATE … SET x = x` may
  count differently on the two backends; pick an `UPDATE` that always
  changes the value.

## Reference

- Phase 01 harness.
- `src/calibration/db.rs:146-247` for the methods under test.
