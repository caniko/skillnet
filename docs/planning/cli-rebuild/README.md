# Plan: skillnet CLI rebuild

> **Recommended Codex model for plan-set orchestration: GPT 5.5 high**
>
> Plan-set coordination spans five phases that touch every layer of the CLI
> crate (args, dispatch, commands, catalog, tests). The orchestrator role
> at this complexity warrants `5.5 high` — non-trivial design decisions
> about the cache schema, divergence semantics, and dispatch shape need
> to stay coherent across phases. Don't downgrade.

## Scope

Rebuild the `skillnet` command surface from the fragmented status quo
(top-level verbs duplicated under `mirror`/`skill`/`toml`, scattered
`--sync` flags, magic-string `--target`) into a clean tree organized
around the user's mental model: `sync`, `skill`, `scope`, `project`,
`catalog`. Clean break — no hidden aliases. Ship a `MIGRATION.md`.

The current surface is documented exhaustively in
[`src/cli/args.rs`](../../../src/cli/args.rs) (389 lines, ~20 verb
duplications). The reorganization is design-locked; this plan executes
it.

## Locked decisions

- **Plan A — clean break.** No hidden aliases, no deprecation window.
  One release; `MIGRATION.md` maps old → new for users.
- **`sync pull --then-push`** is the composed shortcut (replaces
  `reconcile --sync`). Bare `sync` with no subcommand is **not** a
  thing; it errors with a usage hint.
- **Fold `catalog show` into `skill show`.** One lookup surface per
  skill: file metadata + catalog entry, side by side. `catalog
  generate` / `catalog lint` / `catalog search` stay where they are.
- **Status caching.** Cache file under `mirror_root/.skillnet/cache.toml`
  records per-scope last-pull timestamp + content hash. `status` and
  `sync status` only re-walk scopes whose live source mtime is newer
  than the cache stamp; otherwise they trust the cache.
- **Type-safe `--scope`.** Repeatable, value-parser knows configured
  projects (so completion works and typos error at parse time).
  `--all` is an explicit opt-in for "every configured scope". Default
  (no `--scope`, no `--all`) is **all scopes**, matching today's
  `--target all` default.

## Target command tree

```
skillnet                              # alias for `status`
skillnet status                       # scopes + divergence + catalog health
skillnet completions <shell>

skillnet sync
  ├─ pull   [--scope ...] [--all] [--then-push]
  ├─ push   [--scope ...] [--all]
  ├─ status [--scope ...]             # read-only divergence per scope
  └─ diff   [--scope ...]             # file-level diff mirror↔live

skillnet skill
  ├─ list   [--scope ...] [--all]
  ├─ show   <scope>/<skill>           # absorbs `catalog show`
  ├─ delete <scope>/<skill>
  ├─ rename <scope>/<old> <new>
  └─ move   <from>/<skill> <to>[/<as>]

skillnet scope
  ├─ list                             # was: targets
  └─ sources [--scope global|<proj>]  # both groups by default

skillnet project
  ├─ list
  ├─ add    <name> <path> [--allow-missing]
  └─ remove <name> [--prune-mirror]

skillnet catalog
  ├─ generate
  ├─ lint
  └─ search <query>
```

Globals (apply to whichever mutating subcommand runs): `--config`,
`--mirror-root`, `--catalog-config`, `--dry-run`.

## Phases

| # | File | Slug | Model | Depends on | Touches | Parallel with |
|---|------|------|-------|------------|---------|---------------|
| 01 | [01-foundation.md](./01-foundation.md) | foundation | `5.5 medium` | — | `src/cli/scope.rs` (new), `src/cache.rs` (new), `src/cli/args.rs` (types only), `src/lib.rs` | — |
| 02 | [02-cli-surface.md](./02-cli-surface.md) | cli-surface | `5.5 high` | 01 | `src/cli/args.rs` (rewrite), `src/cli/mod.rs` (rewrite), `src/commands/mod.rs` | — |
| 03 | [03-sync-and-status.md](./03-sync-and-status.md) | sync-and-status | `5.5 high` | 02 | `src/commands/sync.rs` (new), `src/commands/status.rs` (new), `src/cache.rs` (impl), `src/reconcile.rs` | 04 |
| 04 | [04-skill-and-catalog.md](./04-skill-and-catalog.md) | skill-and-catalog | `5.5 medium` | 02 | `src/commands/skill.rs`, `src/catalog/mod.rs`, `src/catalog/render.rs` (show removal) | 03 |
| 05 | [05-tests-and-migration.md](./05-tests-and-migration.md) | tests-and-migration | `5.5 medium` | 03, 04 | `tests/cli.rs` (rewrite), `MIGRATION.md` (new) | — |

