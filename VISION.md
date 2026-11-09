# skillnet vision

`skillnet` is a CLI for managing canonical AI skill stores, materialising
derived agent views from them, generating a catalog, and recording calibration
and usage data for `multi-phase-plan`. Global views may be symlinks or
hardlinks; project skills live in `<project>/.skills` with generated working
copies under `.agents/skills` and per-skill symlinks under `.claude/skills`. It
is used by operators and agents that need one reviewed copy of a skill set to
feed several harnesses.

## Problem

Every agent harness wants skills in its own directory with its own layout.
Copying files into each of them drifts: edits land in a view instead of the
canonical store, and there is no record of which skill a session actually used.

## Principles

- One canonical store per scope. Agent directories are derived views
  materialised from that store; edits belong in the canonical store, not in a
  view. The 0.5 migration removed reconcile source arbitration and the
  `skillnet sync` command group in favour of that single source.
- Hardlink views are for immutable sharing: when several agent roots should
  share files without duplicating content, they hardlink from canonical, and
  the canonical store remains the edit target.
- Usage data is metadata only. Events store no skill content, and harness
  adapters pass a source-event ID so retries are idempotent.
- Zero-use is not automatically evidence of non-use. A report includes every
  catalog skill, and a zero count only means something when the harness coverage
  was healthy for the whole window.
- Backends are explicit. Postgres is the default calibration backend with SQLite
  selectable, and the Home Manager module folds its options into the generated
  `skillnet.toml` so declarative installs do not depend on shell environment.
- The supported surface is the `skillnet` binary. No stable embeddable Rust API
  is promised.

## Scope

- Maintain canonical global and project skill stores.
- Materialise generated symlink and hardlink views for agents.
- Materialise canonical project skills in the `ai-skills` checkout and
  hardlinked working copies inside project repositories.
- Inspect, move, rename, delete, export, and bundle mirrored skills, including
  Pkl skill bundles and merged external Pkl manifests.
- Generate catalog output from `skillnet.catalog.toml`.
- Record calibration data for `multi-phase-plan` and normalized skill usage, with
  `hook install`/`hook ingest` for Claude Code and `usage record`/`usage report`
  for adapted harnesses.
- Install and configure the binary through the exported Nix Home Manager module.

## Non-goals

- It does not own external plugin payloads or harness-specific installation
  policy. Harness integrations such as Infernix own adapter and plugin
  installation; skillnet owns canonical skills, views, catalogs, and normalized
  usage events.

## Direction

The checked-in design research for two-way sync and config centralisation is
marked closed, with the design finalised and ready for a `multi-phase-plan`
handoff: smart symlink materialisation, a single resolve-and-sync command, and
user-level configuration definable through the Nix module. Recent commits add
bundle and global-scope support, intersect skill grants with destination hosts,
and move manifest evaluation to restricted synchronous Pklr.

## Place in the fleet

Canix consumes skillnet as a flake input and imports
`inputs.skillnet.homeModules.default` through `home/user/shared/skillnet.nix`,
which declares the Claude and agents views, the mirror root, the catalog, and
the database backend, and which renders external `Skillnet.pkl` manifests for
the `scientific-agent-skills` checkout and for loop-template skills. The catalog
classifies skills by path prefix, category, and tags, including the
`global/simit-*` project-init skills. On atlas the skills root points at the
`ai-skills` checkout.
