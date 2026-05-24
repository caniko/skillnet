# Plan — DB abstraction test coverage

Expand `tests/calibration_db.rs` (and siblings) from the current single
round-trip per backend to a table-driven harness with explicit coverage for
each public method on `Db`, `Tx`, `DbRow`, and `DbParam`, plus
backend-specific behaviours that only exist on one side (WAL pragma,
JSONB validation, BIGSERIAL `RETURNING`, schema isolation).

## Phases

| # | Slug | Model | Depends on |
|---|---|---|---|
| 01 | [harness-and-dialect-hotspots](01-harness-and-dialect-hotspots.md) | GPT 5.5 medium | — |
| 02 | [remaining-parity-tests](02-remaining-parity-tests.md) | GPT 5.5 medium | 01 |
| 03 | [backend-specific-tests](03-backend-specific-tests.md) | GPT 5.5 medium | 01 |
| 04 | [error-path-tests](04-error-path-tests.md) | GPT 5.5 low | 01 |
| 05 | [concurrency-tests](05-concurrency-tests.md) | GPT 5.5 low | 01 |

## Parallelism

- 01 unblocks everything.
- 02, 03, 04, 05 run concurrently after 01 lands. Each writes a different
  test file per the suggested split, so merge conflicts are limited to the
  shared harness helpers extracted in 01.

## Out of scope (whole plan)

- Refactoring the production `Db` API — tests adapt to it, not the
  reverse. Any contract gap surfaced (e.g. `query_one` "exactly one" vs
  `LIMIT 1`) is decided in-phase and documented, not redesigned.
- Adding a connection pool or async API.
- Cross-version migration rollback (no such story exists today).

## Verify

When all phases have been executed, prompt `verify` to audit each phase's
acceptance criteria against the repo state.
