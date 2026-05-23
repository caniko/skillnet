# Calibration loop for `multi-phase-plan`

> **Recommended Codex model for orchestration: GPT 5.5 high**
>
> This plan now spans two repositories: it stands up `skillnet` as a
> standalone public Rust crate on Codeberg (with crates.io publish,
> Codeberg CI, and a Nix Home Manager module), then wires the
> `ai-skills` repo's `multi-phase-plan` skill to *consume* the
> published crate via the HM module. The orchestration decisions are
> the cross-repo seam (where calibration data lives, how the skill
> finds the binary, how versioning flows) and the heuristics rewrite
> in `ai-skills`. Output volume isn't the bottleneck; design judgment
> across the seam is.

## Scope

Two things land together, across two repositories:

1. **A new public crate** at `ssh://git@codeberg.org/caniko/skillnet.git`
   (local path: `/data/nvme0/can/Projects/skillnet`):
   - A SQLite-backed calibration data subsystem with sidecar
     (`.calibration.json`) ingest, structured tag conventions, and
     meta-heuristic-driven sampling that minimizes selection bias.
   - A `skillnet calibration …` CLI surface: `record`, `verify`,
     `tag`, `untag`, `show`, `query`, `migrate`, `vacuum`, `export`,
     `analyze`, `propose`, `proposals`, `decide`, `export-changelog`.
   - Crates.io publication readiness (Cargo metadata, LICENSE,
     README, rustdoc, docs.rs cfgs).
   - Codeberg/Forgejo CI on the self-hosted atlas runner (fmt,
     clippy, test, doc, package, publish).
   - A Nix Home Manager module that lets users declaratively install
     and configure skillnet (`programs.skillnet.enable = true`).
2. **Heuristics + hook rewrite in `ai-skills`** at
   `/data/nvme0/can/Projects/ai-skills`:
   - Drop the 3–8 phase cap in `multi-phase-plan`; replace with
     **per-phase shape rules** and a **trigger-driven heuristics
     catalog** (explicit thresholds, four categories).
   - Define the `.calibration.json` sidecar schema and the verifier
     `surprises` text convention.
   - Add end-of-plan and end-of-verify hooks that shell out to the
     installed `skillnet calibration record|verify`.
   - Add a `calibrate` mode (sister to `plan` and `verify`) that
     walks the user through analysis → proposal → decision →
     changelog export.
   - Propagate the three modes to the flavor wrappers
     (`-codex`, `-claude`, `-mixed`); update the ai-skills flake to
     consume the published `skillnet` via the HM module.

## Current state (both repos)

- **`/data/nvme0/can/Projects/skillnet` does not exist yet.** The
  Rust sources currently live under `ai-skills/src/`. Phase 07 moves
  them; Phases 01–04 are written against the *target* layout.
  Phase 01's working tree should be reinterpreted as the new repo —
  if Phase 01 is executed before the move, the resulting code can
  be lifted into the new repo by Phase 07 with minor path edits.
  (The user instructed phases 01 and 02 stay as-written; their
  working-tree path is the only stale field.)
- **`ai-skills` `multi-phase-plan`** still enforces the 3–8 cap; no
  hooks; no `calibrate` mode; no flake input for an external
  skillnet crate.
- **No calibration data is collected**; threshold guesses don't
  improve.
- **`skillnet` is not packaged as a public crate**; not on
  crates.io; no Codeberg CI; no HM module.

## Architecture after this plan

```
codeberg.org/caniko/skillnet   (public crate, GPL? MIT? choose in 07)
├── src/                      ← the CLI (moved from ai-skills/src/)
├── data/multi-phase-plan/    ← bundled schema migrations (read-only at runtime)
├── nix/
│   └── hm-module.nix         ← Home Manager module
├── flake.nix                 ← packages.skillnet, apps.skillnet, hmModules.default
├── .forgejo/workflows/       ← Codeberg CI (atlas runner)
└── Cargo.toml                ← crates.io metadata

User's HM config:
  programs.skillnet = {
    enable = true;
    dataDir = "~/.local/share/skillnet";   # or per-skill subdirs
  };

ai-skills (this repo)
├── global/multi-phase-plan/SKILL.md       ← heuristics catalog + hooks + calibrate mode
├── global/multi-phase-plan-{codex,claude,mixed}/SKILL.md ← mode propagation
└── flake.nix                              ← inputs.skillnet follows codeberg.org/caniko/skillnet
```

