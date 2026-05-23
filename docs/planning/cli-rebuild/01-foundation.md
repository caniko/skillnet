# Phase 01 — Foundation: scope types, global flags, cache module skeleton

> **Recommended Codex model: GPT 5.5 medium**
>
> Sub-agent role, moderate complexity. The work is mostly type design —
> a `Scope` enum with a clap value parser that hooks the configured
> projects list, a `SkillPath` parser for the new `<scope>/<skill>`
> argument syntax, and a `Cache` module skeleton with schema decided
> but no behavior wired. None of this is frontier work, but the design
> decisions made here ripple through every later phase, so `medium`
> over `low`: the rationale for each shape needs to be intentional, not
> mechanical.

## Working tree

`/data/nvme0/can/Projects/ai-skills`. Same repo as the rest of the
plan.

## Goal

Introduce the three new building blocks the rest of the rebuild stands
on, without touching the live command surface. After this phase, the
crate still compiles and behaves exactly as it does today; new
modules exist but are not yet wired into dispatch.

Concretely: a `Scope` enum + clap `ValueParser` that knows the
configured projects; a `SkillPath` parsed-string type for
`<scope>/<skill>`; a `cache` module with the schema, path resolver,
load/save stubs, and the mtime-walk helper — but no `status` wiring
yet.

## Why this matters now

The new command tree depends on type-safe scope parsing and on the
cache existing as a module. Doing this in Phase 02 alongside the
wholesale `args.rs` rewrite would entangle three independent design
decisions (command tree shape, scope type design, cache schema) into
one giant diff. Splitting foundation out keeps Phase 02 focused on the
surface and lets the type design get its own review pass.

The cache schema in particular benefits from being decided in
isolation — once Phase 03 starts wiring writes on pull, the schema is
effectively locked. A bad schema decided under pressure is a future
breaking change.

## Out of scope

- Do **not** touch `src/cli/args.rs` command definitions or
  `src/cli/mod.rs` dispatch. Only add the new types/modules and
  re-export them.
- Do **not** wire the cache to `reconcile.rs` or to `status`. The
  module exists, with `load`/`save`/`stamp_scope` as functions, but
  nothing calls them yet.
- Do **not** remove any existing verbs. That's Phase 02.
- Do **not** add a top-level `--dry-run` global flag yet. That's
  Phase 02 (it lands alongside the args rewrite).
- Do **not** add `MIGRATION.md`. That's Phase 05.

## Plan

1. **Add `src/cli/scope.rs`** with:
   - `pub enum Scope { Global, Project(String) }` deriving
     `Debug, Clone, PartialEq, Eq, Hash`.
   - `impl Display for Scope` printing `global` or the project name.
   - `impl FromStr` (used by `SkillPath` parsing).
   - `pub fn scope_value_parser(config: &Config) -> clap::builder::PossibleValuesParser`
     that builds the parser from `config.projects` + `"global"`. This
     is the **runtime-built** parser used in Phase 02 to attach to
     `--scope` arguments after `Config::load` runs.
   - `pub struct SkillPath { pub scope: Scope, pub skill: String }`
     with a `parse(input: &str, valid_scopes: &[Scope]) -> Result<Self>`
     that splits on the **first** `/` and validates the scope against
     the configured list. Reject empty skill or unknown scope with a
     clear error.

2. **Add `src/cache.rs`** with:
   - `pub struct Cache { pub stamps: BTreeMap<String, ScopeStamp> }`
     where the map key is `scope.to_string()` (`"global"` or project
     name).
   - `pub struct ScopeStamp { pub last_pulled_at: SystemTime, pub live_source_max_mtime_nanos: u128, pub mirror_content_hash: String }`.
     Serialise with `serde` + `toml`.
   - `pub fn cache_path(mirror_root: &Utf8Path) -> Utf8PathBuf`
     returning `<mirror_root>/.skillnet/cache.toml`.
   - `pub fn load(mirror_root: &Utf8Path) -> Cache` — returns
     `Cache::default()` on any error (missing file, parse error,
     IO error). **Never propagates an error.** Cache is advisory.
   - `pub fn save(mirror_root: &Utf8Path, cache: &Cache) -> Result<()>`
     — creates `.skillnet/` if needed, writes atomically (write to
     `cache.toml.tmp`, rename).
   - `pub fn live_source_max_mtime(sources: &[Utf8PathBuf]) -> Result<u128>`
     — walks every source dir, returns the max mtime in nanos. Skip
     dotfiles, skip non-directories silently.
   - `pub fn is_stale(stamp: &ScopeStamp, live_mtime: u128) -> bool`
     — `live_mtime > stamp.live_source_max_mtime_nanos`.

