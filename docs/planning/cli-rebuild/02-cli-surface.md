# Phase 02 — CLI surface rewrite

> **Recommended Codex model: GPT 5.5 high**
>
> Orchestrator role, complex task. This is a wholesale rewrite of
> `src/cli/args.rs` (389 lines) and `src/cli/mod.rs` (220 lines) into
> the new command tree. The work is mechanically large but design-
> sensitive: every choice about subcommand attributes, global args,
> dispatch shape, and how the runtime-built `Scope` value parser
> attaches has downstream consequences. `medium` would skim past the
> attachment-point design question; `max` is overkill — the design is
> already locked. `high` is the right call.

## Working tree

`/data/nvme0/can/Projects/ai-skills`.

Phase 01 must have landed first: `src/cli/scope.rs` and `src/cache.rs`
exist and compile.

## Goal

Replace the entire command tree in `src/cli/args.rs` and the dispatch
table in `src/cli/mod.rs` with the new shape from
[README.md](./README.md). The binary builds, `skillnet --help`
prints the new tree, every old top-level verb is gone at compile time,
and the handlers either call into existing functions (thin wrappers)
or are deliberately stubbed for Phase 03/04 to fill in.

## Why this matters now

The current surface is the visible blocker: until users see the new
tree, they can't migrate, and Phases 03/04 can't be implemented
against the new shape because their argument signatures depend on it.
Phase 02 unblocks the parallel fanout in Wave 2.

This phase is also where the "no hidden aliases" doctrine becomes
enforceable. Once the old verbs are _deleted from `args.rs`_, they
cannot creep back as `#[command(hide = true)]` shortcuts. The
codebase becomes the source of truth.

## Out of scope

- Do **not** implement `sync status`, `sync diff`, or the top-level
  `status` command's real logic. Stub them as
  `bail!("not yet implemented — see Phase 03")` so the help text
  shows them but invocation cleanly errors. (Phase 03 implements them.)
- Do **not** implement `skill show`'s catalog-fold logic. Stub it
  to call the existing `catalog::show` for now — Phase 04 expands it
  to merge file metadata + catalog entry. (Catalog show is still
  reachable via `skill show` after this phase, just thinly.)
- Do **not** remove `catalog show` from `src/catalog/mod.rs` yet.
  Phase 04 deletes it after `skill show` absorbs it.
- Do **not** rewrite tests. Phase 05 owns `tests/cli.rs`. Existing
  tests will fail at this phase — that's expected; the test crate is
  not part of `cargo build`. **However**, ensure `cargo build --tests`
  is _not_ part of the acceptance criteria for this phase — only
  `cargo build` is.
- Do **not** add `MIGRATION.md`. Phase 05.

## Plan