## Parallelism layer (execution waves)

**Wave 0 — Foundation.** Phase 01 only. It introduces new modules
(`scope`, `cache`) and lays out global args / types. Everything else
depends on this. Single phase, no parallelism.

*Unlock condition: Phase 01's acceptance criteria pass; new types
compile and are unused-but-warning-free.*

**Wave 1 — Surface.** Phase 02 only. It rewrites `args.rs` and
`mod.rs` wholesale. The new tree exists; dispatch routes to handler
stubs (or thin wrappers that call existing functions). The binary
builds and the new help text matches the design.

*Unlock condition: `cargo build` clean, `skillnet --help` shows the
new tree, every old top-level verb is gone (compile-time, not just
hidden).*

**Wave 2 — Fanout (parallel).** Phases 03 and 04 run concurrently.
They touch disjoint files: 03 owns sync verbs + the new
`status`/`sync`/`cache` modules + `reconcile.rs` (cache write hook).
04 owns `skill` verbs + the catalog show fold. No file conflicts.

The user can dispatch them in two sessions or run sequentially if
preferred. Either phase landing first does not block the other.

*Unlock condition: both phases' acceptance criteria pass.*

**Wave 3 — Tests + docs.** Phase 05 only. The integration tests in
`tests/cli.rs` exercise the old verbs heavily; they need wholesale
rewriting once 03 and 04 have landed. `MIGRATION.md` documents the
old → new mapping for users.

*Unlock condition: full test suite green; `MIGRATION.md` covers every
removed verb; plan is exhausted.*

## Whole-set acceptance criteria

- [ ] `cargo build` clean, no warnings.
- [ ] `cargo test` green; every test exercises the new surface.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `skillnet --help` shows exactly the tree above. No `mirror`,
  `toml`, `globalize`, `deglobalize`, top-level `reconcile`/`sync`/
  `delete`/`rename`/`move`/`list`/`targets`/`sources` verbs.
- [ ] `skillnet <old-verb>` (any removed verb) errors with a clap-
  level "unknown subcommand" — the help is the migration cue;
  `MIGRATION.md` is the canonical reference.
- [ ] `--dry-run` works as a global flag on every mutating
  subcommand; no per-subcommand `--dry-run` remains.
- [ ] `--scope` is type-safe (typo → clap error citing valid scopes).
- [ ] `skillnet` (no args) runs `status`.
- [ ] `skillnet status` prints, for each configured scope:
  scope name, last-pulled timestamp (from cache), divergence
  summary (clean / N files diverged), plus a catalog lint summary.
- [ ] `skillnet sync pull --then-push` runs pull and then push in
  sequence; failure in pull aborts before push.
- [ ] `skillnet skill show <scope>/<skill>` shows file metadata
  **and** the catalog entry in one output.
- [ ] `mirror_root/.skillnet/cache.toml` exists after a pull and
  contains per-scope stamps; subsequent `status` calls don't
  re-walk live sources whose mtime hasn't advanced.
- [ ] `MIGRATION.md` covers every removed/renamed verb and flag.

## Global constraints

- **No hidden aliases.** Removed verbs are gone at compile time, not
  just `#[command(hide = true)]`. Don't sneak them back.
- **Dispatch goes through new types.** Phase 02 introduces the
  `Scope`, `SkillPath` (parsed `<scope>/<skill>`) types from Phase 01
  and uses them through to the handlers — don't accept `&str` at the
  handler boundary "for now". The whole point is type-safety.
- **Cache is best-effort.** Missing or corrupt cache → fall back to a
  full walk; never error out of `status` because of a cache problem.
  Cache writes happen on `sync pull` only.
- **`MIGRATION.md` lives at repo root.** Users look there; not under
  `docs/`.

## Reference

- Current CLI surface: [`src/cli/args.rs`](../../../src/cli/args.rs).
- Current dispatch: [`src/cli/mod.rs`](../../../src/cli/mod.rs).
- Current handlers: [`src/commands/`](../../../src/commands/).
- Current catalog: [`src/catalog/`](../../../src/catalog/).
- Reconcile + sync core: [`src/reconcile.rs`](../../../src/reconcile.rs).
- Design discussion (pre-plan): chat transcript leading to this plan
  set, summarized in `## Locked decisions` above.
