# Phase 05 — Tests rewrite + MIGRATION.md

> **Recommended Codex model: GPT 5.5 medium**
>
> Leaf node role, moderate complexity. The work is high-volume but
> low-design: rewrite 599 lines of integration tests against the
> new surface, write a verb-by-verb migration mapping for users.
> Neither involves novel decisions if Phases 01–04 landed cleanly —
> the test rewrite mirrors the new command tree, and the migration
> doc is a translation table. `low` would risk dropping coverage by
> not auditing what the old tests proved; `high` is wasteful for
> what is fundamentally a translation exercise. `medium` ensures
> careful coverage parity without over-thinking.

## Working tree

`/data/nvme0/can/Projects/ai-skills`.

Phases 03 and 04 must both have landed. `cargo build` is clean.
`tests/cli.rs` has been red since Phase 02 (its old verbs no longer
exist); this phase makes it green again.

## Goal

Rewrite `tests/cli.rs` against the new command surface, restore
`cargo test` green and `cargo clippy --all-targets -- -D warnings`
green, and ship `MIGRATION.md` at the repo root documenting every
old → new verb and flag.

## Why this matters now

The rebuild has been merged-in-parts without test coverage since
Phase 02. Landing 05 closes that gap and is the last gate before
the rebuild can be shipped. `MIGRATION.md` is the user-facing
deliverable — without it, users have to read commit messages to
figure out what changed.

## Out of scope

- Do **not** add new test scenarios beyond what the old test file
  covered, *unless* they're necessary to validate behavior new to
  the rebuild (cache invalidation, `--then-push`, the merged `skill
  show` output, the `status` front door). Coverage parity first;
  net-new coverage only where the rebuild introduced net-new
  behavior.
- Do **not** rewrite as `#[test]` unit tests inside `src/`. Keep
  them as integration tests in `tests/cli.rs` — they exercise the
  built binary, which is the right scope.
- Do **not** add a tutorial / "getting started" doc. `MIGRATION.md`
  is the only doc this phase ships; future work can add a richer
  guide.
- Do **not** publish to crates.io or tag a release. The user owns
  release timing.

## Plan

1. **Audit the old test file.** Read `tests/cli.rs` end to end.
   Build a checklist of behaviors it asserts, grouped by verb:
   - `mirror reconcile` / `reconcile` — what's asserted? (count
     of skills, side effects on `.agents/.claude` dirs, etc.)
   - `mirror sync` / `sync` — what's asserted?
   - `skill delete/rename/move/globalize/deglobalize` — what's
     asserted?
   - `mirror list`, `mirror targets`, `mirror sources` — what's
     asserted?
   - `toml project add/remove/list` — what's asserted?
   - `catalog generate/lint/show/search` — what's asserted?

   This checklist is the coverage parity target.

2. **Map each old test to its new-tree equivalent.** For each:
   - Replace verb (e.g., `reconcile --target global` → `sync pull
     --scope global`).
   - Replace per-command `--dry-run` with the global `--dry-run`.
   - Replace string `--target` with typed `--scope` (clap will
     reject typos at parse time; tests should rely on this).
   - For `catalog show <skill>` tests, retarget to `skill show
     <scope>/<skill>` and update assertions to cover the merged
     output (file metadata section + catalog section).
   - For `globalize`/`deglobalize` tests, retarget to `skill move`
     with the right scope arguments.

3. **Add net-new test coverage for rebuild behavior**:
   - `skillnet` (no args) runs status (exit code 0; output contains
     "scope" lines).
   - `sync pull --then-push --scope global` runs pull then push;
     deliberately break pull (e.g., point `--config` at a bad TOML)
     and assert push doesn't run (no live-side mutation).
   - Cache file exists after `sync pull`; deleting it doesn't break
     subsequent `status`; corrupting it (`echo garbage`) doesn't
     break `status`.
   - `sync status` on a clean scope reports clean; touching a live
     source then `sync status` reports diverged.
   - `skill show <scope>/<missing>` exits non-zero with a "not
     found" message.
   - `skill show <scope>/<skill>` output contains both a `path:`
     line and a `catalog entry:` section.