1. **Inventory the new command tree** from
   [README.md § Target command tree](./README.md#target-command-tree).
   The tree has 5 top-level groups (`status`, `completions`, `sync`,
   `skill`, `scope`, `project`, `catalog`) plus the no-arg alias and
   the global flags.

2. **Rewrite `src/cli/args.rs`** from scratch:
   - Top-level `Cli` struct: globals `--config`, `--mirror-root`,
     `--catalog-config`, `--dry-run` (all `global = true`), plus
     `command: Option<Command>` (Option so `skillnet` with no args
     resolves to `status`).
   - `enum Command`: `Status`, `Completions`, `Sync { command: SyncCommand }`,
     `Skill { command: SkillCommand }`, `Scope { command: ScopeCommand }`,
     `Project { command: ProjectCommand }`, `Catalog { command: CatalogCommand }`.
   - `enum SyncCommand`: `Pull { scope: Vec<String>, all: bool, then_push: bool }`,
     `Push { scope: Vec<String>, all: bool }`,
     `Status { scope: Vec<String> }`,
     `Diff { scope: Vec<String> }`.
   - `enum SkillCommand`: `List { scope: Vec<String>, all: bool }`,
     `Show { path: String }`, `Delete { path: String }`,
     `Rename { path: String, new: String }`,
     `Move { from: String, to: String }`.
     Use a single positional `String` for the `<scope>/<skill>` path;
     parse with `SkillPath::parse` _after_ config loads (inside
     `mod.rs::run`), not via clap's value parser, because parsing
     needs the configured-projects list. For `Move`, `to` is `<scope>`
     or `<scope>/<name>` — same parser handles both.
   - `enum ScopeCommand`: `List`, `Sources { scope: Option<String> }`.
   - `enum ProjectCommand`: unchanged from today's shape — `List`,
     `Add { name, path, allow_missing }`, `Remove { name, prune_mirror }`.
     (Drop the per-command `--dry-run`; use the global.)
   - `enum CatalogCommand`: `Generate`, `Lint`, `Search { query }`.
     **No `Show`** — it's folded into `SkillCommand::Show` in Phase 04;
     Phase 02 already excludes it from the args enum.

   Annotate `--scope` arguments with `value_name = "SCOPE"`, repeatable
   via `action = clap::ArgAction::Append`. Do **not** attach the
   `Scope` value parser as a `#[arg]` attribute — see Pitfalls.

3. **Rewrite `src/cli/mod.rs`** to dispatch the new tree:
   - Build the `Cli` via `Cli::parse()`.
   - Handle `Completions` before config load (today's pattern; keep it).
   - Load `Context`.
   - If `command` is `None`, dispatch to `Status` (the no-args alias).
   - For every command, parse string args into typed forms _here_
     using `SkillPath::parse(&ctx)` and a `Scope` resolver that turns
     `Vec<String>` + `all: bool` into `Vec<Scope>`. **Validation
     happens at the dispatch boundary, before handlers run.** Pass
     typed values to handlers.
   - Handler functions: call into existing `commands::*` for now where
     possible (e.g., `Sync::Pull` → today's `commands::reconcile`,
     `Sync::Push` → today's `commands::sync`, `Skill::Move` → today's
     `commands::move_skill`, etc.). For the merged `from`/`to` move
     args, parse both into `(Scope, Option<String>)` and adapt to the
     old multi-arg call shape.
   - For not-yet-implemented commands (`Status`, `Sync::Status`,
     `Sync::Diff`), `bail!` with a marker message ("Phase 03 wires
     this — implementation pending"). Do not panic.

4. **Delete the old top-level duplicate verbs** and the `Mirror` and
   `Toml` enums and their dispatch arms. They're gone at compile time.
   Same with `Globalize` / `Deglobalize`.

5. **Delete `--sync` flags from edit verbs in args.** The handler
   signatures still accept `sync_live: bool` — pass `false`.
   Phase 04 prunes the unused parameter from `commands::skill::*`.

6. **Wire `sync pull --then-push`** in the dispatch: when `then_push`
   is true, after `commands::reconcile` succeeds, call
   `commands::sync` with the same scope selection. Halt and propagate
   the error if pull fails — do not push on a failed pull.

7. **Wire the global `--dry-run`** by stashing it on `Context` (or
   passing it through to every mutating call site). Recommended:
   add `pub dry_run: bool` to `Context` so handlers read
   `ctx.dry_run` instead of taking a per-call argument. Update the
   existing handlers' signatures to drop their `dry_run` parameter
   and read it from context — this is a wide but mechanical change.
   Verify with `rg 'dry_run' src/`.

8. **Verify the help output.** Run `cargo run -- --help` and the
   subcommand helps; capture into a scratch file and eyeball-check
   against the tree in the README. The acceptance criteria pin this.

## Acceptance criteria

- [ ] `cargo build` clean, no warnings.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
      (Note: `--all-targets` includes tests, which will fail Phase 02 —
      use `cargo clippy --lib --bins` for this phase's gate, and re-run
      `--all-targets` once Phase 05 lands. Document this in the chat
      reply when running the phase.)
- [ ] `skillnet --help` lists exactly: `status`, `completions`,
      `sync`, `skill`, `scope`, `project`, `catalog`. Nothing else. No
      `mirror`, `toml`, `reconcile`, `globalize`, `deglobalize`, top-
      level `delete`/`rename`/`move`/`list`/`targets`/`sources`.
- [ ] `skillnet sync --help` lists `pull`, `push`, `status`, `diff`.
- [ ] `skillnet skill --help` lists `list`, `show`, `delete`,
      `rename`, `move`. No `globalize`/`deglobalize`.
- [ ] `skillnet scope --help` lists `list`, `sources`.
- [ ] `skillnet project --help` lists `list`, `add`, `remove`.
- [ ] `skillnet catalog --help` lists `generate`, `lint`, `search`.
      **No `show`** (still wired internally via `skill show` — Phase 04
      removes the underlying function).
- [ ] `skillnet` (no args) executes the same code path as `skillnet
status`. Both currently error with the Phase 03 marker — that's
      expected at end of Phase 02.
- [ ] `skillnet --dry-run sync push` (or any mutating subcommand)
      honors the dry-run flag globally. No `--dry-run` on individual
      subcommands.
- [ ] Old verb invocation errors at clap level with "unknown
      subcommand". E.g., `skillnet reconcile` →
      `error: unrecognized subcommand 'reconcile'`.
- [ ] `skillnet sync pull --then-push --scope global` runs the
      existing reconcile + sync code paths back to back for the global
      scope.
- [ ] `skillnet skill move global/foo myproj` parses to a `Move`
      command with `from=(Global, "foo")`, `to=(Project("myproj"), None)`
      and executes today's move_skill behavior.
- [ ] `Context` has a `pub dry_run: bool` field and every mutating
      handler reads it from `ctx`, not from a per-call parameter.

## Files likely touched

- `src/cli/args.rs` — **wholesale rewrite**. Expect ~250 lines down
  from 389 (no duplicates).
- `src/cli/mod.rs` — **wholesale rewrite**. Expect ~200 lines.
- `src/cli/scope.rs` — minor additions: a `resolve_scopes(config,
scope_args: &[String], all: bool) -> Result<Vec<Scope>>` helper if
  not already in Phase 01.
- `src/commands/context.rs` — add `pub dry_run: bool` field; update
  `Context::load` signature to take it.
- `src/commands/{mirror,project,skill}.rs` — drop per-function
  `dry_run: bool` parameters; read from `ctx.dry_run`. Mechanical
  signature changes; the bodies stay identical for now.
- `src/commands/mod.rs` — re-export changes if needed.
- `src/main.rs` — pass `--dry-run` through to `Context::load` if the
  loader signature changes there.
- `tests/cli.rs` — **do not touch**; will fail and is rewritten in
  Phase 05.

## Pitfalls

- **`Scope` value parser cannot be a `#[arg(value_parser = ...)]`
  literal**, because the valid-projects list is config-derived.
  Symptom: trying to attach the parser at derive-time fails because
  the projects list isn't known until `Config::load` runs. Cause:
  clap derive macros resolve at compile time. Recovery: accept
  `Vec<String>` from clap, validate inside `mod.rs::run` after config
  loads using the helper from Phase 01. Phase 01's
  `scope.rs` documentation flagged this — re-read it before starting.

- **`Option<Command>` for the no-args alias.** clap derive wants
  required subcommands by default. Mark the top-level subcommand as
  optional with `#[command(subcommand_required = false, arg_required_else_help = false)]`,
  then dispatch `None` → `Status`. Symptom: forgetting this prints
  help instead of running `status`. Recovery: add the attribute
  and a unit test on dispatch (`Cli::parse_from(["skillnet"])` →
  `None` command).

- **Drop-vs-rename for `--sync`.** Every edit verb today takes
  `--sync`. The new design says these flags are gone; the user must
  run `sync push` after. Symptom: if you keep `--sync` "for
  ergonomics", you've re-fragmented the very thing this rebuild is
  fixing. Recovery: delete `--sync` from args. Workflow ergonomics
  for "edit + sync" is satisfied by `skillnet skill move ... && skillnet sync push --scope ...`
  — document the pattern in `MIGRATION.md` (Phase 05).

- **`tests/cli.rs` failing during Phase 02.** Symptom: `cargo test`
  fails because the old verbs the tests invoke don't exist. Cause:
  tests are rewritten in Phase 05. Recovery: do not gate Phase 02 on
  `cargo test`. Use `cargo build` and `cargo clippy --lib --bins`
  only. The plan README's whole-set criteria gate full `cargo test`
  green at the end of Phase 05.

- **Help-text drift.** Symptom: README lists `sync pull` but
  `--help` shows `sync reconcile` because copy-paste. Recovery: at
  end of phase, diff `skillnet --help`, `skillnet sync --help`, etc.
  against the README tree by hand. Don't rely on rg — small
  ordering / wording drift is fine, but the verb names must match
  exactly.

- **Catalog show still callable internally.** Symptom: a user runs
  `skillnet catalog show foo` and gets a different error than they
  expect. Cause: Phase 02 removes `catalog show` from the args enum
  but `catalog::show` (the function) still exists for Phase 04 to
  wire into `skill show`. Recovery: the args-level removal is enough
  for users; the lingering internal fn is invisible. Phase 04 deletes
  it.

## Reference

- Phase 01 foundation modules: [`src/cli/scope.rs`](../../../src/cli/scope.rs),
  [`src/cache.rs`](../../../src/cache.rs) (both new in Phase 01).
- Old args to replace: [`src/cli/args.rs`](../../../src/cli/args.rs).
- Old dispatch to replace: [`src/cli/mod.rs`](../../../src/cli/mod.rs).
- Target tree: [README.md § Target command tree](./README.md#target-command-tree).
- Plan README: [README.md](./README.md).