Calibration data is **runtime data**, owned by the user, not by the
repo. The `~/.local/share/skillnet/multi-phase-plan/calibration.sqlite`
file lives on the user's machine; the schema (`data/multi-phase-plan/schema/`)
is bundled with the crate and applied at first run. The calibration
*changelog* (the audit trail of threshold edits) lives in
`ai-skills/global/multi-phase-plan/SKILL.md`, since it's the artifact
the skill body is calibrating.

## Phase table

| Phase | File | Repo | Depends on | Touches | Can parallel with |
|---|---|---|---|---|---|
| 01 | [01-sqlite-schema-storage.md](./01-sqlite-schema-storage.md) | skillnet | — | `Cargo.toml`, `src/calibration/{mod,db}.rs`, `data/multi-phase-plan/schema/` | 05 |
| 02 | [02-cli-record-verify.md](./02-cli-record-verify.md) | skillnet | 01 | `src/calibration/{sidecar,record}.rs`, `src/cli/args.rs`, `src/commands/calibration.rs` | 05 |
| 03 | [03-cli-inspect-tag-housekeeping.md](./03-cli-inspect-tag-housekeeping.md) | skillnet | 02 | `src/calibration/{tag,query,housekeeping}.rs`, `src/cli/args.rs`, `src/commands/calibration.rs` | 04, 05 |
| 04 | [04-cli-analyze-propose-decide.md](./04-cli-analyze-propose-decide.md) | skillnet | 02 | `src/calibration/{analyze,propose,decide,changelog}.rs`, `src/cli/args.rs`, `src/commands/calibration.rs` | 03, 05 |
| 05 | [05-base-skill-heuristics-rewrite.md](./05-base-skill-heuristics-rewrite.md) | ai-skills | — | `global/multi-phase-plan/SKILL.md` | 01, 02, 03, 04, 07 |
| 06 | [06-base-skill-hooks-calibrate-mode.md](./06-base-skill-hooks-calibrate-mode.md) | ai-skills | 04, 05, 07 | `global/multi-phase-plan/SKILL.md` | 08 |
| 07 | [07-skillnet-crate-publication.md](./07-skillnet-crate-publication.md) | skillnet | 04 | `Cargo.toml`, `README.md`, `LICENSE`, `.forgejo/workflows/`, `flake.nix` | 05 |
| 08 | [08-nix-hm-module.md](./08-nix-hm-module.md) | skillnet | 07 | `nix/hm-module.nix`, `flake.nix` | 06 |
| 09 | [09-ai-skills-consumption.md](./09-ai-skills-consumption.md) | ai-skills | 06, 08 | `global/multi-phase-plan-{codex,claude,mixed}/SKILL.md`, `flake.nix` | — |

## Parallelism layer

**Wave 0** (start from current tree):
- Phase 01 — SQLite foundation, in the (about-to-exist) skillnet repo.
- Phase 05 — Base SKILL.md heuristics rewrite in ai-skills. Pure
  docs; no code dependency.

**Wave 1** (after 01):
- Phase 02 — Record/verify CLI in skillnet.

**Wave 2** (after 02):
- Phase 03 — Inspect/tag/housekeeping CLI.
- Phase 04 — Analyze/propose/decide CLI.
- 03 and 04 run in parallel; both touch `src/cli/args.rs` and
  `src/commands/calibration.rs` — see **Serialization points**.

**Wave 3** (after 04):
- Phase 07 — Crate publication readiness (Cargo metadata, README,
  LICENSE, Codeberg CI, crates.io publish workflow). Triggers the
  first published version of `skillnet`.

**Wave 4** (after 07):
- Phase 08 — Nix HM module (`programs.skillnet.enable`) and flake
  outputs.
- Phase 06 — ai-skills hooks + `calibrate` mode. Can start as soon
  as Phase 07 has published a crate the hooks can reference; doesn't
  need 08 strictly but its prose is cleaner if 08 is in flight.

**Wave 5** (after 06 + 08):
- Phase 09 — ai-skills flavor propagation + flake consumption of
  the published `skillnet` via the HM module.

## Serialization points

| File | Phases | Order | Recovery if conflict |
|---|---|---|---|
| `skillnet/src/cli/args.rs` | 02 → 03, 04 | 02 first; 03 and 04 each rebase, add their own `Calibration*` variants at the placeholder | trivial three-way merge |
| `skillnet/src/commands/calibration.rs` | 02 → 03, 04 | same | match-arm order conflicts only |
| `skillnet/Cargo.toml` | 01, 07 | 01 adds deps; 07 fills metadata | sectioned file, low conflict risk |
| `skillnet/flake.nix` | 07, 08 | 07 adds `packages.skillnet` + `apps.skillnet`; 08 adds `hmModules.default` | additive |
| `ai-skills/global/multi-phase-plan/SKILL.md` | 05 → 06 | 05 rewrites the body; 06 inserts hook + calibrate mode sections | reread 06's Plan against the post-05 file |
| `ai-skills/global/multi-phase-plan-{codex,claude,mixed}/SKILL.md` | 09 only | n/a | — |
| `ai-skills/flake.nix` | 09 only | n/a | — |

