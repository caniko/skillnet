# Phase 09 — ai-skills consumption + flavor propagation

> **Recommended Codex model: GPT 5.5 medium**
>
> Closes the loop on the ai-skills side: adds the skillnet flake
> input, exposes its HM module from ai-skills' own flake (so
> downstream consumers of ai-skills get skillnet transparently),
> propagates the `calibrate` mode to the three flavor wrappers, and
> deletes the old in-repo skillnet Rust sources now that they live
> at the published crate. Moderate complexity: the flake plumbing
> needs to flow inputs correctly through the module system, and the
> source deletion is a non-reversible cleanup that must happen only
> after every other phase has accepted. `low` would underspec the
> flake re-export pattern; `high` is unnecessary.

## Working tree

`/data/nvme0/can/Projects/ai-skills` (this repo). The published
skillnet is at `ssh://git@codeberg.org/caniko/skillnet.git`; this
phase only consumes it.

## Goal

1. `ai-skills/flake.nix` has `inputs.skillnet` pointing at the
   Codeberg-published flake.
2. ai-skills' flake re-exports skillnet's HM module so users who
   import ai-skills' HM module get skillnet automatically, or can
   opt in/out per their preference.
3. The three flavor wrappers (`-codex`, `-claude`, `-mixed`) each
   list `calibrate` as a third mode and reference the base skill's
   calibrate body.
4. The old skillnet Rust sources in `ai-skills/src/`,
   `ai-skills/tests/`, `ai-skills/Cargo.toml`, `ai-skills/Cargo.lock`,
   `ai-skills/data/multi-phase-plan/` are deleted (they live in the
   skillnet repo now). The ai-skills repo becomes Rust-free.
5. Repo-root `README.md` is updated to point at the skillnet repo
   for CLI work and reflect the post-extraction state.

After this phase: a fresh `home-manager switch` on a clean machine
with ai-skills' HM module enabled installs skillnet, the hooks in
`multi-phase-plan` work without any `nix run` ceremony, and the
calibration loop is fully operational.

## Why this matters now

Phases 06 (hooks) and 08 (HM module) prepared both sides of the
seam. This phase actually flips the consumption: removes the
duplicate sources, wires the flake, and propagates modes. Without
it, the ai-skills repo still contains skillnet sources that drift
from the published crate, and the flavor wrappers don't tell users
about `calibrate` mode.

## Out of scope

- Any change to skillnet itself — done in 01–04, 07, 08.
- Any change to the base skill body — done in 05, 06.
- Removing other (non-skillnet) sources from ai-skills — this repo
  has more than just skillnet in it; only the skillnet-related
  files leave.
- Migrating existing calibration data from any pre-Phase-08
  location to the new XDG path. There is no pre-existing data
  (this is a greenfield loop); skip the migration concern.

## Plan

1. **Pre-requisite check.** Confirm all of:
   - Phase 07 has tagged and published at least `v0.1.0` (or
     `v0.1.1` after Phase 08).
   - Phase 08 has tagged and published a version exposing
     `hmModules.default`.
   - Phase 06 has rewritten `global/multi-phase-plan/SKILL.md`
     with hooks + calibrate mode.
   - The published crate works (`cargo install skillnet` succeeds
     on a clean machine; `nix run codeberg.org:caniko/skillnet --
--help` succeeds).

   Without all four, this phase is premature; pause and finish the
   prereqs.

2. **Add the flake input.** In `ai-skills/flake.nix`:

   ```nix
   inputs = {
     # … existing inputs …
     skillnet = {
       url = "git+ssh://git@codeberg.org/caniko/skillnet.git";
       inputs.nixpkgs.follows = "nixpkgs";   # if ai-skills already pins nixpkgs
     };
   };
   ```

   Run `nix flake update skillnet` (or `nix flake lock --update-input
skillnet`) to populate `flake.lock`. Commit the lock change.

3. **Re-export the HM module.** Decide the consumption story:
   - **Option A (recommended): pass-through.** ai-skills' flake
     exposes its own `hmModules.default` that imports both
     ai-skills' own modules (if any) and skillnet's. Downstream
     users who import ai-skills' HM module get skillnet
     automatically.
     ```nix
     outputs = { self, nixpkgs, skillnet, ... }: {
       hmModules.default = { ... }: {
         imports = [
           skillnet.hmModules.default
           # … any ai-skills-own HM modules go here …
         ];
       };
     };
     ```
   - **Option B (alternative): re-export only, no auto-import.**
     ai-skills exposes `hmModules.skillnet = skillnet.hmModules.default;`
     so users opt in explicitly. More verbose for users but more
     composable.
   - Pick A; document the choice in the flake.nix comment.

