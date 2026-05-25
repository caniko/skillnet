# Phase 04 — Skill namespace + catalog show fold

> **Recommended Codex model: GPT 5.5 medium**
>
> Sub-agent role, moderate complexity. The skill verbs are largely a
> mechanical refactor: today's handlers take `(scope: &str, skill:
&str)`; rewrite them to take `&SkillPath`. The catalog show fold is
> the design-touchy part — deciding what `skill show` displays from
> the catalog entry versus the on-disk file, and how the merged
> output reads. `low` would treat the fold as "just call both
> functions and concat output", which is wrong. `high` is overkill —
> the design surface here is one output format and a handful of
> handler signatures.

## Working tree

`/data/nvme0/can/Projects/ai-skills`.

Phase 02 must have landed (the new `Skill::*` args exist; handlers
may call into today's `commands::skill::*` and `catalog::show`).

Runs in parallel with Phase 03; touches disjoint files
(`src/commands/skill.rs`, `src/catalog/`). Coordinate at the
dispatch layer in `src/cli/mod.rs` — both phases edit it, but in
different match arms.

## Goal

Bring the skill namespace to its final shape: handlers take typed
`SkillPath` arguments, no `--sync` flag remnants, no `dry_run`
parameters (read from `ctx`), and `skill show` is the single lookup
surface — one command, one output — that merges file metadata with
the catalog entry. Drop `catalog show` from the `catalog/` module
entirely.

## Why this matters now

The Phase 02 dispatch layer is currently passing `String` paths into
handlers that still take `(&str, &str)`. This works (the dispatch
parses then re-splits, or passes pre-split parts) but defeats the
type-safety the rebuild promised. The handler signatures need to
finish migrating to `SkillPath`.

The catalog show fold also has to happen now to avoid documentation
drift — Phase 05's `MIGRATION.md` documents one lookup surface;
shipping the rebuild with both `catalog show` (still callable
internally) and `skill show` (a thin wrapper) breaks the promise.

## Out of scope

- Do **not** touch `src/commands/sync.rs`, `src/commands/status.rs`,
  or `src/cache.rs`. Phase 03's territory; file-level conflict if
  this phase races there.
- Do **not** rewrite tests. Phase 05.
- Do **not** add new catalog features (richer metadata, new
  fields). The fold is purely a refactor: the union of today's
  `skill show` and `catalog show` outputs, deduplicated where they
  overlap.
- Do **not** add interactive prompts to `skill delete` or `skill
rename` ("are you sure?"). Not a goal of the rebuild.

## Plan

1. **Update handler signatures in `src/commands/skill.rs`**:
   - `delete(ctx, path: &SkillPath)` instead of `(ctx, scope, skill,
sync_live, dry_run)`.
   - `rename(ctx, path: &SkillPath, new: &str, force: bool)`.
   - `move_skill(ctx, from: &SkillPath, to_scope: &Scope, to_name:
Option<&str>, copy: bool, force: bool)`. Drop the `sync_live`
     parameter — sync is no longer composable from edit verbs.
   - All read `dry_run` from `ctx.dry_run`.

2. **Add `pub fn show(ctx: &Context, path: &SkillPath) -> Result<()>`**
   to `src/commands/skill.rs`. This is the absorbed surface:
   - Resolve the mirror path `<mirror_root>/<scope-dir>/<skill>`.
   - Read the skill directory: list files at top level, find a
     `SKILL.md` (or whatever the convention is — check
     `src/catalog/discover.rs`), parse its frontmatter via
     `catalog::frontmatter` to extract description/tags/etc.
   - Look up the catalog entry from `catalog::discover` or whatever
     today's `catalog::show` uses — reuse the resolver, don't
     re-implement.
   - Print in two sections:

     ```
     skill: <scope>/<skill>
     path:  <absolute mirror path>
     files: <count> files, <count> dirs
       SKILL.md (<bytes>)
       <other top-level entries>

     catalog entry:
       description: <from frontmatter>
       tags:        <from catalog>
       category:    <from catalog>
       routing:     <from catalog>
       ...
     ```

     If no catalog entry exists for this skill, print `catalog
entry: (none — run 'skillnet catalog generate')`. Don't error.

3. **Delete `pub fn show` from `src/catalog/mod.rs`** (or wherever
   `catalog::show` lives — grep first). Move any reusable helpers
   it called (frontmatter parse, catalog entry resolution) to be
   `pub` so `commands::skill::show` can call them. Don't duplicate
   logic.

4. **Update dispatch in `src/cli/mod.rs`**:
   - `SkillCommand::Show { path }` → parse `path` via
     `SkillPath::parse(&ctx, &path)`, then call
     `commands::skill::show(&ctx, &skill_path)`. Replace the Phase
     02 stub that called `catalog::show`.
   - `SkillCommand::Delete`, `Rename` → switch to new
     `SkillPath`-typed handler calls.
   - `SkillCommand::Move { from, to }` → parse both. `from` is
     `<scope>/<skill>` (required); `to` is either `<scope>` (rename
     within destination keeps skill name) or `<scope>/<name>` (rename
     on move). Add a small `parse_move_target(input: &str) -> (Scope,
Option<String>)` helper.

5. **Drop the `--sync` parameter** from every `commands::skill::*`
   handler signature. The Phase 02 dispatch was passing `false`; now
   it's not passing it at all. Update the function bodies to remove
   the unused `maybe_sync` call and the `sync_live: bool` parameter.

6. **Verify all old verb-related code is gone**:
   - `rg 'globalize|deglobalize' src/` → no matches.
   - `rg '--sync' src/` → no matches (or only matches in doc
     comments, which should be updated).
   - `rg 'catalog::show|catalog_show' src/` → no matches.
   - `rg '\.target\(' src/commands/` → calls should use `&Scope` or
     `&SkillPath`, not raw strings. Update `Context::target` to
     accept `&Scope` if it doesn't already (it might still take
     `&str` for compatibility with project add/remove; check before
     changing).

7. **Smoke-test manually**:
   - `skillnet skill list --all` prints today's list output (just
     verify it still works after the signature changes).
   - `skillnet skill show global/<some-skill>` prints the merged
     file + catalog view.
   - `skillnet skill show global/<nonexistent>` errors clearly with
     "skill not found in scope".
   - `skillnet skill move global/foo myproj` moves; `skillnet skill
