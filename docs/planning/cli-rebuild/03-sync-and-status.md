# Phase 03 — Sync namespace, status cache, top-level status

> **Recommended Codex model: GPT 5.5 high**
>
> Sub-agent role, complex task. This phase carries three intertwined
> design decisions: what "divergence" means concretely (file presence
> vs content hash), when the cache is valid (mtime walk threshold and
> per-source vs per-scope granularity), and how the no-args `status`
> front door composes scope summaries. None are frontier work, but
> any one of them done sloppily becomes a footgun: a wrong divergence
> definition means `sync status` lies; a wrong cache invalidation
> rule means stale results; a wrong status layout means the front
> door is noisy and ignored. `medium` would skim past these; `max`
> isn't warranted because the surrounding scaffolding (Phase 02) has
> already constrained the shape.

## Working tree

`/data/nvme0/can/Projects/ai-skills`.

Phase 02 must have landed: the new command tree exists, dispatch
routes to handler stubs, `Status`/`Sync::Status`/`Sync::Diff` are
stubbed with the Phase 03 marker error.

## Goal

Implement the sync namespace end-to-end (`pull`, `push`, `status`,
`diff`, including `pull --then-push`), implement the cache as the
authoritative "is this scope clean?" oracle, and implement the
top-level `status` command as the front door of the CLI. After this
phase, a user can:

- Run `skillnet` and see a coherent overview of every scope.
- Run `skillnet sync status` to ask "what diverged?" without writing.
- Run `skillnet sync diff` to see the actual file-level deltas.
- Run `skillnet sync pull --then-push` and watch a clean pull + push
  pipeline.
- Trust that `status` doesn't lie about a stale scope just because
  nobody touched the live sources.

## Why this matters now

The user explicitly called out the sync umbrella and the status cache
as the two biggest UX wins of the rebuild. Phase 02 set up the tree;
03 is where the umbrella actually becomes useful. Without this phase
the rebuild is mostly cosmetic.

The cache also has to be wired _now_, before Phase 04 tries to use
`status` to validate its own changes. If 04 lands first and `status`
is still stubbed, the verify pass on 04 has nothing to lean on.

## Out of scope

- Do **not** touch `src/commands/skill.rs` or `src/catalog/`. Those
  are Phase 04's territory; landing them here causes file conflicts
  on the Wave 2 parallel fanout.
- Do **not** rewrite the integration tests in `tests/cli.rs`. Phase 05.
- Do **not** add a `--json` or `--quiet` output mode on `status` or
  `sync status`. Out of scope for the rebuild; future work.
- Do **not** parallelise the per-scope walks. Sequential is fine for
  a few-dozen-scope workload. Future work if profiling justifies it.

## Plan

1. **Define divergence semantics**, in code and in doc comments:
   - A scope is **clean** if, for every file path that exists in
     either the mirror or any live source for that scope, the file
     exists in both and their content hashes match.
   - A scope is **diverged** otherwise.
   - "Content hash" reuses `fs_ops::content_signature` (today's
     directory-level signature). For per-file diff (`sync diff`),
     use byte-level diff on the file contents — see step 4.

   Document this in a top-of-module doc comment in
   `src/commands/sync.rs`.

2. **Create `src/commands/sync.rs`** with these public functions:
   - `pub fn pull(ctx: &Context, scopes: &[Scope], then_push: bool) -> Result<()>`
     — reuses today's `reconcile::reconcile_target` per scope, then
     writes the cache stamp for each pulled scope, then if `then_push`
     calls `push` with the same scopes.
   - `pub fn push(ctx: &Context, scopes: &[Scope]) -> Result<()>`
     — reuses today's `reconcile::sync_target`.
   - `pub fn status(ctx: &Context, scopes: &[Scope]) -> Result<()>`
     — for each scope, consult the cache; if stale (live mtime newer
     than cached stamp), re-walk + recompute; print a one-line summary
     per scope: `<scope>  clean | diverged (N files)  last-pulled <relative-time>`.
   - `pub fn diff(ctx: &Context, scopes: &[Scope]) -> Result<()>`
     — for each scope, list per-file deltas: `+ path` (only in
     live), `- path` (only in mirror), `~ path` (content differs,
     show first 3 diff lines or just mark as differs — keep simple
     for now).

3. **Wire the cache writes in `pull`**, not in `reconcile.rs`. Keep
   `reconcile.rs` ignorant of the cache; sync.rs is the orchestration
   layer that knows about both. Steps inside `pull`:
   1. For each scope, call `reconcile::reconcile_target(&target)`.
   2. After it succeeds, compute `live_source_max_mtime` and
      `content_signature` of the mirror dir.
   3. Build a `ScopeStamp` and insert into the loaded `Cache`.
   4. After all scopes pulled, `cache::save(&ctx.mirror_root, &cache)`.
   5. If `then_push`, dispatch to `push`.

