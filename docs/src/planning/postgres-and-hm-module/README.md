# Plan — Postgres support + HM module update

Adds an optional Postgres backend to the `skillnet` calibration store (currently
SQLite-only via `rusqlite`) and surfaces the choice through the Home Manager
module so users can point calibration at either a local SQLite file or a remote
Postgres URL.

## Phases

| # | Slug | Model | Depends on |
|---|---|---|---|
| 01 | [db-backend-abstraction](01-db-backend-abstraction.md) | GPT 5.5 high | — |
| 02 | [postgres-backend](02-postgres-backend.md) | GPT 5.5 high | 01 |
| 03 | [config-and-cli-wiring](03-config-and-cli-wiring.md) | GPT 5.5 medium | 01 |
| 04 | [hm-module-postgres](04-hm-module-postgres.md) | GPT 5.5 medium | 03 |
| 05 | [tests-docs-changelog](05-tests-docs-changelog.md) | GPT 5.5 medium | 02, 03, 04 |

## Parallelism

- 02 and 03 can run concurrently after 01 lands.
- 04 must follow 03.
- 05 closes out, touching docs/tests across all backends.

## Out of scope (whole plan)

- Async runtime adoption — keep the calibration code synchronous.
- Schema redesign — preserve current tables/columns; only adapt SQL to dialect.
- Multi-tenant or shared deployment guidance beyond a single-user Postgres URL.
- Migrating existing SQLite data into Postgres (separate effort if requested).

## Verify

When all phases have been executed, prompt `verify` to audit each phase's
acceptance criteria against the repo state.