4. **Update flavor wrappers.** For each of
   `global/multi-phase-plan-codex/SKILL.md`,
   `global/multi-phase-plan-claude/SKILL.md`,
   `global/multi-phase-plan-mixed/SKILL.md`:
   - Find the "## Modes" section. It currently lists `plan` and
     `verify`. Add a third entry:
     > - **calibrate** — when the user says "calibrate", "tune the
     >   heuristics", or "review calibration data", invoke the base
     >   skill's `calibrate` mode. See
     >   `global/multi-phase-plan/SKILL.md` "Mode: calibrate" for
     >   the workflow. The calibrate mode shells out to the
     >   installed `skillnet calibration …` binary; users who have
     >   the skillnet HM module enabled get this transparently
     >   (see ai-skills' `flake.nix` HM module re-export).
   - Find the "## Plan workflow" section. Append a short final
     step:
     > **Calibration recording.** Follow the base skill's
     > end-of-plan hook step: evaluate meta-heuristics, write
     > `.calibration.json` if any fire, and run `skillnet
calibration record <plan-dir>`.
   - The flavor-specific routing skill (gpt-plan-routing for codex,
     claude-plan-routing for claude, both for mixed) is _not_
     consulted for `calibrate` — calibrate analyzes past plans, it
     doesn't route new ones. Add a one-line note to that effect in
     the calibrate entry.
   - Do not duplicate the calibrate body or the heuristics catalog;
     wrappers stay thin.

5. **Delete the old skillnet sources from ai-skills.** Verify first
   that nothing else in ai-skills depends on them:

   ```sh
   rg "use crate::calibration" .                  # should match nothing
   rg "skillnet" --type rust .                    # confirm scope
   ```

   Then:

   ```sh
   git rm -r src/calibration/
   git rm -r src/catalog/ src/cli/ src/commands/  # if these are entirely skillnet
   git rm src/main.rs src/lib.rs src/config.rs src/fs_ops.rs src/model.rs src/reconcile.rs src/cache.rs
   git rm Cargo.toml Cargo.lock
   git rm -r tests/                               # if all tests are skillnet
   git rm -r data/multi-phase-plan/               # schema lives in the crate now
   ```

   **Caution**: ai-skills currently contains skillnet as its
   primary content (per repo status). If the consensus is to keep
   ai-skills repo Rust-free, the deletion is wholesale. If
   ai-skills should retain some Rust tooling (e.g., a separate
   `ai-skills-mirror` binary distinct from skillnet), the
   deletion is partial — re-scope this step.

   Conservative default: do a wholesale delete and commit; if
   anything was needed, restore it from git history. This is
   reversible until the commit is pushed.

6. **Update repo-root `README.md` (or `CALIBRATION.md`)** with a
   short pointer:

   ```markdown
   ## Calibration

   The `multi-phase-plan` skill records calibration data via
   `skillnet`, a separate published crate at
   <https://codeberg.org/caniko/skillnet> (also on crates.io).

   To install: enable ai-skills' HM module (which re-exports
   skillnet's), or directly `cargo install skillnet`. The skill's
   hooks shell out to the installed binary; data lives at
   `$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`.

   See `global/multi-phase-plan/SKILL.md` for the heuristics
   catalog, sidecar schema, calibrate mode workflow, and
   calibration changelog.
   ```

7. **Validate.**
   - `nix flake check` in ai-skills → clean.
   - On a clean test machine (or VM): clone ai-skills, enable its
     HM module via a sample HM config, run `home-manager switch`,
     verify `skillnet --help` works.
   - Trigger an end-to-end smoke: write a small phase plan via the
     `multi-phase-plan` skill, observe that the end-of-plan hook
     fires and records (when a meta-heuristic triggers), confirm
     the row appears in `$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`.
   - Verify ai-skills builds (its `mdbook` or other tooling, if
     any) without the removed Rust sources.

8. **Update the calibration changelog footer in
   `global/multi-phase-plan/SKILL.md`** with the first entry — not
   a threshold change, but a provenance note for the loop's
   activation:

   ```markdown
   ### YYYY-MM-DD — Calibration loop activated

   - Initial heuristic thresholds set per Phase 05 of the
     `calibration-loop` plan.
   - skillnet `0.1.1` (or whichever version) consumed via HM module.
   - Calibration database lives at
     `$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`.

   No threshold changes in this entry; this is the genesis row.
   Future entries will be appended by `skillnet calibration
   export-changelog` runs via the `calibrate` mode.
   ```

## Acceptance criteria

- [ ] `ai-skills/flake.nix` has `inputs.skillnet` pointing at
      `git+ssh://git@codeberg.org/caniko/skillnet.git`.
- [ ] `ai-skills/flake.lock` has the locked skillnet revision.
- [ ] `ai-skills/flake.nix` exposes `hmModules.default` that
      imports skillnet's HM module (Option A from Plan step 3).
- [ ] Each of `global/multi-phase-plan-{codex,claude,mixed}/SKILL.md`
      lists three modes including `calibrate`, with the
      flavor-specific note that calibrate doesn't consult routing
      skills.
- [ ] Each flavor wrapper documents the end-of-plan hook step.
- [ ] No flavor wrapper duplicates the calibrate body or
      heuristics catalog.
- [ ] Old skillnet Rust sources (`src/calibration/`, `src/catalog/`,
      `src/cli/`, `src/commands/`, `src/main.rs`, etc.),
      `Cargo.toml`, `Cargo.lock`, `tests/`, and
      `data/multi-phase-plan/` are removed from ai-skills.
- [ ] `nix flake check` in ai-skills is clean post-deletion.
- [ ] On a clean machine, enabling ai-skills' HM module and running
      `home-manager switch` puts `skillnet` on PATH.
- [ ] Repo-root `README.md` (or `CALIBRATION.md`) has the
      calibration pointer described in Plan step 6.
- [ ] The calibration changelog footer in
      `global/multi-phase-plan/SKILL.md` has a genesis entry.
- [ ] End-to-end smoke: a meta-heuristic-triggering plan generates
      a recorded row at
      `$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`.

## Files likely touched

- `ai-skills/flake.nix` (+ `inputs.skillnet`, + `hmModules.default`)
- `ai-skills/flake.lock` (auto-updated)
- `ai-skills/global/multi-phase-plan-codex/SKILL.md`
- `ai-skills/global/multi-phase-plan-claude/SKILL.md`
- `ai-skills/global/multi-phase-plan-mixed/SKILL.md`
- `ai-skills/global/multi-phase-plan/SKILL.md` (calibration
  changelog genesis entry only — Phase 06's content is preserved)
- `ai-skills/README.md` (calibration pointer; alternatively a new
  `CALIBRATION.md`)
- Deletions: `ai-skills/src/**/*`, `ai-skills/Cargo.toml`,
  `ai-skills/Cargo.lock`, `ai-skills/tests/`,
  `ai-skills/data/multi-phase-plan/`

## Pitfalls

- **Wholesale source deletion is non-reversible after push.** If
  ai-skills should retain some Rust tooling, partial-delete only.
  If wholesale is correct, commit on a feature branch first, run
  `nix flake check`, smoke test, _then_ merge to main.
- **Flake input via SSH requires SSH agent / keys.** For CI or
  users without SSH access to Codeberg, document the
  `git+https://codeberg.org/caniko/skillnet.git` alternative.
  Note this in the README.
- **`inputs.nixpkgs.follows`.** If ai-skills pins a specific
  nixpkgs and skillnet pins a different one, `follows` collapses
  them. Without `follows`, the flake brings in two nixpkgs trees
  (slow, bloats the lockfile).
- **Pass-through HM module composition.** Option A's
  `hmModules.default` collapses ai-skills' own modules with
  skillnet's. If a downstream user wants ai-skills' modules
  _without_ skillnet, they can't easily do that with Option A.
  If this matters, switch to Option B (explicit per-module
  imports) and document the choice.
- **Flavor-wrapper drift.** Three near-copy files. Edit all three
  with the same wording (modulo flavor-specific routing-skill
  name). Grep them after to confirm.
- **Don't update `multi-phase-dispatch`.** That sister skill is
  out of scope; calibrate mode is in the base, not in dispatch.
- **The `calibrate` mode entry needs to mention the no-routing
  invariant.** Both codex and claude flavors have routing skills
  (gpt-plan-routing, claude-plan-routing); the wrapper currently
  documents how `plan` mode consults them. Calibrate doesn't.
  Make this explicit so the user (or the agent) doesn't try to
  invoke routing for calibrate.
- **Test on a _clean_ machine.** Your dev machine already has
  skillnet installed (from Phase 07 cargo-install or Phase 08 HM
  test). Use a VM or container to confirm the fresh-install path.
- **Calibration database starts empty.** First few `calibrate`
  runs will report "no triggers above min-N" and emit no
  proposals. That's expected; document it in the genesis
  changelog entry so the user isn't confused when the first
  calibrate-mode session looks empty.
- **`berg-codeberg-ci` may help** if the skillnet publish CI runs
  intersect with anything in ai-skills (unlikely but possible).

## Reference

- Parent plan: `docs/planning/calibration-loop/README.md`.
- Predecessor phases: `06-base-skill-hooks-calibrate-mode.md`
  (hooks must exist), `07-skillnet-crate-publication.md` (crate
  must be published), `08-nix-hm-module.md` (HM module must be
  exported).
- HM module composition reference:
  <https://nix-community.github.io/home-manager/index.xhtml#sec-flakes-standalone>.
- ai-skills repo: `/data/nvme0/can/Projects/ai-skills`.
- skillnet repo: `ssh://git@codeberg.org/caniko/skillnet.git` →
  `/data/nvme0/can/Projects/skillnet`.