## Shared-file lockstep

The skill catalog being built by Phase 05 includes a
**shared-file-contention** heuristic; this plan's own files trigger it
twice — once in skillnet (`src/cli/args.rs`, `src/commands/calibration.rs`,
touched by phases 02, 03, 04) and once in ai-skills
(`global/multi-phase-plan/SKILL.md`, touched by phases 05, 06). The
Plan sections of each affected phase cross-link the others so the
agent doesn't discover the conflict mid-execution.

## Infrastructure SPOF

Two infrastructure phases:

- **Phase 01** is the SQLite SPOF for the entire CLI: a wrong
  migration runner or a wrong schema invalidates 02–04's tests and
  06's hook contract.
- **Phase 07** is the *publication* SPOF: a botched crates.io
  publish, broken Cargo metadata, or a non-functional Codeberg CI
  blocks 08 (HM module needs the crate to consume), 09 (ai-skills
  needs the HM module), and effectively the entire `ai-skills`
  side of the plan.

Both phases' acceptance criteria include round-trip smoke tests
(open a fresh db / install the published crate fresh) to catch
SPOF failures early.

## Serial-chain recovery

The critical-path chain is **01 → 02 → 04 → 07 → 08 → 09** (depth
6). Each link compounds recovery cost: a regression in 02
invalidates 04's fixtures, 07's publish surface, 08's HM module
config, and 09's consumption. Mitigation: every phase from 02
onward includes a smoke command in its Acceptance criteria that
exercises the prior phase's surface. If smoke fails downstream,
treat the upstream phase as not-actually-accepted and re-verify.

## External-repo coordination

Two repos, two push targets:

| Repo | Path | Push target | Phases that push |
|---|---|---|---|
| skillnet | `/data/nvme0/can/Projects/skillnet` | `ssh://git@codeberg.org/caniko/skillnet.git` | 01, 02, 03, 04, 07, 08 |
| ai-skills | `/data/nvme0/can/Projects/ai-skills` | (existing remote) | 05, 06, 09 |

Each phase's "Working tree" section names the absolute path it
operates in. Cross-repo phases (none in this set — every phase
operates in exactly one repo) would need explicit branch
synchronization; we avoid them by design.

## Whole-set acceptance criteria

- [ ] `skillnet` is a published crate on crates.io with version
      ≥0.1.0; `cargo install skillnet` succeeds on a clean machine.
- [ ] `ssh://git@codeberg.org/caniko/skillnet.git` is the canonical
      source-of-truth repo; Codeberg CI is green on `main`.
- [ ] `skillnet calibration --help` lists all 14 subcommands.
- [ ] On a fresh system, adding `programs.skillnet.enable = true;`
      to a HM config and rebuilding installs `skillnet` on PATH and
      creates `~/.local/share/skillnet/` (or the configured dataDir).
- [ ] A round-trip test exists: write a synthetic
      `.calibration.json`, `skillnet calibration record`, `verify`,
      `analyze`, observing expected fire/signal counts.
- [ ] `ai-skills/global/multi-phase-plan/SKILL.md` contains the
      heuristics catalog, meta-heuristics, sidecar schema, tag
      conventions, hooks, `calibrate` mode, and an initially-empty
      calibration changelog footer.
- [ ] The three flavor skills document the `calibrate` mode and
      inherit the hooks.
- [ ] `ai-skills/flake.nix` has an input pointing at
      `codeberg.org/caniko/skillnet` (locked to a specific
      revision) and consumes the HM module.
- [ ] No regressions in either repo: `cargo test`, `cargo clippy
      --all-targets -- -D warnings`, `cargo fmt --check`,
      `nix flake check` clean in skillnet;
      `nix flake check` clean in ai-skills.

## Global constraints

- The skill (`ai-skills/global/multi-phase-plan/SKILL.md`) never
  writes SQLite directly. All persistence goes through the
  published `skillnet` CLI.
- Storage location is per-user, not per-repo:
  `$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`
  (defaulting to `~/.local/share/...`). Configurable via the HM
  module's `dataDir` option.
- Schema migrations ship inside the crate
  (`data/multi-phase-plan/schema/NNN-*.sql` embedded via
  `include_str!`) so the binary is self-contained.
- Sidecar format is JSON at `<plan-dir>/.calibration.json`.
- One database per skill (current plan ships
  `multi-phase-plan`; future skills get
  `~/.local/share/skillnet/<skill>/calibration.sqlite`).