3. **Wire the modules into `src/main.rs`** by adding them to the lib
   tree (`mod cache;`, `mod cli` already exists — add `mod scope;`
   inside `cli/mod.rs` and `pub use`). Verify they compile.

4. **Add a small unit test in `src/cache.rs`** that round-trips a
   `Cache` through TOML and asserts the schema is stable. This pins
   the schema; Phase 03 must not silently break it.

5. **Add a small unit test in `src/cli/scope.rs`** that asserts:
   - `SkillPath::parse("global/foo", &[Global])` → ok.
   - `SkillPath::parse("nope/foo", &[Global])` → err mentions `nope`.
   - `SkillPath::parse("global/", &[Global])` → err mentions empty
     skill.
   - `SkillPath::parse("global", &[Global])` → err mentions missing
     `/`.

6. **Document the deprecated `"project"` selector.** The current code
   in [`src/commands/context.rs`](../../../src/commands/context.rs)
   accepts `--target project` (= every project, no global). The new
   surface drops this; record a TODO comment in `scope.rs` noting
   that if anyone misses it, the answer is `--scope a --scope b ...`
   or a future `--projects` flag. Do not implement either now.

## Acceptance criteria

- [ ] `cargo build` clean.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `cargo test` green, including the two new unit tests
  (cache round-trip + SkillPath parse).
- [ ] `src/cli/scope.rs` exists and exports `Scope`, `SkillPath`,
  `scope_value_parser`.
- [ ] `src/cache.rs` exists and exports `Cache`, `ScopeStamp`,
  `cache_path`, `load`, `save`, `live_source_max_mtime`, `is_stale`.
- [ ] No existing CLI behavior changed. `skillnet --help` output is
  byte-identical to before this phase (the new modules are unwired).
- [ ] No new dependencies added unless strictly required. (`toml` and
  `serde` should already be in `Cargo.toml` via the existing config
  loader; reuse them. `walkdir` may need adding if `live_source_max_mtime`
  needs recursive walking — check existing usage in `fs_ops.rs` first
  before adding.)

## Files likely touched

- `src/cli/mod.rs` — add `mod scope;` and re-exports. No dispatch
  changes.
- `src/cli/scope.rs` — **new**, ~120 lines including tests.
- `src/cache.rs` — **new**, ~180 lines including tests.
- `src/main.rs` — add `mod cache;` if `main.rs` declares modules; if
  modules are declared in `src/lib.rs` (check), add it there.
- `Cargo.toml` — possibly add `walkdir` if not present; otherwise
  unchanged.

## Pitfalls

- **Parser closures vs values.** `clap::builder::PossibleValuesParser`
  takes static or owned values; the project list is config-derived,
  so it has to be built at runtime *after* `Config::load`. This means
  the value parser is attached in `cli/mod.rs::run` after config
  loads, not in the `#[derive(Parser)]` definition. Document this
  attach point in `scope.rs` doc comments so Phase 02 doesn't try to
  set it as a clap attribute. Recovery: if Phase 02 hits this wall,
  go back and refactor to a `clap::builder::TypedValueParser`
  implementation that takes the project list at construction.

- **`SkillPath` slashes on Windows.** Camino uses forward slashes
  always. The parser splits on `/`, not `std::path::MAIN_SEPARATOR`.
  Don't accept backslashes. Recovery: explicit error if input
  contains `\`.

- **Cache file permissions.** `.skillnet/cache.toml` lives in the
  user's mirror root, which is a working tree. Don't chmod it; default
  file mode is fine. Don't worry about concurrent writers — skillnet
  is a single-user CLI; ignore TOCTOU on the cache.

- **`live_source_max_mtime` walking too much.** If a source dir
  contains a `node_modules` or `.git`, the walk explodes. Look at
  `fs_ops::newest_mtime_nanos` for the existing convention and reuse
  its filter logic. If it doesn't filter, follow up with a Phase 03
  optimization; for foundation, exact behavior matching `fs_ops` is
  the goal. Recovery: if benchmarks show >1s walks on real projects,
  add a `.gitignore`-aware filter in Phase 03.

- **Unit test discoverability.** `#[cfg(test)] mod tests {}` inside
  the new modules is the convention; don't add a new
  `tests/foundation.rs` — that's the integration tests' territory
  and is rewritten wholesale in Phase 05.

## Reference

- Existing mtime walker: [`src/fs_ops.rs`](../../../src/fs_ops.rs) —
  reuse `newest_mtime_nanos` and `content_signature` patterns.
- Existing config + project list:
  [`src/config.rs`](../../../src/config.rs).
- Existing `--target` magic-string parser to replace:
  [`src/commands/context.rs:44`](../../../src/commands/context.rs#L44)
  (`selected_targets`).
- Plan README: [README.md](./README.md).