move global/foo myproj/bar` moves and renames.
   - `skillnet skill delete global/foo` deletes from the mirror;
     `skillnet sync push --scope global` then mirrors the deletion
     to live (verify the two-step workflow is ergonomic).

## Acceptance criteria

- [ ] `cargo build` clean, no warnings.
- [ ] `cargo clippy --lib --bins -- -D warnings` clean.
- [ ] `rg 'globalize|deglobalize' src/` returns no matches.
- [ ] `rg 'catalog::show|catalog_show' src/` returns no matches.
- [ ] `rg 'sync_live|--sync' src/` returns no matches in source code
      (doc/comment hits in `MIGRATION.md`-bound text are fine if any
      exist).
- [ ] `commands::skill::*` handlers take `&SkillPath` or `&Scope`,
      not `&str` scope/skill pairs.
- [ ] `commands::skill::show` exists and prints the merged view.
- [ ] `skillnet skill show global/<skill>` shows both file metadata
      (path, file list, frontmatter summary) and the catalog entry in
      one output.
- [ ] `skillnet skill show <scope>/<missing-skill>` errors with a
      clear message ("skill `<missing-skill>` not found in scope
      `<scope>`"), not a panic.
- [ ] `skillnet catalog --help` does **not** list `show` (Phase 02
      already removed it from args; Phase 04 removes the underlying fn).
- [ ] `skillnet skill move global/foo myproj` moves the skill.
- [ ] `skillnet skill move global/foo myproj/bar` moves + renames.
- [ ] `skillnet skill delete global/foo` deletes; subsequent
      `skillnet sync push --scope global` removes from live.

## Files likely touched

- `src/commands/skill.rs` — handler signatures + new `show` fn.
  ~180 lines after.
- `src/catalog/mod.rs` — delete `show` function; possibly re-export
  helpers it called.
- `src/catalog/render.rs` — possibly trim if `show`'s rendering
  helpers lived here.
- `src/cli/mod.rs` — update `SkillCommand::*` dispatch arms to use
  typed paths and call the new `show`.
- `src/commands/context.rs` — possibly relax `target(&str)` to also
  accept `&Scope` via a new `target_for_scope(&Scope)` overload.

## Pitfalls

- **`skill show` becoming a kitchen sink.** Symptom: it tries to
  also print git status, last-modified, dependencies, lint warnings.
  Recovery: the fold is _only_ file metadata + catalog entry.
  Anything else is future work. If a section feels like it belongs,
  ask: "would Phase 05's MIGRATION.md need to document this section
  as a new feature?" If yes, defer it.

- **Catalog helpers becoming over-public.** Symptom: making
  internal catalog parsing functions `pub` to support `skill show`
  leaks abstractions and grows the API surface. Recovery: prefer a
  single `pub fn catalog::entry_for(&ctx, &SkillPath) -> Option<&CatalogEntry>`
  resolver that hides the internals. `skill::show` calls only that
  one entry point.

- **`SkillPath` re-parsing on every handler call.** Symptom: the
  dispatch parses once, then the handler also parses (defensive).
  Cause: not trusting upstream. Recovery: parsing happens at the
  dispatch boundary, exactly once, in `cli/mod.rs::run`. Handlers
  take `&SkillPath` and trust it. Document this on the handlers.

- **`Move`'s `to` ambiguity.** Symptom: `skill move global/foo bar`
  — is `bar` a project scope or a new skill name in `global`? In
  the design, `<to>` without `/` means "rename within source scope".
  But the user said `to_scope` is required. Re-read the README: the
  syntax is `<from>/<skill> <to>[/<as>]`. `<to>` is always a scope;
  the optional `/<as>` is a rename. So `skill move global/foo bar`
  means scope=`bar`. If `bar` isn't a known scope, parse errors at
  dispatch time. Document this in `skill move`'s help text.

- **`catalog::show` callers outside `cli/mod.rs`.** Symptom: deleting
  the function breaks something internal. Recovery: `rg
'catalog::show'` before deletion; the function is small enough to
  rewrite if needed. Phase 02 already removed the args-level entry
  point, so callers should only be internal — likely none.

- **The `delete` workflow's two-step ergonomics.** Symptom: users
  complain that "delete then sync push" is more typing than
  `delete --sync`. Recovery: don't re-add `--sync`. Document the
  workflow in `MIGRATION.md` (Phase 05). The umbrella the user
  asked for is worth the two-step.

## Reference

- Phase 01 `SkillPath` parser: [`src/cli/scope.rs`](../../../src/cli/scope.rs).
- Old skill handlers to refactor:
  [`src/commands/skill.rs`](../../../src/commands/skill.rs).
- Old `catalog::show`:
  [`src/catalog/mod.rs`](../../../src/catalog/mod.rs) (look for
  `pub fn show`).
- Catalog entry discovery:
  [`src/catalog/discover.rs`](../../../src/catalog/discover.rs).
- Frontmatter parser:
  [`src/catalog/frontmatter.rs`](../../../src/catalog/frontmatter.rs).
- Plan README: [README.md](./README.md).