4. **Wire `sync status` to consult the cache**:
   - `cache = cache::load(&ctx.mirror_root)`.
   - For each scope, fetch `cache.stamps.get(scope.to_string())`.
   - Compute the live source max mtime cheaply (this is the only walk
     we have to do; the rest is hashing only on staleness).
   - If no stamp, or stamp is stale per `cache::is_stale`, walk the
     scope and re-compute the divergence; do **not** update the cache
     here (status is read-only).
   - If stamp is fresh, trust it: `clean` if mirror's current
     `content_signature` equals stamp's `mirror_content_hash`,
     otherwise `diverged`. (This catches mid-flight mirror edits.)

   Status writes are pull-only. Document this invariant in the
   module.

5. **Create `src/commands/status.rs`** with `pub fn run(ctx: &Context) -> Result<()>`:
   - Section 1: configured scopes (count, names — `scope list`
     condensed inline).
   - Section 2: per-scope divergence summary (same one-line format
     as `sync status`).
   - Section 3: catalog health — count of skills, count of catalog
     lint failures (call `catalog::lint` and capture results
     non-fatally; print "catalog: N skills, M lint issues" or
     "catalog: N skills, clean").
   - Section 4: cache freshness — print path and "last updated X
     ago" or "no cache yet (run `skillnet sync pull`)".

   Format as plain text; no tables. Short and scannable.

6. **Wire dispatch in `src/cli/mod.rs`**: replace the Phase 02 stubs
   for `Command::Status`, `SyncCommand::Status`, `SyncCommand::Diff`
   with real calls. `Command::Status` calls `commands::status::run`.
   `SyncCommand::Status` calls `commands::sync::status`.

7. **Replace `commands::reconcile` and `commands::sync`** call sites
   in dispatch with `commands::sync::pull` and `commands::sync::push`.
   The old functions in `commands::mirror` can either be deleted (if
   unused after this swap) or left for Phase 04 to clean up. Prefer
   deleting now if they're dead — verify with `rg`.

8. **Update `reconcile.rs` only if necessary.** The cache write
   lives in `sync::pull`, not `reconcile.rs`. The only edit
   `reconcile.rs` may need is exposing whatever it needs to return so
   that `sync::pull` can compute the post-pull hash without a second
   walk. If `reconcile_target` already returns enough info or
   `fs_ops::content_signature(&mirror_path)` is cheap on a freshly
   pulled mirror, no `reconcile.rs` change is needed. Check before
   editing.

9. **Smoke-test manually** before declaring the phase done:
   - Fresh clone, no cache: `skillnet status` prints "no cache yet".
   - `skillnet sync pull --scope global`: writes
     `.skillnet/cache.toml`; subsequent `skillnet status` reads it.
   - Touch a file in a live source; `skillnet status` notes
     divergence on that scope.
   - `skillnet sync diff --scope <that-scope>` shows the changed path.
   - `skillnet sync pull --then-push --scope global` runs pull then
     push; introduce a deliberate pull failure (e.g., corrupt a
     source) and verify push does **not** run.

## Acceptance criteria

- [ ] `cargo build` clean, no warnings.
- [ ] `cargo clippy --lib --bins -- -D warnings` clean.
      (`--all-targets` still excluded until Phase 05.)
- [ ] `src/commands/sync.rs` exists with `pull`, `push`, `status`,
      `diff` exported.
- [ ] `src/commands/status.rs` exists with `run` exported.
- [ ] Dispatch in `src/cli/mod.rs` routes `Sync::*` and `Status` to
      the new functions; the Phase 02 marker errors are gone.
- [ ] After `skillnet sync pull --scope global`, the file
      `<mirror_root>/.skillnet/cache.toml` exists and contains a
      `[stamps.global]` table with `last_pulled_at`, `live_source_max_mtime_nanos`,
      `mirror_content_hash`.
- [ ] `skillnet status` on a fresh checkout (no cache) prints a
      "no cache yet" hint and still produces a divergence summary by
      doing a one-shot walk; it does **not** write the cache.
- [ ] `skillnet sync status` on a scope whose live mtime hasn't
      changed since the last pull does **not** walk live sources
      recursively — verify by `strace -c -e openat skillnet sync status`
      or by adding a temporary log line and removing it. (Acceptance is
      "verified once, not regression-gated"; an `#[cfg(debug)]` counter
      is fine but not required.)
- [ ] `skillnet sync pull --then-push --scope global` runs both
      operations sequentially; a forced pull failure aborts before push.
      Test by temporarily breaking a source path.
