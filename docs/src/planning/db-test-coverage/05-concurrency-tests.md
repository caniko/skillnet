# Phase 05 — Concurrency tests (D-series, opt-in)

> **Recommended Codex model: GPT 5.5 low**
>
> Two SQLite-only smoke tests, both `#[ignore]`d by default because they
> are timing-sensitive and would slow the default suite. The shape is
> standard: spawn threads, share a path, observe lock behaviour.
> Mechanical — `low` is the right tier.

## Working tree

`/data/nvme0/can/Projects/skillnet` on `main`, with phase 01 landed.

## Goal

Two `#[ignore]`-by-default `#[test]` functions in
`tests/calibration_db_concurrency.rs` that document SQLite's WAL
contract under multiple readers/writers. They run only when invoked
explicitly (`cargo test --all-targets -- --ignored`) so flakes can't
break CI.

## Why

WAL semantics are easy to break in a future refactor (switching to
DELETE journal, removing the `WAL` pragma, holding a lock too long).
These tests serve as living documentation; even when ignored, their
presence flags the contract.

## Out of scope

- Postgres concurrency — the abstraction holds a single `Client`, so
  there is no in-process concurrency story to test today.
- Connection pooling.
- Stress tests / soak tests.

## Plan

Add `tests/calibration_db_concurrency.rs` with the standard `#[path]`
include of `db.rs`. Both tests share a `tempdir()` and a path.

1. **D1 — `sqlite_wal_allows_concurrent_readers`.** `#[ignore]`.
   - Open one writer `Db`, begin a `transaction`, insert a `tags` row,
     but **do not commit**.
   - From a second thread (spawned with `std::thread::spawn`), open a
     reader `Db` at the same path and `query_one("SELECT COUNT(*) FROM
     tags …", …)`.
   - Assert the reader observes the *pre-transaction* state (count
     `0` or whatever the seed was) without blocking. Bound the reader
     thread with a `std::time::Duration::from_secs(2)` join timeout —
     if it blocks, the test fails with a clear "WAL reader was blocked
     by uncommitted writer" message.
   - Commit, then re-query; reader sees the new row.

2. **D2 — `sqlite_writers_serialise_without_panic`.** `#[ignore]`.
   - Spawn two writer threads, each opening its own `Db` at the same
     path and inserting 50 `tags` rows in a loop (one row per
     `execute`, no shared transaction).
   - Join both. Assert no panic, and that the final row count is
     exactly 100.
   - This test currently *may* fail with `SQLITE_BUSY` if the bridge
     has no retry/backoff. That's the point — the test documents the
     gap. If it fails, the chat reply proposes either:
     - Adding a `busy_timeout` pragma to `Db::open`.
     - Wrapping `execute` in a bounded retry loop.
     Either is a separate, out-of-scope follow-up.

## Acceptance criteria

- [ ] `tests/calibration_db_concurrency.rs` exists with D1 and D2,
      both annotated `#[ignore]`.
- [ ] `cargo test --all-targets` (no `--ignored`) passes and does not
      execute D1 or D2.
- [ ] `cargo test --all-targets -- --ignored` either passes both or
      fails D2 with the documented `SQLITE_BUSY` symptom (which is
      reported in the chat reply with the proposed follow-up, not
      silently relaxed).
- [ ] D1 has a join timeout so a regression to blocking-reader
      semantics fails fast.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.

## Files likely touched

- `tests/calibration_db_concurrency.rs` (new)

## Pitfalls

- **Shared tempdir lifetime.** The `tempfile::TempDir` must outlive
  both threads. Bind it in the parent scope and pass the `PathBuf` by
  value (clone) into each `spawn`'d closure.
- **Bounded join.** `JoinHandle::join` blocks indefinitely; wrap
  reader-side work in a `channel + recv_timeout` pattern so a
  regression in WAL semantics fails the test in 2s, not hangs the
  suite.
- **D2 ordering.** Each writer thread uses its own `Db` (its own
  `Connection`). They share the file. `SQLITE_BUSY` arises when both
  try to grab the write lock; whether it panics depends on the bridge.
  The test must catch the panic with `catch_unwind` *only if* the
  bridge is panic-not-error-on-busy; otherwise assert on `Result::Err`.
- **`#[ignore]` is the right floor.** Don't promote these to default
  unless the bridge gains a busy-timeout policy and you're confident
  in the test's timing margins.

## Reference

- `Db::open` (SQLite WAL pragma) at `src/calibration/db.rs:88-108`.
- SQLite WAL docs: <https://www.sqlite.org/wal.html>.
- `tempfile::TempDir` lifecycle (drops on scope exit).