- The calibration changelog lives in ai-skills (the artifact being
  calibrated); the raw dataset lives on the user's machine.
- Skillnet crate is `MIT` *or* `Apache-2.0` (pick in Phase 07; default
  to dual `MIT OR Apache-2.0` per Rust ecosystem norm).
- Calibrate mode is a sister mode in the base skill, not a separate
  skill.

## Locked design defaults

| Decision | Value |
|---|---|
| New crate name | `skillnet` |
| New crate repo | `ssh://git@codeberg.org/caniko/skillnet.git` |
| New crate local path | `/data/nvme0/can/Projects/skillnet` |
| Storage path (runtime) | `$XDG_DATA_HOME/skillnet/<skill>/calibration.sqlite` |
| Sidecar format | JSON (`.calibration.json`) |
| CLI command name | `skillnet calibration <verb>` (full word) |
| DB scope | one per skill |
| Calibrate placement | sister mode in base skill (`ai-skills`) |
| Random sample rate | 7% (anti-bias floor) |
| Min N before threshold delta | 10 fires |
| Min N per-tag-band | 30 fires |
| Crate license | `MIT OR Apache-2.0` (dual) |
| MSRV | match current edition; document in Cargo.toml |
| HM module path | `nix/hm-module.nix` exposed as `hmModules.default` |
| Codeberg runner | self-hosted atlas (single label) |
| Schema versioning | `data/multi-phase-plan/schema/NNN-<desc>.sql` + `schema_versions` table, embedded at compile time |

## Routing summary

| Phase | Repo | Layout | Sub-layers | Model | Blocking? |
|---|---|---|---|---|---|
| 01 | skillnet | flat | — | 5.5 medium | yes (foundation for 02–04) |
| 02 | skillnet | flat | — | 5.5 medium | yes (gates 03, 04, 06) |
| 03 | skillnet | flat | — | 5.5 low | no (parallel with 04) |
| 04 | skillnet | flat | — | 5.5 high | yes (gates 07) |
| 05 | ai-skills | flat | — | 5.5 high | no (parallel) |
| 06 | ai-skills | flat | — | 5.5 medium | yes (gates 09) |
| 07 | skillnet | flat | — | 5.5 high | yes (gates 08, blocks first publish) |
| 08 | skillnet | flat | — | 5.5 medium | yes (gates 09) |
| 09 | ai-skills | flat | — | 5.5 medium | no (final) |

No phase routes to `max`. Phase 07 (crate publication, including
crates.io publish — a non-reversible action) is the closest to
needing `max`; it sits at `high` because the work is mostly
mechanical when guided by the `rust-crate-release-prep` family of
skills the repo already provides. If the publish bricks, recovery
is "yank + publish 0.1.1" — painful but bounded.

## Skills available to leverage

This plan should ride on existing skills rather than reinventing:

- **`rust-crate-release-prep`**, **`rust-crate-release-chaperone`**,
  **`rust-crate-publish-workflow`** — Phase 07 (crate publication).
- **`rust-crate-forgejo-release-ci`**, **`forgejo-atlas-ci`** —
  Phase 07 (Codeberg CI on self-hosted atlas).
- **`rust-crate-manifest-metadata`**, **`rust-crate-legal-readme`**,
  **`rust-crate-rustdoc`**, **`rust-crate-quality-gates`** — Phase 07
  (Cargo.toml, README, rustdoc, lint gates).
- **`rust-project-flake`** — Phase 07 (Nix flake for the new crate).
- **`mvp2prod`** — Phase 07 (bootstrap LICENSE/CHANGELOG/etc. for a
  brand-new public repo).
- **`simit-project-init`** — Phase 07 (if simit is the preferred
  release infrastructure in this organization, run it instead of
  hand-rolling).
- **`forgejo-docs`**, **`rust-crate-forgejo-docs`** — Phase 07
  (optional: docs site on Codeberg Pages; can defer to a follow-up).

Each phase that benefits from a skill calls it out in its own
Pitfalls or Plan section.

## Reference

- Originating conversation: extended design discussion on
  trigger-driven heuristics + SQLite-backed calibration loop +
  spinning skillnet out into its own public crate with HM module.
- Base skill being modified: `ai-skills/global/multi-phase-plan/SKILL.md`.
- Flavor skills inheriting changes: `ai-skills/global/multi-phase-plan-{codex,claude,mixed}/SKILL.md`.
- New crate target repo: `ssh://git@codeberg.org/caniko/skillnet.git`.
- Sister planning set for layout convention: `docs/planning/cli-rebuild/`.