- [ ] `skillnet sync diff` prints per-file deltas (`+ path`,
      `- path`, `~ path`) with no panics on a deliberately divergent
      scope.
- [ ] `skillnet` (no args) calls `Status::run` (the no-args alias).
- [ ] Cache is best-effort: deleting `.skillnet/cache.toml` mid-use
      does not break any command; commands that need it recompute.
      Corrupting the file (e.g., `echo garbage > cache.toml`) does not
      break `status` — it falls back to a full walk and logs nothing
      alarming.

## Files likely touched

- `src/commands/sync.rs` — **new**, ~250 lines.
- `src/commands/status.rs` — **new**, ~120 lines.
- `src/commands/mod.rs` — add `pub mod sync; pub mod status;`.
- `src/commands/mirror.rs` — possibly delete if all callers are
  gone after rerouting; check with `rg 'commands::mirror'`.
- `src/cli/mod.rs` — replace stubs with real calls; minor.
- `src/cache.rs` — possibly add a `hash_mirror_scope(target: &Target) -> Result<String>`
  helper if it makes `sync::pull` cleaner; otherwise unchanged.
- `src/reconcile.rs` — likely unchanged. Only edit if `pull` needs
  something it doesn't already expose.
- `Cargo.toml` — likely unchanged. If you need a human-readable
  relative time for `last_pulled_at`, add a minimal helper inline
  rather than pulling `chrono` (`SystemTime::elapsed` + format is
  enough).

## Pitfalls

- **Cache writes from `status` create a feedback loop.** Symptom:
  `status` and `sync status` start to silently mutate cache entries,
  making "no-op idempotent observation" suddenly stateful. Cause:
  convenience temptation — "we just walked, may as well save". Don't.
  Cache is `sync pull`-only. Document this in the module top.
  Recovery: if you find yourself writing cache from anywhere other
  than `pull`, stop.

- **`live_source_max_mtime` racing the walk.** Symptom: between
  taking the mtime and computing the hash, a user edits a live file;
  the cached stamp claims clean-as-of-T but the mirror lags. Cause:
  ordering. Recovery: compute hash _first_, then mtime, then write.
  The stamp's promise becomes "as of this mtime, the mirror matched
  the hash" — which is what `is_stale` needs anyway. Document the
  ordering.

- **Per-source-dir vs per-scope mtime.** A scope can pull from N
  sources (today's `selected_targets` resolution). Walking each
  separately and taking the max is correct but slow on large source
  trees. Symptom: `status` slow on cold runs. Recovery: that's the
  cost of cold; subsequent runs hit the cache. Don't over-engineer.

- **TOML round-trip drift.** Symptom: writing the cache, reading it
  back, comparing — `SystemTime` round-trips losslessly via
  serialization but display formats differ. Cause: TOML datetime
  precision. Recovery: store as `u128` nanos since epoch alongside or
  instead of `SystemTime`; the Phase 01 schema already uses
  `live_source_max_mtime_nanos: u128`, so apply the same to
  `last_pulled_at`. If Phase 01 used `SystemTime`, change it now
  and re-run the Phase 01 unit test.

- **`diff` output format scope creep.** Symptom: 200 lines of unified
  diff per file on a large divergence. Cause: trying to be `git diff`.
  Recovery: keep it short — `+ path`, `- path`, `~ path` plus
  _optionally_ the first 3 lines of unified diff for `~`. Future
  work can add `--full`.

- **Catalog lint as part of `status`.** Symptom: `status` becomes
  slow because `catalog::lint` walks every skill. Cause: lint isn't
  cached. Recovery: live with it for now; the lint cost is bounded by
  skill count, which is small. Future work: cache lint results in
  the same `.skillnet/cache.toml`.

- **`then_push` partial failure.** Symptom: pull succeeds for 5/7
  scopes, then fails on scope 6; should push run for the 5 that
  succeeded? Recovery: no. Treat the pull as atomic at the
  command level: any scope failing aborts the whole command, push is
  not attempted. The cache writes for the 5 successful scopes do
  persist (they were written incrementally before scope 6 failed),
  but `--then-push` is skipped. Document this.

## Reference

- Phase 01 cache module: [`src/cache.rs`](../../../src/cache.rs).
- Phase 01 scope types: [`src/cli/scope.rs`](../../../src/cli/scope.rs).
- Today's pull-equivalent: `reconcile::reconcile_target` in
  [`src/reconcile.rs`](../../../src/reconcile.rs).
- Today's push-equivalent: `reconcile::sync_target` in the same file.
- Existing content hashing: `fs_ops::content_signature` in
  [`src/fs_ops.rs`](../../../src/fs_ops.rs).
- Plan README: [README.md](./README.md).
