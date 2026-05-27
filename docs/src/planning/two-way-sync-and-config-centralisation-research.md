# Two-Way Sync And Config Centralisation Design

> Status: research closed, design finalised, ready for a `multi-phase-plan-*`
> handoff. Sections through "Risks And Constraints" are the research record;
> "Candidate Next Steps" is the high-level decomposition; "Final Design"
> closes every open decision and pins concrete specs (flag names, exit codes,
> Nix option types, error messages). No further design work is needed before
> planning.

## Goal And Trigger

User asked, in one breath, for three changes:

1. **Smart symlink materialisation.** Before `skillnet sync` replaces a view
   entry with a symlink, it must inspect the entry. If it is already a symlink,
   keep current behaviour. If it is a real file/directory, compare its age
   against the canonical version: older view → overwrite with symlink (current
   `--force` behaviour); newer view → promote the view's content back into the
   canonical store first, then create the symlink. "Newest version becomes the
   source, no matter what happened in between syncs."
2. **Single CLI command.** Behaviour above must be reachable with one
   invocation that "resolves all of the paths from its configuration", not a
   multi-step recipe.
3. **Centralised, NM-definable configs.** The two config files currently
   sitting at `/data/nvme0/can/Projects/ai-skills/skillnet.toml` and
   `/data/nvme0/can/Projects/ai-skills/skillnet.catalog.toml` must move to the
   user-level config location and be definable through the Nix module ("NM").

This dossier feeds a multi-phase plan to amend skillnet + its Home Manager
module + the ai-skills checkout. It extends, not replaces, the existing
[reconcile-pull-research.md](reconcile-pull-research.md), which already
designed the comparator ladder and the canonical-write primitives. The reader
should treat that dossier as the load-bearing reference for promotion
semantics; this dossier closes the remaining gaps the user just opened.

## Current Reality

### Skill promotion path already designed

[reconcile-pull-research.md](reconcile-pull-research.md) already specifies:

- The comparator outcomes (`Identical`, `ViewNewer`, `CanonicalNewer`,
  `EqualMtimeDifferentContent`, `BothAdvanced`, `AdoptCandidate`) and which
  ones can resolve automatically vs need user input
  ([reconcile-pull-research.md:236-267](reconcile-pull-research.md#L236-L267)).
- The library primitives needed (`newest_mtime_nanos`, `content_signature`,
  `copy_dir` in [src/fs_ops.rs:15-112](../../../src/fs_ops.rs#L15-L112)).
- Atomic per-skill staging mirroring the deleted `write_skill_set` shape
  ([reconcile-pull-research.md:269-285](reconcile-pull-research.md#L269-L285)).
- Per-target dirty-destination gating for both `mirror_root` and per-project
  canonicals ([reconcile-pull-research.md:269-275](reconcile-pull-research.md#L269-L275)).

What that dossier *did not* settle, which this user message now collapses:

- **Surface shape.** The prior recommendation was a two-tier surface:
  `skillnet view reconcile` + `skillnet project reconcile` + top-level
  `skillnet sync --pull`. The user's new request demands a single top-level
  command, so the prior recommendation is now overconstrained.
- **Default behaviour.** The prior dossier kept reconcile-pull opt-in to
  preserve the v0.5.0 "no reconcile" stance. The user's new request implies
  promotion should be the *default* of `skillnet sync`, with no opt-in flag at
  the call site.
- **Config location.** The prior dossier never addressed where the configs
  live; it implicitly assumed today's working-directory pickup.

### Current sync command shape

`skillnet sync` ([src/cli/args.rs:71-79](../../../src/cli/args.rs#L71-L79),
[src/cli/mod.rs:85-91](../../../src/cli/mod.rs#L85-L91)) runs
`commands::view::sync` then `commands::project_sync(ctx, &[], true, ...)`
unconditionally — a single shot covering every configured global view *and*
every configured project view. It already resolves `mirror_root`, every
configured project root, and every view destination from `Config`
([src/config.rs:217-285](../../../src/config.rs#L217-L285)), so the "resolve
all paths from configuration" half of ask 2 is *already true today*.

What it does not do: handle non-symlink view entries without `--force`. That is
exactly the gap [reconcile-pull-research.md](reconcile-pull-research.md)
addresses.

### Config discovery and the legacy CWD pickup

Config discovery is already XDG-first
([src/config.rs:440-462](../../../src/config.rs#L440-L462),
[src/cli/mod.rs:102-138](../../../src/cli/mod.rs#L102-L138)):

| Rank | Source                         | Resolves to                                            |
| ---- | ------------------------------ | ------------------------------------------------------ |
| 1    | `--config <path>`              | absolute or cwd-relative                               |
| 2    | `SKILLNET_CONFIG` env          | absolute or cwd-relative                               |
| 3    | XDG, if present                | `$XDG_CONFIG_HOME/skillnet/skillnet.toml`              |
| 4    | legacy cwd, if present         | `./skillnet.toml`                                      |
| 5    | missing-config error path      | XDG path                                               |

The same precedence applies to `--catalog-config` /
`SKILLNET_CATALOG_CONFIG` (`skillnet.catalog.toml`).

Today the files live at `/data/nvme0/can/Projects/ai-skills/skillnet.toml`
and `/data/nvme0/can/Projects/ai-skills/skillnet.catalog.toml`. They are only
picked up because the user typically runs `skillnet` from inside that
directory (rank 4 in the table). On a fresh shell in any other cwd, the CLI
silently falls through to the XDG path that does not yet exist and fails with
"no such file or directory".

So the *infrastructure* to centralise is already there; the *files* and the
*HM-managed override* are missing.

### Existing HM module surface

[nix/hm-module.nix](../../../nix/hm-module.nix) already exposes:

- `programs.skillnet.settings` — TOML pass-through written to
  `$XDG_CONFIG_HOME/skillnet/skillnet.toml`
  ([hm-module.nix:71-103](../../../nix/hm-module.nix#L71-L103),
  [hm-module.nix:236-240](../../../nix/hm-module.nix#L236-L240)).
- `programs.skillnet.catalogSettings` — TOML pass-through written to
  `$XDG_CONFIG_HOME/skillnet/skillnet.catalog.toml`
  ([hm-module.nix:105-114](../../../nix/hm-module.nix#L105-L114),
  [hm-module.nix:242-246](../../../nix/hm-module.nix#L242-L246)).
- `programs.skillnet.skillsRoot` / `programs.skillnet.mirrorRoot` —
  absolute-path options folded into the generated `skillnet.toml`
  ([hm-module.nix:65-69, 136-140, 22-32](../../../nix/hm-module.nix#L22-L32)).
- Activation script that already calls `skillnet view sync --all --allow-delete`
  and `skillnet project sync --all --allow-delete` on `home-manager switch`
  ([hm-module.nix:311-327](../../../nix/hm-module.nix#L311-L327)).

The schema gap with the user's existing files:

- `skillnet.toml`: the live file uses `views = [{ label, path }, ...]` (no
  `scope` key). The module's example shows `scope = "global"` per view
  ([hm-module.nix:74-86](../../../nix/hm-module.nix#L74-L86)). Both forms parse
  ([src/config.rs:62-75](../../../src/config.rs#L62-L75)) because `scope`
  defaults to `Global`. No translation issue, but the example may mislead
  someone copying it verbatim — it is verbose for the common case.
- `skillnet.catalog.toml`: uses `[settings]` and many `[[rules]]` tables. The
  module declares `catalogSettings` as raw `tomlFormat.type`
  ([hm-module.nix:105-114](../../../nix/hm-module.nix#L105-L114)), so it round-
  trips fine.

What the HM module does *not* yet expose:

- A toggle that flips `skillnet sync` from "fail on non-symlink view" to
  "promote view → canonical, then symlink". The prior dossier names it
  `programs.skillnet.activation.reconcile`
  ([reconcile-pull-research.md:342-345](reconcile-pull-research.md#L342-L345));
  the user's new ask makes that toggle meaningless if promotion becomes the
  always-on default.
- A first-class way to declare "this host owns the canonical store" vs "this
  host only consumes views" — relevant if a future shared-canonical setup
  emerges and you want non-owning hosts to never promote.

### What "single CLI command" already means today

`skillnet sync` is *one* command already. Three things still feel multi-step
from the user's vantage:

1. **Non-symlink resolution requires `--force`.** The user wants this to
   "just work" automatically when promotion is safe.
2. **Status before sync is a separate command** (`skillnet status --all`).
   This is by design (read-only vs write) and the user did not ask to change
   it; flagging only for completeness.
3. **Manual config edits when adding a project.** Already covered by
   `skillnet project add` ([commands/project.rs:22-48](../../../src/commands/project.rs#L22-L48)).
   Out of scope here, but the centralised-config decision affects whether this
   command can write to its config when the config is HM-managed
   (`/nix/store/...` is read-only).

## Evidence Inventory

| Source | What it proves |
|---|---|
| [src/cli/mod.rs:85-91](../../../src/cli/mod.rs#L85-L91) | `skillnet sync` already chains view + project sync, already iterates every configured target via `Config::targets` |
| [src/view.rs:337-359](../../../src/view.rs#L337-L359) | `ensure_symlink` is the single chokepoint that refuses non-symlinks without `--force`; the promotion path slots in here |
| [src/config.rs:440-462](../../../src/config.rs#L440-L462) | XDG path resolution is already implemented; centralisation is a file-move + HM wiring, not a code change |
| [src/config.rs:102-138 via cli/mod.rs](../../../src/cli/mod.rs#L102-L138) | The CWD-legacy rank still exists; removing it would break the user's current invocation pattern until the file is moved |
| [nix/hm-module.nix:71-114, 236-246](../../../nix/hm-module.nix#L71-L114) | `programs.skillnet.settings` and `catalogSettings` are TOML pass-throughs; the schema the user already has is supported as-is |
| [nix/hm-module.nix:198-231](../../../nix/hm-module.nix#L198-L231) | Module assertions enforce absolute paths for `skillsRoot`/`mirrorRoot`/`configFile`/`catalogConfigFile`; no surprise rewrite |
| [nix/hm-module.nix:311-327](../../../nix/hm-module.nix#L311-L327) | HM activation already runs `view sync --all --allow-delete` and `project sync --all --allow-delete` — the entry point that today errors on non-symlinks |
| `cat /data/nvme0/can/Projects/ai-skills/skillnet.toml` | Live `skills_root = mirror_root = /data/nvme0/can/Projects/ai-skills`; 12 configured projects; views use the short `[{ label, path }]` form |
| `cat /data/nvme0/can/Projects/ai-skills/skillnet.catalog.toml` | Catalog rules are all keyed by `path_prefix`/`name`/`project`; they reference paths relative to `skills_root`, so the file is portable to any host that points at the same `skills_root` |
| `ls -la /home/can/.claude/skills/berg-codeberg-ci` | Existing view entries are already symlinks pointing into `ai-skills/global/...`; the promotion path is exercised only when something *else* replaces a symlink with a real directory |
| `git show 15a1352:src/reconcile.rs` (via the existing dossier) | Recovers the pre-0.5.0 staging+rename+manifest writer; the template for the canonical-side write |
| [reconcile-pull-research.md](reconcile-pull-research.md) (full) | Comparator ladder, dirty-state gating extension, doctor wiring, and test fixtures already designed. Avoid redoing this work. |

Commands run for research:

- `git log --oneline -15` and `git show 21f1f83 --stat` to confirm current
  `skillnet sync` lineage.
- `cat <each src module>` and `cat <each config file>` listed in the table
  above.
- `ls -la /home/can/.claude/skills/` to confirm the live view materialisation.

No active database query was needed; the calibration DB is unrelated.

## Existing Plan Status

One related, still-live dossier:

| Plan | Status | What carries forward |
|---|---|---|
| [docs/src/planning/reconcile-pull-research.md](reconcile-pull-research.md) | **active**, never converted into phase docs | Comparator ladder, library primitives, atomic staging pattern, doctor wiring, HM activation toggle pattern, test fixtures. Two of its recommendations need to be *amended* by the user's new constraints (see Work That Should Survive). |

No multi-phase plan set has been generated from that dossier yet. This
dossier is positioned to feed the same downstream `multi-phase-plan-*`
invocation, with the amendments baked in so the planner does not contradict
itself.

The retired `docs/planning/calibration-loop/` and `docs/planning/cli-rebuild/`
sets (deleted in
[21f1f83](https://codeberg.org/caniko/skillnet/commit/21f1f83)) are not
related and can be ignored.

## Work That Should Survive

From [reconcile-pull-research.md](reconcile-pull-research.md), these
constraints still hold and must be respected by any planner that picks this
up:

1. **Per-skill atomicity** ([reconcile-pull-research.md:153-156](reconcile-pull-research.md#L153-L156)).
   The pull side must be atomic per skill, mirroring the existing view
   materialisation contract in [src/view.rs:7-14](../../../src/view.rs#L7-L14).
2. **`copy_dir` preserves mtimes** ([reconcile-pull-research.md:157-160](reconcile-pull-research.md#L157-L160)).
   Reusing it prevents ping-pong on the next sync because both sides end up
   with equal mtimes.
3. **Dirty-destination gate extends to project canonicals**
   ([reconcile-pull-research.md:161-164](reconcile-pull-research.md#L161-L164)).
   `ensure_destination_clean` currently checks only `ctx.mirror_root`; pulling
   into `<project>/.skills` mutates a different repo.
4. **Doctor stays the source of truth for invariants**
   ([reconcile-pull-research.md:165-167](reconcile-pull-research.md#L165-L167)).
   Half-pulled state (canonical updated, symlink not yet re-established) must
   surface as a doctor error.
5. **No `RECONCILIATION.md` manifest revival**
   ([reconcile-pull-research.md:168-171](reconcile-pull-research.md#L168-L171)).
   Telemetry lives in dry-run output, sync summary lines, and (optionally) the
   calibration DB.

From this dossier specifically, these new constraints emerge:

6. **`skillnet sync` stays one command and gains promotion as default.** The
   prior two-tier `view reconcile` / `project reconcile` + `sync --pull`
   recommendation is **withdrawn**. The single-command requirement wins.
7. **An explicit opt-out flag exists for the prior safe behaviour.** Even
   though promotion becomes default, users need an escape hatch when a host
   has no business mutating canonical (consumer-only hosts, CI). Suggested
   flag: `--no-promote` (sym/asym with `--force` is the user's call; my
   recommendation is `--no-promote` because it reads as "skip the new step").
8. **Config files migrate to XDG and become HM-owned.** The legacy
   working-directory pickup stays in the discovery ladder for one release
   (deprecation window), then drops in the same release that ships promotion.
   That keeps the documented migration short.
9. **HM ownership is the default story for users running NixOS or
   Home Manager.** The two files become Nix expressions under
   `programs.skillnet.settings` and `programs.skillnet.catalogSettings`; the
   ai-skills checkout keeps no `*.toml` at its root.

## Blockers And Missing Artifacts

None blocking. Three soft inputs deserve a decision before the planner
fires; they are repeated in **Open Decisions For The User** below with
recommendations.

A note on mtime trust: filesystem mtimes are spoofable
([reconcile-pull-research.md:200-203](reconcile-pull-research.md#L200-L203)).
Promotion-as-default amplifies that risk because no opt-in step exists for
the user to inspect before promotion happens. Mitigation lives in the design
section; flagging here so the user can override if they want a stricter
default.

## Risks And Constraints

- **Architectural reversal, amplified.** The prior dossier flagged that even
  an opt-in reconcile-pull would surprise users of the v0.5.0 "no reconcile"
  stance. Making promotion the *default* of `skillnet sync` magnifies this
  risk by an order of magnitude. Mitigations:
  - Ship under a deliberate version bump to `0.6.0` and a prominent
    CHANGELOG entry.
  - Default `skillnet sync` to **dry-run-on-conflict**: any outcome that is
    not `Identical` or `Created` or `Updated` (clean symlink ops) prints the
    intended promotion and waits for re-invocation with `--apply-promote`.
    Idempotent reruns are then safe; surprise mutation is impossible.
    Counter-recommendation if the user wants "fully automatic": skip this
    and accept the risk explicitly.
  - Surface a per-target log line in the sync summary saying *exactly which
    canonical was overwritten and from where*.
- **HM activation now mutates canonical.** Today,
  `home-manager switch` calls `skillnet view sync --all --allow-delete`
  ([hm-module.nix:324-325](../../../nix/hm-module.nix#L324-L325)). If
  promotion becomes default in `skillnet sync`, then on every HM switch a
  view's stray real-directory entry would silently rewrite canonical. That is
  too quiet for a hands-off activation flow. Mitigations:
  - HM activation invokes `skillnet sync --no-promote` by default; opt-in
    via `programs.skillnet.activation.promote = true|false` (default
    `false`).
  - This inverts the prior dossier's `programs.skillnet.activation.reconcile`
    suggestion: opt-in is now needed to *enable* the default CLI behaviour
    during activation, not to enable a separate subcommand.
- **Config-file location of a mutating CLI vs. a read-only `/nix/store`.**
  `programs.skillnet.settings != null` writes the config into
  `/nix/store/...` (read-only). The mutating commands —
  `skillnet project add` and `skillnet project remove`
  ([commands/project.rs:22-77](../../../src/commands/project.rs#L22-L77)) —
  will hard-fail when run against an HM-managed config. Mitigations:
  - Detect HM-managed configs at command entry; fail with a clear message
    pointing at the right Nix option.
  - Optional, more elegant: split the config into a small "stable HM part"
    (paths, views, NM-owned) and a "mutable cwd part" (per-project add/remove
    bookkeeping) — *do not pursue without a strong reason*; it adds schema
    complexity for limited gain.
- **`skills_root` host-coupling.** The live `skillnet.toml` hard-codes
  `/data/nvme0/can/Projects/ai-skills`. Moving the config to HM means this
  path is now in the Nix expression too. Other hosts adopting the same module
  will need to override `programs.skillnet.skillsRoot`. Already supported
  ([hm-module.nix:136-140](../../../nix/hm-module.nix#L136-L140)); flag for
  the migration doc so users see it.
- **Catalog rule semantics post-promotion.** `skillnet.catalog.toml` rules
  reference paths relative to `skills_root`
  ([skillnet.catalog.toml `path_prefix` lines, e.g. `global/`, `projects/`]).
  Promotion writes into canonical, so any rule status/category change
  triggered by content changes applies on the very next
  `skillnet catalog lint`. This is fine, but document it: promotion can flip
  a skill from `active` to `retired` if the promoted version sets a frontmatter
  status the rules look at.
- **Legacy CWD discovery drop is a breaking change for non-NM users.** The
  user runs `skillnet` from inside `/data/nvme0/can/Projects/ai-skills`
  today. After centralisation, that pickup still works through the
  deprecation window. Once it drops, anyone with the same habit on a fresh
  install will see a confusing "no config" error. Mitigations:
  - Keep the legacy pickup for one full release after centralisation
    (so the release that introduces promotion does *not* simultaneously drop
    legacy CWD discovery).
  - Print a deprecation warning when the CLI falls through to rank 4 of the
    discovery table.
- **Doctor under promotion-default.** A `NonSymlink` entry today is an
  error. Under promotion-default it is a *normal incoming state* that
  `skillnet sync` resolves on next run. Doctor should classify it as
  `Severity::Warn` with the hint "next `skillnet sync` will promote view →
  canonical and re-link" — but only when the view content is newer than
  canonical. View older than canonical stays `Severity::Error` because the
  next sync will silently destroy view-side content (clean symlink replaces
  stale real dir).

## Candidate Next Steps

Sequencing matches the prior dossier's phase letters; this section *amends*
them where the new constraints demand it. Anything not amended carries over
verbatim from
[reconcile-pull-research.md § Candidate Next Steps](reconcile-pull-research.md#L234-L356).

### Phase A — Library primitives

Unchanged from the prior dossier. Implement the `reconcile_view` outcome
enum, the per-entry decision function, and the per-skill atomic write into
canonical. Tests via tempdir + `filetime::set_file_times` to avoid sleep-
based flake.

One new clarification: the outcome enum needs a `WouldPromote` variant
distinct from `Promoted`, so the sync command can choose between
"print and stop" (dry-run-on-conflict default, see Phase C below) and
"actually promote".

### Phase B — Canonical-side write with extended dirty gating

Unchanged from the prior dossier. Generalise `ensure_destination_clean` to
take a target dir, add `ensure_target_clean`, port the `write_skill_set`
staging+rename shape.

### Phase C — CLI surface, **revised**

Replace the prior two-tier proposal with:

1. **`skillnet sync` becomes the single command**, runs `view sync --all`
   then `project sync --all`, and gains a promotion-on-newer behaviour at
   every `ensure_symlink` call site that hits a non-symlink entry.
2. **Default conflict policy: dry-run-on-conflict**. When a view entry is a
   non-symlink, sync prints the proposed promotion (`would promote
   <view-path> → <canonical-path> (view newest_mtime=...,
   canonical newest_mtime=...)`) and **does not mutate canonical**. It still
   performs the harmless clean-symlink work for other entries. The exit code
   is non-zero so wrappers (HM activation, CI) notice.
3. **`--apply-promote`** re-runs and actually performs every
   `WouldPromote` outcome. Combine with `--prefer view|canonical` for
   `EqualMtimeDifferentContent` / `BothAdvanced` tie-breaks. Combine with
   `--adopt-new` for view-only skills.
4. **`--no-promote`** suppresses the new behaviour entirely; sync behaves as
   today (`ensure_symlink` errors on non-symlink). Required for CI and
   consumer-only hosts.
5. **No new `reconcile` subcommand.** `view reconcile` / `project reconcile`
   from the prior dossier are dropped in favour of folding into `sync`.
6. **Status surface gains a `would-promote` count** so `skillnet status
   --format json` rows expose whether a future `sync --apply-promote` would
   change canonical.
7. Honour the existing global `--dry-run` flag (prints would-promote
   diagnostics without exit-code escalation) and
   `--allow-dirty-destination` (per-target, not just `mirror_root`).

### Phase D — Doctor wiring, **revised**

Same intent as the prior dossier but adjusted severities:

- `NonSymlink` with `view newer mtime` → `Severity::Warn`, hint
  `"next 'skillnet sync --apply-promote' will pull view-side edits into
   canonical and re-link"`.
- `NonSymlink` with `canonical newer mtime` or `Identical` → keep
  `Severity::Error`. These remain demolition candidates; promotion does not
  rescue them.
- `EqualMtimeDifferentContent` / `BothAdvanced` → `Severity::Error` with
  hint pointing at `--prefer`.

### Phase E — Config centralisation, **new**

This phase did not exist in the prior dossier. Three sub-steps, in order:

E1. **Add a one-shot migration command.**
`skillnet config migrate` (idempotent):
- Finds the current `skillnet.toml` and `skillnet.catalog.toml` via the same
  precedence the runtime uses, except prefers rank 4 (legacy cwd) when both
  rank 3 (XDG) and rank 4 exist *and* they differ — and refuses, asking the
  user to disambiguate.
- Moves both files to `$XDG_CONFIG_HOME/skillnet/`.
- Leaves a `.skillnet.toml.moved-to-xdg` breadcrumb in the original
  directory pointing at the new location.
- `--dry-run` and `--force` flags. No automatic execution from `sync`.

E2. **Land the actual move.** Run `skillnet config migrate` against the
live `/data/nvme0/can/Projects/ai-skills/skillnet*.toml` files. Update
`ai-skills/README.md` if it references the in-repo paths.

E3. **Drop legacy CWD discovery in `0.7.0` (not `0.6.0`)**. Promotion lands
in `0.6.0`; CWD discovery removal lands in `0.7.0` to give downstreams a
release to migrate. Print a deprecation warning whenever rank 4 fires during
the `0.6.x` series.

### Phase F — HM module surface, **new and revised**

Three things:

F1. **Translate the live files into a Nix expression.** Produce a
`programs.skillnet.settings = { ... }` and `programs.skillnet.catalogSettings
= { ... }` block in the canix repo (or wherever the user's HM config lives —
the user-level HM configuration). Document this as the recommended adoption
path in `docs/src/quickstart.md`.

F2. **Add `programs.skillnet.activation.promote`** (default `false`). When
`true`, HM activation runs `skillnet sync --apply-promote`. When `false` (the
default), activation runs `skillnet sync --no-promote --allow-delete` so the
read-only path stays loud-on-conflict but never mutates canonical from a
nightly switch.

F3. **Surface `programs.skillnet.activation.failOnConflict`** (default
`true`). Today the activation script already uses `|| true` on the
project-sync step
([hm-module.nix:325](../../../nix/hm-module.nix#L325)), which masks failures.
Tie this to the new default exit-code-on-conflict behaviour so users opt into
either "loud activation that can wedge home-manager switch" or "quiet
activation that needs `skillnet doctor` for visibility".

### Phase G — Docs, tests, release

Mostly carries over from the prior dossier's Phase E. New requirements:

- `docs/src/commands.md` § `sync` is rewritten end-to-end to describe the
  new default, `--apply-promote`, `--no-promote`, `--prefer`, `--adopt-new`.
- A migration section in `docs/src/migration/` (new file
  `centralised-config.md`) covers Phase E.
- The HM module quickstart shows the recommended pattern.
- The single-command claim is enshrined in a CLI snapshot test that asserts
  `skillnet --help` short-help lists `sync` as the canonical entry point and
  does **not** mention any reconcile subcommand.

### Sequencing

A → B → C → D in one cohesive phase set (library + CLI + doctor).
E and F are independent of A-D and can run in parallel. G ships alongside.

Recommended fan-out for a `multi-phase-plan-claude` invocation: three
parallel sub-layers in Phase 1 — (A+B+C+D as one sub-layer because they share
state), (E as a second sub-layer because it is pure migration/CLI add), (F as
a third sub-layer because it is HM-module-only and never touches Rust). One
shared verification phase at the end that runs `skillnet sync --dry-run`
against a fixture host and `nix flake check` against the canix repo.

## Resolved Decisions

Each of the six open questions is closed; the concrete specs in
[Final Design](#final-design) below assume these answers.

| # | Decision | Choice | Why |
|---|---|---|---|
| 1 | Default behaviour of `skillnet sync` on non-symlink view | **Dry-run-on-conflict**: print the proposed promotion, do not mutate canonical, exit `2`. Mutation requires explicit `--apply-promote` | Idempotent reruns are safe; HM activation cannot silently rewrite canonical; mtime-spoof has no force-multiplier effect |
| 2 | Adoption policy for unknown view skills | Never adopt without `--adopt-new`. `Stale` semantics on view-only entries unchanged | Foreign content stays foreign; promotion path stays explicit |
| 3 | Migration command vs. manual `mv` | Ship `skillnet config migrate` | Small, idempotent, handles the both-locations-present-and-different edge case the user will hit at least once |
| 4 | HM activation default | `programs.skillnet.activation.promote = false`, `programs.skillnet.activation.failOnConflict = true` | Multi-host safe; mutation only on hosts that explicitly opt in; loud activation surfaces drift instead of masking it like today's `|| true` |
| 5 | Per-target `--allow-dirty-destination` | Extend the existing global flag to gate every canonical write (mirror + per-project repo) | Smallest surface; no concrete need yet for per-scope override |
| 6 | Legacy CWD config discovery sunset | Deprecation warning in `0.6.0`, drop in `0.7.0` | Two-release window matches the cadence of every other breaking change in this CHANGELOG |

## Final Design

### 1. `skillnet sync` flag surface

Single command, no subcommands added. Existing flags kept; new flags added.

| Flag | Default | Behaviour |
|---|---|---|
| `--apply-promote` | off | Executes pending `ViewNewer` outcomes and any `--prefer`-resolved or `--adopt-new`-promoted outcomes. Without this flag, those outcomes are reported as `WouldPromote`/`WouldAdopt` only |
| `--no-promote` | off | Hard-disables the promotion path entirely. Non-symlink view entries error as in `0.5.x` unless `--force` is also passed. Required for CI and consumer-only hosts |
| `--force` | off | Demote `CanonicalNewer` entries (destroys view-side content). Same name and shape as today; semantics narrowed to the destructive demote-only branch |
| `--prefer <view\|canonical>` | unset | Tie-breaker for `EqualMtimeDifferentContent` and `BothAdvanced`. Only consulted when `--apply-promote` is also passed |
| `--adopt-new` | off | Treat `AdoptCandidate` outcomes as promotion candidates. Only acts when `--apply-promote` is also passed |
| `--allow-delete` | off | Existing semantics. Prunes view entries with no canonical sibling and no `--adopt-new` |
| `--dry-run` (global) | off | Never mutates, never exit-2-escalates. Prints would-* lines and exits `0` |
| `--allow-dirty-destination` (global) | off | Now gates every canonical write site, not just `mirror_root`. See §5 |

Mutually exclusive combinations rejected at parse time:

- `--apply-promote` with `--no-promote`.
- `--force` with `--no-promote` (deliberate: `--no-promote` requires the
  user to pass `--force` *without* `--no-promote` to demote, matching today's
  behaviour and making the destructive path explicit). *Reconsider only if a
  CI use-case for "demote-only, never promote, never error" emerges.*

### 2. Comparator outcomes and action matrix

```rust
pub enum ReconcileOutcome {
    Identical,
    ViewNewer            { view_mtime: u128, canonical_mtime: u128 },
    CanonicalNewer       { view_mtime: u128, canonical_mtime: u128 },
    EqualMtimeDifferentContent { view_sha: String, canonical_sha: String, mtime: u128 },
    BothAdvanced         { view_only: Vec<Utf8PathBuf>, canonical_only: Vec<Utf8PathBuf> },
    AdoptCandidate,
}
```

Per-entry action matrix:

| Outcome | Default action | With `--apply-promote` | With `--force` | With `--no-promote` |
|---|---|---|---|---|
| `Identical` | auto-demote to symlink | same | same | same |
| `ViewNewer` | report `WouldPromote`, no mutation, escalate exit | promote view → canonical (atomic stage+rename), then demote | error: "view newer; pass `--apply-promote`" | error: "view non-symlink; pass `--force` to destroy view-side edits" |
| `CanonicalNewer` | report `WouldDemoteDestructive`, no mutation, escalate exit | error: "canonical newer; `--force` required to discard view content" | demote (destroys view content) | error as today |
| `EqualMtimeDifferentContent` | report `NeedsPreferenceTieBreak`, escalate exit | requires `--prefer view\|canonical`; otherwise error | n/a | error |
| `BothAdvanced` | same as above | requires `--prefer view\|canonical`; promote-or-discard whole skill (no per-file merge) | n/a | error |
| `AdoptCandidate` | no-op (stale entry visible in status) | requires `--adopt-new`; promote into canonical | n/a | no-op |

Per-skill atomicity is preserved. Three-way file-level merge is explicitly
not done.

### 3. Exit codes

| Code | Meaning |
|---|---|
| `0` | All outcomes were `Created`, `Updated`, `Unchanged`, `Removed`, `Identical`, auto-demoted, or `AdoptCandidate` (visible-but-not-promoted) |
| `2` | At least one `WouldPromote`, `WouldDemoteDestructive`, or `NeedsPreferenceTieBreak` was reported and not actioned |
| `1` | Any other error (IO, parse, dirty destination, parse-time flag conflict, etc.) |

`--dry-run` collapses code `2` to code `0` because it is explicitly a
preview, never a gate.

### 4. Status JSON additions

`skillnet status --format json` rows gain two fields:

```json
{
  "scope": "global",
  "kind": "global",
  "canonical_path": "/data/nvme0/can/Projects/ai-skills/global",
  "skill_count": 42,
  "drift_entries": 0,
  "would_promote": 0,
  "needs_tie_break": 0
}
```

`drift_entries` keeps its existing meaning (`Missing`, `WrongTarget`,
`NonSymlink`-with-no-mtime-comparison, `Stale`). The new fields are derived
by classifying each `NonSymlink` entry through the comparator above.

The `DriftEntry` JSON shape gains optional `view_mtime_nanos`,
`canonical_mtime_nanos`, `view_sha`, `canonical_sha`, populated only for the
new `NonSymlink` sub-classifications. Existing consumers see the new fields
as `null` and ignore them.

### 5. Per-target dirty-destination gate

`Context::ensure_destination_clean` is generalised:

```rust
impl Context {
    pub fn ensure_target_clean(&self, target_path: &Utf8Path) -> Result<()>;
}
```

The existing call site that passes `ctx.mirror_root` is replaced with one
call per write target. Every canonical write — mirror-side or per-project —
flows through this gate. The `--allow-dirty-destination` flag bypasses
*every* such gate; this is the same flag, expanded in scope. Mirror-only
gating remains the default; per-project gating is added.

When the target is not inside a git working tree (`vcs::ensure_clean`
returns `Ok(())` today for non-git paths), the gate is a no-op. No new
config knob needed.

### 6. `skillnet config migrate`

New subcommand, idempotent, no side effects beyond moving the two files
plus writing one breadcrumb each.

```
skillnet config migrate
    [--dry-run]
    [--force]                # overwrite XDG when both locations exist and differ
    [--remove-breadcrumbs]   # one-shot: delete previously written breadcrumbs
```

Per-file decision table (run independently for `skillnet.toml` and
`skillnet.catalog.toml`):

| Legacy cwd file | XDG file | Default action | Notes |
|---|---|---|---|
| absent | absent | no-op | exit code `0`, prints "no config to migrate" |
| absent | present | no-op | prints "already centralised at <path>" |
| present | absent | move cwd → XDG, write breadcrumb `<cwd>/.skillnet.toml.moved-to-xdg` containing the XDG path | `mkdir -p $XDG_CONFIG_HOME/skillnet` if missing |
| present | present, content equal | delete cwd, write breadcrumb | sha256 compare; cheap |
| present | present, content different | refuse with diff hint, exit `1` | `--force` overwrites XDG with cwd content |

Breadcrumb file contents are exactly the absolute destination path plus a
trailing newline. The file is plain text; no TOML, no JSON. Easy to
`cat` and recoverable if the user removes it manually.

`--dry-run` prints the per-file decision without touching the filesystem.
`--remove-breadcrumbs` deletes any `.skillnet.toml.moved-to-xdg` and
`.skillnet.catalog.toml.moved-to-xdg` files it finds at the rank-4
discovery location (cwd of invocation only — does not traverse).

Discovery uses the same path resolution as the runtime so the user can run
`skillnet config migrate` from the same cwd they previously ran `skillnet`
from and have it find both files.

### 7. HM-managed config detection

CLI mutating commands (`project add`, `project remove`, and any future
write to the config file) check whether the resolved config path is under
the Nix store before attempting a write:

```rust
fn config_is_hm_managed(path: &Utf8Path) -> bool {
    path.starts_with("/nix/store/")
}
```

On `true`, the command exits with code `1` and prints:

```
error: skillnet.toml at <path> is managed by Home Manager (read-only).
hint: edit programs.skillnet.settings in your Home Manager configuration,
      then run `home-manager switch`.
```

Symmetric error for `skillnet.catalog.toml`. No fallback "write somewhere
else" behaviour; the user must choose declarative or mutable.

This check fires only on commands that write the config. Read commands
(every other command) are unaffected.

### 8. HM module additions

Three new options on `programs.skillnet`:

```nix
activation.promote = lib.mkOption {
  type = lib.types.bool;
  default = false;
  description = ''
    Whether `home-manager switch` runs `skillnet sync --apply-promote`
    or `skillnet sync --no-promote`. Set to true only on the host that
    owns the canonical skill store; consumer-only hosts must leave it
    false to avoid silently mutating canonical from a routine switch.
  '';
};

activation.failOnConflict = lib.mkOption {
  type = lib.types.bool;
  default = true;
  description = ''
    Whether a non-zero exit from `skillnet sync` during activation
    fails the `home-manager switch`. Default true surfaces drift loudly;
    set false to restore the pre-0.6.0 silent-on-conflict behaviour.
  '';
};

activation.allowDelete = lib.mkOption {
  type = lib.types.bool;
  default = true;
  description = ''
    Whether activation passes --allow-delete. Existing default; broken
    out so a consumer-only host can disable it without rewriting the
    activation script.
  '';
};
```

The activation script ([hm-module.nix:311-327](../../../nix/hm-module.nix#L311-L327))
is rewritten to derive its flags from those options:

```nix
home.activation.skillnet-views = lib.hm.dag.entryAfter ["writeBoundary" "skillnet-skills-root"] ''
  if [ -z "''${SKILLNET_MIRROR_ROOT-}" ]; then
    mirror=${lib.escapeShellArg (if cfg.mirrorRoot != null then cfg.mirrorRoot else "")}
  else
    mirror="$SKILLNET_MIRROR_ROOT"
  fi
  if [ -z "$mirror" ] || [ ! -d "$mirror/global" ]; then
    echo "WARNING: skillnet: mirror not found at $mirror; skipping sync" >&2
  else
    $DRY_RUN_CMD ${cfg.package}/bin/skillnet sync \
      ${lib.optionalString cfg.activation.promote "--apply-promote"} \
      ${lib.optionalString (!cfg.activation.promote) "--no-promote"} \
      ${lib.optionalString cfg.activation.allowDelete "--allow-delete"} \
      ${lib.optionalString (!cfg.activation.failOnConflict) "|| true"}
  fi
'';
```

The `view sync` + `project sync` pair collapses to one `skillnet sync` call
because the top-level command already chains both. The `|| true` is
conditional, defaulting off.

No new option is added for the catalog or the migration command — those
flow through the existing `programs.skillnet.settings` / `catalogSettings`
TOML pass-throughs unchanged.

### 9. Doctor severity matrix

`skillnet doctor` ([commands/doctor.rs](../../../src/commands/doctor.rs))
classifies each `NonSymlink` entry by its comparator outcome and reports:

| Sub-outcome | Severity | Hint |
|---|---|---|
| `Identical` | Info | "`skillnet sync` will silently demote to symlink" |
| `ViewNewer` | Warn | "`skillnet sync --apply-promote` will pull view → canonical and re-link" |
| `CanonicalNewer` | Error | "`skillnet sync --force` will destroy view-side edits; review before running" |
| `EqualMtimeDifferentContent` | Error | "`skillnet sync --apply-promote --prefer view\|canonical` required" |
| `BothAdvanced` | Error | "`skillnet sync --apply-promote --prefer view\|canonical` required; per-file merge is not supported" |
| `AdoptCandidate` | Info | "view-only skill; `skillnet sync --apply-promote --adopt-new` to promote" |

Doctor's overall exit code stays as today: `0` if all entries are `Info`,
non-zero if any `Warn`/`Error` row is present. `Info` is purely
informational.

### 10. mtime-spoof mitigation

The dry-run-on-conflict default removes the silent-mutation amplifier.
Concrete mitigations layered on top:

- Every `WouldPromote` log line includes `view_mtime`, `canonical_mtime`,
  and `view_sha[..8]` so the user reviewing the proposal can spot a
  suspiciously-future mtime.
- `skillnet doctor` is the recommended pre-sync inspection step; the
  quickstart and migration docs both name it explicitly in the
  centralisation flow.
- The CLI never honours `view_mtime > now()` as the deciding factor on
  its own: if `view_mtime` is in the future relative to wall-clock at
  invocation, the comparator downgrades the outcome to
  `NeedsPreferenceTieBreak` so the user must pass `--prefer view` to
  proceed. (Cheap to implement; bounds the attack to "spoofer must also
  win an explicit tie-break".)

### 11. Catalog rule fallout from promotion

`skillnet.catalog.toml` rules apply to canonical paths
([src/catalog/](../../../src/catalog/)). Promotion writes into canonical, so
on the very next `skillnet catalog generate` or `skillnet catalog lint`,
rule status (`active`, `retired`, `reference`) can change if the promoted
skill's frontmatter sets a status the rules read.

No code change needed. Document this in the new
`docs/src/migration/centralised-config.md` so users see the implication.

### 12. Migration and release sequencing

| Version | Ships |
|---|---|
| `0.6.0` | All twelve sections above, including `skillnet config migrate`. Legacy cwd config discovery still works but prints a deprecation warning every invocation that falls through to rank 4. `programs.skillnet.activation.promote` defaults to `false` so HM hosts upgrading to `0.6.0` see no behaviour change |
| `0.7.0` | Rank-4 legacy cwd discovery is removed. `skillnet config migrate` is kept (idempotent no-op once XDG is populated). Single CHANGELOG line plus a one-paragraph note in `docs/src/migration/centralised-config.md` |

No `0.5.x` patch release. The migration command is small enough to ship
inside the `0.6.0` cut and gives users a one-shot path that will not break.

### 13. Tests required (acceptance criteria for the planner)

Listed at the level of "would I accept this PR" — the planner is free to
group these into phases as it likes.

- `tests/view_sync.rs` gains one fixture per comparator outcome, using
  `filetime::set_file_times` to control mtimes. Each fixture covers both
  the dry-run-on-conflict default and the `--apply-promote` path.
- `tests/cli.rs` adds a snapshot test pinning `skillnet sync --help` so the
  flag table cannot drift silently.
- `tests/cli.rs` adds default-shape tests analogous to
  `sync_command_defaults_to_safe_flags` for the new flags
  (`--apply-promote`, `--no-promote`, `--prefer`, `--adopt-new`,
  `--force` semantics narrowing).
- `tests/cli.rs` asserts the mutually-exclusive combinations reject at
  parse time with the documented error messages.
- A new `tests/config_migrate.rs` covers the six rows of the migration
  decision table plus `--dry-run` and `--force`.
- A new `tests/dirty_gate.rs` covers the per-target gate by initialising
  a project repo as a separate `git init` inside a tempdir and asserting
  promotion refuses without `--allow-dirty-destination`.
- `nix/test-hm-module.nix` (existing harness) gains assertions that
  `programs.skillnet.activation.promote = true|false` and
  `failOnConflict = true|false` produce the correct activation script
  flags. One snapshot test per combination.

### 14. Out of scope, deliberately

- **Per-file three-way merge.** Skills stay per-skill atomic.
  `BothAdvanced` always asks the user; never auto-merges.
- **Adoption from a view into a *different* project's canonical.** Promotion
  always targets the view's own canonical (global view → global canonical,
  project view → that project's canonical). Cross-scope promotion is not
  modelled.
- **Splitting the config into a stable HM part and a mutable cwd part.**
  Flagged as a future option but explicitly not in this scope.
- **Reviving `RECONCILIATION.md` or any other manifest.** Telemetry stays
  in sync summary lines, dry-run output, and (optionally, untouched here)
  the calibration DB.
- **Auto-commit of canonical after promotion.** The v0.5.0 stance "commit
  canonical store changes with Git directly"
  ([docs/src/commands.md:113-114](../commands.md#L113-L114)) holds.
  Promotion writes; the user commits.

---

The design is locked. Hand to a `multi-phase-plan-*` skill with this dossier
as the planner input; no further design questions should arise during
fan-out.
