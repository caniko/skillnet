# Phase 04 — Error-path tests (C-series)

> **Recommended Codex model: GPT 5.5 low**
>
> Six tests, each asserts that a specific failure mode produces an
> actionable `anyhow::Error` rather than a panic or silent default.
> Mechanical work once each contract is decided (`query_one` exactly-one
> vs LIMIT 1; type-mismatch behaviour of `DbRow::get_*`). `medium` would
> overspend; `low` matches the volume of code per test.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`, with phase 01 landed.

## Goal

Six `#[test]` functions in a new `tests/calibration_db_errors.rs` that
pin the error contracts of `Db::query_one`, `DbRow::get_*`, transactions,
and the two `open_*` constructors.

## Why

Error paths are where abstractions leak. The parity suite confirms the
happy path is identical across backends; the error suite confirms the
*unhappy* path produces messages that point at the user's mistake, not
at internals.

## Out of scope

- Patching `Db` to fix error messages this phase exposes as poor —
  surface them in the chat reply and propose a follow-up.
- Tests that require process-level fault injection.

## Plan

Add `tests/calibration_db_errors.rs`, `#[path]`-including `db.rs`.

1. **C1 — `query_one_errors_when_no_rows_match`.** Parity (loop
   `backends()`). Insert nothing matching, call `query_one` with a
   predicate that returns zero rows, assert `Err`. The error's
   `Display` should mention "no rows" or the SQL fragment / table; do
   not pin exact wording. Anchor on `is_err()` plus a
   non-empty-message check.

2. **C2 — `query_one_errors_when_multiple_rows_match`.** Parity. Insert
   two rows whose predicate matches the same `WHERE`, call `query_one`,
   assert `Err`. **Contract decision**: `query_one` is "exactly one",
   not "first of many". If the current implementation behaves as
   `LIMIT 1`, surface that in the chat reply with a recommendation
   (either change the impl or rename to `query_first`).

3. **C3 — `db_row_accessor_type_mismatch_returns_anyhow_error`.**
   Parity. Insert a `tags` row, then `query_one("SELECT key FROM tags
   WHERE …", |row| row.get_i64(0))`. Assert `Err`, not panic. Use
   `std::panic::catch_unwind` only as a defensive layer; the assertion
   is on the `Result`.

4. **C4 — `transaction_rolls_back_on_panic_inside_closure`.**
   Postgres-focused (Postgres's `transaction()` method rolls back on
   `Drop` if not committed). Run a transaction whose closure panics;
   wrap the call in `std::panic::catch_unwind(AssertUnwindSafe(...))`.
   Outside the `catch_unwind`, query the table the closure tried to
   insert into — assert the row is absent. Run on both backends if the
   bridge supports panic-safe rollback; otherwise gate to Postgres and
   note the SQLite gap in the chat reply.

5. **C5 — `open_postgres_with_invalid_url_returns_actionable_error`.**
   Postgres-only. `Db::open_postgres("not://a/url")` → `Err` whose
   `Display` contains the substring `"not://a/url"` or names
   `"postgres"`/`"connect"`. Asserts the error path doesn't panic and
   surfaces enough context for the user.

6. **C6 — `open_sqlite_in_unwritable_dir_returns_actionable_error`.**
   SQLite-only. Create a tempdir, place a regular *file* at
   `<temp>/notadir`, then call `Db::open(&temp.join("notadir").join("x.sqlite"))`.
   The parent-creation step will fail with `NotADirectory` (or
   equivalent). Assert `Err` and that the message names the path
   `notadir`.

## Acceptance criteria

- [ ] `tests/calibration_db_errors.rs` exists and includes `db.rs` via
      `#[path]`.
- [ ] Six tests with the names above (or close paraphrases). C1, C2,
      C3 iterate `backends()`; C5 and C6 are backend-gated; C4 is on
      whichever backends honour the panic-safe rollback contract.
- [ ] Every test asserts on `.is_err()` *and* on a substring of the
      error message that would help a user diagnose.
- [ ] No test uses `unwrap()` on the operation under test.
- [ ] `cargo test --all-targets` and `cargo test --all-targets
      --features postgres` pass; if a contract gap is exposed (e.g.
      C2 finds `query_one` is LIMIT 1), it's documented in the
      chat reply.
- [ ] `cargo clippy` clean on both feature configurations.

## Files likely touched

- `tests/calibration_db_errors.rs` (new)

## Pitfalls

- **Substring assertions on error messages.** Error wording changes
  between driver versions and Rust toolchains. Pick stable anchors
  (table names, user-supplied URL fragments) rather than driver phrases.
- **C2 contract decision.** If the impl is LIMIT 1 today and the user
  wants to keep it that way, change the test to assert "first row" and
  rename the helper. Don't let the test silently encode a contract the
  code doesn't keep.
- **C4 panic safety.** `Db::transaction` takes `FnOnce(&mut Tx) -> Result`.
  A panicking closure aborts the unwind through the helper; the
  underlying driver transaction must drop and roll back. Postgres's
  `Transaction` does this naturally; the SQLite bridge may not if it
  holds the transaction in a way that doesn't drop on panic. Verify
  before adding the SQLite case to C4.
- **C6 platform variance.** `mkdir over a file` errors look slightly
  different on Linux vs macOS; the test only asserts on the path
  substring, not the OS error code.

## Reference

- `Db::query_one`/`transaction` at `src/calibration/db.rs:175-235`.
- `Db::open` and `Db::open_postgres` constructors.
- `anyhow::Error::to_string()` for message inspection.