4. **Use a temp-dir fixture pattern.** The old tests likely already
   build a temp mirror root + config; reuse the pattern. Each test
   should be hermetic (no shared state with other tests, no reliance
   on the repo's real `global/` or `projects/` directories).

5. **Write `MIGRATION.md` at the repo root.** Required sections:

   ```markdown
   # Migration: skillnet CLI rebuild

   ## TL;DR

   The CLI tree was reorganized around what you're acting on:
   `sync`, `skill`, `scope`, `project`, `catalog`. Every old verb
   maps to one new verb. No hidden aliases — old invocations error.

   ## Removed top-level shortcuts

   | Old                               | New                                |
   |-----------------------------------|------------------------------------|
   | `skillnet reconcile`              | `skillnet sync pull`               |
   | `skillnet reconcile --sync`       | `skillnet sync pull --then-push`   |
   | `skillnet sync`                   | `skillnet sync push`               |
   | `skillnet delete <s> <k>`         | `skillnet skill delete <s>/<k>`    |
   | `skillnet rename <s> <o> <n>`     | `skillnet skill rename <s>/<o> <n>`|
   | `skillnet move <fs> <k> <ts>`     | `skillnet skill move <fs>/<k> <ts>`|
   | `skillnet globalize <p> <k>`      | `skillnet skill move <p>/<k> global`|
   | `skillnet deglobalize <k> <p>`    | `skillnet skill move global/<k> <p>`|
   | `skillnet list`                   | `skillnet skill list`              |
   | `skillnet targets`                | `skillnet scope list`              |
   | `skillnet sources --target X`     | `skillnet scope sources --scope X` |
   | `skillnet project ...`            | unchanged                          |

   ## Removed namespaces

   - `skillnet mirror <verb>` — every verb moved under `sync`,
     `skill`, or `scope` as above.
   - `skillnet toml project <verb>` — moved to top-level
     `skillnet project <verb>`.
   - `skillnet catalog show <skill>` — folded into
     `skillnet skill show <scope>/<skill>`.

   ## Flag changes

   - `--sync` on edit verbs: **removed**. Run `skillnet sync push
     --scope <scope>` afterward.
   - `--target <all|global|project|<name>>`: **removed**. Use
     `--scope` (repeatable) and `--all`.
   - Per-command `--dry-run`: **removed**. Use the global
     `--dry-run` flag: `skillnet --dry-run sync push`.

   ## New commands

   - `skillnet` (no args): runs `status`.
   - `skillnet status`: scopes + divergence + catalog health.
   - `skillnet sync status`: read-only divergence per scope.
   - `skillnet sync diff`: file-level diff mirror↔live.
   - `skillnet sync pull --then-push`: composed pull then push.

   ## Caching

   `mirror_root/.skillnet/cache.toml` stores per-scope pull
   timestamps and content hashes. `status` and `sync status` use
   it to skip redundant walks. The cache is best-effort; deleting
   or corrupting it falls back to a full walk on the next command.

   ## Common workflows

   Edit and push:
       skillnet skill move global/foo myproj
       skillnet sync push --scope global --scope myproj

   Refresh from live and immediately re-mirror:
       skillnet sync pull --then-push

   Check what would change without writing:
       skillnet sync status         # cached, fast
       skillnet sync diff           # file-level detail
       skillnet --dry-run sync push # plan-only
   ```

6. **Run the full validation gate**:
   - `cargo build`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test`
   - Manual: `skillnet --help` matches the README tree exactly.
   - Manual: every workflow in `MIGRATION.md § Common workflows`
     executes on a real mirror.

## Acceptance criteria

- [ ] `cargo build` clean, no warnings.
- [ ] `cargo clippy --all-targets -- -D warnings` clean (note:
  this is the first phase where `--all-targets` must be green).
- [ ] `cargo test` green; at least as many tests as before, plus
  the net-new rebuild coverage in step 3.
- [ ] `tests/cli.rs` contains zero references to removed verbs
  (`reconcile`, `globalize`, `deglobalize`, `--sync` flag, `--target`
  flag, `mirror` subcommand, `toml` subcommand, `catalog show`).
  Verify: `rg 'reconcile|globalize|--sync|--target' tests/` returns
  zero matches.
- [ ] `MIGRATION.md` exists at repo root, contains the mapping table,
  the namespace-removal list, the flag changes, the new-commands
  list, the caching section, and the common-workflows section.
- [ ] Every workflow in `MIGRATION.md § Common workflows` runs on a
  real mirror without error.
- [ ] Each old verb in the mapping table errors at clap level with a
  message that includes "unrecognized subcommand" or "unexpected
  argument" — verify by running each.
- [ ] The whole-set acceptance criteria from
  [README.md](./README.md#whole-set-acceptance-criteria) all pass.

## Files likely touched

- `tests/cli.rs` — **wholesale rewrite**. Expect ~500–700 lines.
- `MIGRATION.md` — **new** at repo root. ~120 lines.
- No `src/` changes expected. If a test reveals a bug in Phases
  01–04's implementations, fix it in the offending phase's files
  here — don't add a Phase 06.

## Pitfalls

- **Dropping coverage by translating only the happy paths.**
  Symptom: tests still pass, but error-path coverage shrinks because
  the new verbs have different error shapes. Recovery: for every old
  test that asserted on error output, find or write a new-tree
  equivalent that asserts a corresponding error. The clap-level
  "unknown scope" errors are *better* than the old runtime errors —
  cover them.

- **Cache test flakiness.** Symptom: a test asserts "cache stamp
  was updated" but the mtime granularity on the test FS isn't fine
  enough; two pulls in quick succession produce the same stamp.
  Recovery: pin the cache assertion to "stamp exists and is recent"
  rather than "stamp differs from prior". Or use `sleep_ms(10)`
  between pulls if you really need ordering — sparingly.

- **Temp-dir fixtures leaking state.** Symptom: tests pass in
  isolation, fail in parallel. Cause: shared `$HOME` or `$XDG_*`
  env vars affecting tools the binary calls. Recovery: set
  `--config` and `--mirror-root` explicitly on every invocation;
  use `assert_cmd::Command::cargo_bin("skillnet")` (or whatever the
  old tests use) and pass globals through.

- **`MIGRATION.md` going stale.** Symptom: the mapping table
  references a flag that ended up named differently in Phase 02.
  Recovery: before declaring the phase done, eyeball every entry in
  the mapping table against `skillnet --help` and the subcommand
  helps. If anything diverges, the help text wins; update
  `MIGRATION.md`.

- **Forgetting `cargo clippy --all-targets` until the end.**
  Symptom: clippy warnings in the test file go unnoticed because
  Phases 02–04 only gated on `--lib --bins`. Recovery: run
  `--all-targets` as the *first* step of this phase, after `cargo
  build`, before writing new tests. Fix any warnings inherited from
  the old test file (they'll be on code you're rewriting anyway).

- **The `assert_cmd` crate version skew.** Symptom: test framework
  helpers have moved between versions; old usage compiles fail.
  Cause: dependency bump somewhere during Phases 01–04. Recovery:
  check `Cargo.toml` for `assert_cmd`/`predicates` versions; if
  unchanged from before, the old test patterns still apply.

## Reference

- Old test file: [`tests/cli.rs`](../../../tests/cli.rs).
- All previous phases:
  - [01-foundation.md](./01-foundation.md)
  - [02-cli-surface.md](./02-cli-surface.md)
  - [03-sync-and-status.md](./03-sync-and-status.md)
  - [04-skill-and-catalog.md](./04-skill-and-catalog.md)
- Plan README: [README.md](./README.md).
