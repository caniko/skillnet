# Reconcile-Pull Research Dossier

## Goal And Trigger

User asked: "pull from target projects and global, meaning we only overwrite local
[canonical] if they are older than our version, we might've diverged. This is in
case there is no symlink, broken for some reason and overwritten by another
source."

In Option B (skillnet 0.5.x), every view is a generated symlink into a single
canonical store. The current sync logic assumes the only legitimate writer of
content is the canonical store; views are either correct symlinks, broken
symlinks, missing entries, or surprise non-symlinks (`DriftKind::NonSymlink` in
[src/view.rs:59](../../../src/view.rs#L59)). The only resolutions today are
`--force` (blindly replace the surprise content with a symlink, destroying any
unique edits) or `--allow-delete` (prune stale entries). Neither preserves
divergent content that may have landed inside a view by accident.

The user wants a third resolution path: when a view entry is a real directory
whose content is newer than its canonical sibling (because some other tool
clobbered the symlink and edits happened against the resulting copy), pull the
view content back into canonical before re-materialising the symlink. This is
the recovery path for the failure mode "symlink got overwritten by another
source and we kept working in the copy."

This dossier feeds a multi-phase plan to modify skillnet itself (across
`/data/nvme0/can/Projects/skillnet` source + the Nix HM module + the
documentation). The change set spans Rust library code, CLI surface, tests,
docs/mdBook, and the Home Manager activation flow.

## Current Reality

### What skillnet does today

- `skillnet sync` ([src/cli/args.rs:71-79](../../../src/cli/args.rs#L71-L79),
  [src/cli/mod.rs:85-91](../../../src/cli/mod.rs#L85-L91)) calls
  `commands::view::sync` then `commands::project_sync` with the same
  `allow_delete` / `force` flags.
- `materialize_view_with_options`
  ([src/view.rs:82-113](../../../src/view.rs#L82-L113)) walks canonical skills,
  computes the desired symlink target for each, and calls `ensure_symlink`
  ([src/view.rs:337-359](../../../src/view.rs#L337-L359)).
- `ensure_symlink` on a non-symlink entry returns
  `"{link} exists and is not a symlink; pass --force to replace it"`
  ([src/view.rs:352](../../../src/view.rs#L352)). With `--force`,
  `remove_view_entry` deletes the entry and a fresh symlink is created
  ([src/view.rs:424-432](../../../src/view.rs#L424-L432)) — any unique edits
  the entry held are gone forever.
- `view_status` already enumerates `DriftKind::NonSymlink` entries
  ([src/view.rs:143-150](../../../src/view.rs#L143-L150)), so the new logic
  already has a precise list of recovery candidates without re-scanning.

### The primitives we still have

The pre-`0.5.0` reconciler is gone (deleted in `15a1352`), but its
content-and-time primitives survive in [src/fs_ops.rs](../../../src/fs_ops.rs):

| Function                   | Location                                              | What it does                                                                                                                                 |
| -------------------------- | ----------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `newest_mtime_nanos(path)` | [fs_ops.rs:15-32](../../../src/fs_ops.rs#L15-L32)     | Walks every file + symlink under the skill dir, returns max mtime in nanos                                                                   |
| `content_signature(path)`  | [fs_ops.rs:34-70](../../../src/fs_ops.rs#L34-L70)     | sha256 of (sorted relative path + 0x00 + kind + bytes/target + 0x00) per entry — covers files and symlinks but not executable-bit-only diffs |
| `copy_dir(src, dest)`      | [fs_ops.rs:72-112](../../../src/fs_ops.rs#L72-L112)   | Recursive copy preserving permissions and mtimes (files, symlinks, and dirs) via `filetime`                                                  |
| `ensure_skill_dir(path)`   | [fs_ops.rs:166-171](../../../src/fs_ops.rs#L166-L171) | Validates a skill dir by `SKILL.md` presence                                                                                                 |

The deleted `reconcile::overwrite_action` had the exact selection ladder we
need (reversed direction: we want view → canonical):

> if `allow_older` → take incoming; else compare mtimes; if equal mtimes
> compare content_signature; if equal both → no-op; else bail with
> "equal-mtime conflicting skill content" and require an explicit override.
> (Git history: `git show 15a1352^:src/reconcile.rs` lines 272-296.)

The deleted `write_skill_set` ([reconcile.rs lines 197-264 in `15a1352^`](#))
used a `.skillnet-tmp` staging dir + `replace_dir` for atomic per-target
writes — same pattern we need on the canonical side.

### Source of truth and dirty-state gating

- Canonical store lives at `<mirror_root>/global/` and
  `<project>/<canonical_rel>/` (per
  [docs/src/migration/option-b.md:46-52](../migration/option-b.md#L46-L52)).
- Writes to canonical are gated by `Context::ensure_destination_clean`
  ([src/commands/context.rs:40-45](../../../src/commands/context.rs#L40-L45)),
  which calls `vcs::ensure_clean` and refuses unless `--dry-run` or
  `--allow-dirty-destination` is set
  ([src/vcs.rs:51-64](../../../src/vcs.rs#L51-L64)). Reconcile-pull must use
  the same gate when its target is the global canonical (which lives in the
  `ai-skills` repo) **or** when a project canonical is itself a git checkout.
- Per-project canonical roots live under each project repo, so each project
  has its own git working tree to consult — the existing code only checks
  `ctx.mirror_root`, so a per-target git check needs to be added.

### Today's failure modes that motivated the request

`skillnet status --all` and `skillnet doctor` against the live host currently
report **no drift** across 12 scopes and 163 skills — there is no extant
corruption to inspect. The user is pre-empting the case that has happened
before (the deleted
[docs/planning/reconciliation-anomalies-research.md](#) — staged for delete on
`main`, recoverable via `git show HEAD:docs/planning/reconciliation-anomalies-research.md`)
documented a related symptom: when multiple writable sources existed, edits
landed in one source but were silently invisible to the manifest.
Reconcile-pull is the narrower successor: only one writable source remains
(canonical), but a view can become an accidental writable source when an
external tool replaces the symlink with a directory.

## Evidence Inventory

| Source                                                                    | Lines                                                                    | What it proves                                                                                                                                                                                                                           |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `which skillnet && readlink -f $(which skillnet)`                         | n/a                                                                      | Installed binary is `skillnet-0.5.1` from a Nix store path; source matches local repo at HEAD                                                                                                                                            |
| `cd /data/nvme0/can/Projects/skillnet && git log --oneline -20`           | n/a                                                                      | Recent commits show `21f1f83 Add top-level sync command` (current `sync`), `15a1352 feat!: delete reconcile, sync pull/roundtrip; release 0.5.0` (the deletion this dossier partially reverses), `00ada1a feat!: Option B config schema` |
| `git show 15a1352 --stat`                                                 | n/a                                                                      | Shows `src/reconcile.rs` (534 lines), `src/commands/sync.rs` (787 lines), `src/cache.rs`, `src/codex.rs` were the major deletions                                                                                                        |
| `git show 15a1352^:src/reconcile.rs`                                      | full file                                                                | Recovers the prior reconcile model — `Candidate`, `Choice`, `choose_latest`, `overwrite_action`, `write_skill_set`, `RECONCILIATION.md` manifest writer                                                                                  |
| [src/view.rs:32-76](../../../src/view.rs#L32-L76)                         | DriftKind enum + structs                                                 | Confirms `NonSymlink` is already a first-class drift class                                                                                                                                                                               |
| [src/view.rs:337-359](../../../src/view.rs#L337-L359)                     | `ensure_symlink`                                                         | Confirms `--force` is the only existing escape hatch for non-symlinks                                                                                                                                                                    |
| [src/fs_ops.rs:15-70](../../../src/fs_ops.rs#L15-L70)                     | mtime + sha primitives                                                   | The exact comparators reconcile-pull needs are present                                                                                                                                                                                   |
| [src/commands/context.rs:40-45](../../../src/commands/context.rs#L40-L45) | dirty-destination gate                                                   | Confirms the gate exists for mirror_root but not per-project canonical roots                                                                                                                                                             |
| [src/cli/args.rs:686-718](../../../src/cli/args.rs#L686-L718)             | sync defaults tests                                                      | Pins current default flag shape; new flag must keep `allow_delete: false, force: false` as defaults                                                                                                                                      |
| `skillnet status --all` on the live host                                  | n/a                                                                      | All 12 scopes report `clean`, so testing must rely on fixtures, not live drift                                                                                                                                                           |
| `skillnet doctor`                                                         | "doctor: no issues"                                                      | Confirms no current invariant violations to anchor reconcile against                                                                                                                                                                     |
| [docs/src/migration/option-b.md:78-89](../migration/option-b.md#L78-L89)  | doctor invariants list                                                   | Reconcile-pull must keep these invariants; a partial pull is a doctor-correctable state, not a permanent one                                                                                                                             |
| [docs/src/commands.md:111-113](../commands.md#L111-L113)                  | "The removed `sync pull` auto-commit flow has no replacement in `0.5.0`" | Confirms the user is asking for a deliberate (partial) revert of a documented decision                                                                                                                                                   |

## Existing Plan Status

Only one prior plan set is materially related, and it has been retired:

- `docs/planning/mirror-canonical-store/` and `docs/planning/mirror-canonical-store-research.md`
  in the `ai-skills` repo. All staged for deletion on `main`
  (`git status` in `/data/nvme0/can/Projects/ai-skills` shows 22 `D` rows
  under `docs/planning/`). The plan that originally drove the v0.5.0 removal
  of reconcile is **done and being garbage-collected** — its premise was
  "canonical is the only writer, reconcile is unnecessary." This dossier
  amends that premise: canonical is the only _intended_ writer, but external
  tools can accidentally promote a view to a writer, so we need a one-way
  recovery path.
- `docs/planning/reconciliation-anomalies-research.md` in `ai-skills`. Also
  staged for delete. Its findings (mtime-tie tie-break, manifest staleness)
  are not directly relevant to reconcile-pull, which is a one-way pull rather
  than multi-source arbitration. Cite it for context only.

No existing plan covers the reconcile-pull workflow. Nothing to carry forward.

## Work That Should Survive

These constraints from the audit must shape the new implementation:

1. **Canonical remains the only intentional writer.** Reconcile-pull is a
   recovery path, not a return to multi-source arbitration. The default
   behaviour of `skillnet sync` must remain "treat view drift as an error
   that requires opt-in resolution"; reconcile-pull must be opt-in.
2. **Per-skill atomicity, mirroring the existing view materialisation
   contract.** [src/view.rs:7-14](../../../src/view.rs#L7-L14) documents that
   "Atomicity is per skill"; the pull-side must match that boundary.
3. **mtime-preserving copies stay.** `copy_dir`'s
   `set_file_times`/`set_symlink_file_times` calls are load-bearing — they
   prevent endless ping-pong after a successful pull, because the canonical
   side ends up with the same mtime as the view that just won. The new code
   must not regress that.
4. **`--allow-dirty-destination` gate must extend to project canonicals.**
   Today `ensure_destination_clean` checks only `ctx.mirror_root`. Pulling
   into `<project>/.skills` mutates a different repo; the gate must check the
   project's own git working tree.
5. **Doctor stays the source of truth for invariants.** A partially pulled
   state (view → canonical succeeded but symlink not re-established yet)
   should be a doctor error, not silent.
6. **The deleted `RECONCILIATION.md` manifest pattern should NOT be revived.**
   The committed v0.5.0 CHANGELOG removed it deliberately; per-pull telemetry
   should live in dry-run output and the calibration database, not in a file
   that drifts.

## Blockers And Missing Artifacts

None blocking. Two soft inputs deserve a decision before phase 1 starts:

- **Adoption policy for unknown skills in a view.** If a view directory
  contains a non-symlink whose name has no canonical counterpart at all (e.g.
  `~/.claude/skills/some-new-thing/` with no `global/some-new-thing/`), is
  this a _new skill to promote_ or _foreign content to leave alone_? Today
  `view_status` would classify it as `Stale` and `--allow-delete` would prune
  it. Recommendation: keep "Stale" semantics by default; add a separate
  `--adopt-new` flag (or refuse) so promotion is always explicit. Open
  decision below.
- **Project repo dirty-state policy.** When pulling into a project canonical
  whose own repo is dirty, default behaviour should match the
  `ai-skills`-side gate (refuse, require `--allow-dirty-destination`). The
  flag is currently global; whether to allow per-target dirty overrides is
  also an open decision below.

## Risks And Constraints

- **Architectural reversal risk.** v0.5.0 advertised "no reconcile". Even a
  narrow reconcile-pull will surprise users who relied on the simpler mental
  model. Mitigations: ship behind explicit subcommand/flag (no default
  behaviour change for `skillnet sync`); document clearly in
  CHANGELOG/MIGRATION; keep the comparator strictly one-way (view → canonical
  on pull, never the reverse).
- **mtime-spoof risk.** A malicious or buggy tool can `touch -t` future
  mtimes on a view copy. Reconcile-pull would treat it as newer and overwrite
  canonical with attacker content. Mitigations: (a) document that pull-mode
  trusts filesystem mtimes; (b) `--dry-run` always available; (c) `doctor`
  warns about NonSymlink-with-newer-mtime before sync ever pulls.
- **Symlink resolution hazard.** A reconcile-pull walker must use
  `fs::symlink_metadata`, never `fs::metadata`. `content_signature` and
  `newest_mtime_nanos` already do this, but the new code that decides
  "should I attempt pull on this view entry?" must check `is_symlink()` first
  to avoid pulling a perfectly valid symlink's target into itself.
- **Cross-filesystem rename.** Reconcile-pull's atomic write should mirror
  the deleted `write_skill_set`: write to a `.skillnet-tmp` staging dir
  inside the canonical's parent, then `rename`/`replace_dir` over the
  existing canonical skill dir. Same filesystem guarantee that already holds
  for `ai-skills` and per-project repos.
- **Permission preservation.** `copy_dir` already preserves modes
  ([fs_ops.rs:88-105](../../../src/fs_ops.rs#L88-L105)). Reconcile-pull must
  reuse it, not roll its own copy.
- **Ambiguous additive change.** If canonical gained a new file
  (e.g. `references/new.md`) while the view also gained edits, per-skill
  "view newer wins" loses the new canonical file. The deleted reconciler
  bailed on this case at the mtime-tie boundary only. Recommendation: keep
  the per-skill atomicity but require `--prefer view|canonical` on a true
  three-way diff (both sides advanced). Most real cases will be unambiguous
  (one side untouched since the symlink broke), so the bail path is rare.
- **Catalog implications.** [skillnet.catalog.toml](../../../../ai-skills/skillnet.catalog.toml)
  rules apply to canonical paths; reconcile-pull writes into canonical, so
  any rule status changes (e.g. `status = "active"` based on `path_prefix`)
  apply automatically post-pull. No catalog change is needed in this phase.
- **HM module surface.** [nix/hm-module.nix](../../../nix/hm-module.nix)
  drives the install on the host; if reconcile-pull becomes part of the HM
  activation script (e.g., a default `sync` extended with `--reconcile`),
  it must be opt-in to avoid silent canonical mutation on every
  `home-manager switch`.

## Candidate Next Steps

A natural decomposition for the planner, in dependency order:

### Phase A — Library primitives (skillnet/src/view.rs + fs_ops.rs)

Introduce a `reconcile_view` primitive parallel to `materialize_view`. It
should:

1. Compute the set of view entries that are `DriftKind::NonSymlink` (already
   available via `view_status_with_options`).
2. For each such entry, decide an outcome:
   - `NoView` (entry missing): nothing to pull.
   - `Identical` (sha matches canonical): safe to demote to symlink without
     pulling.
   - `ViewNewer` (view newest_mtime > canonical newest_mtime): pull view →
     canonical via `copy_dir` into staging + atomic rename.
   - `CanonicalNewer`: leave canonical; report; require `--force` to demote.
   - `EqualMtimeDifferentContent`: bail with `--prefer view|canonical`
     required.
   - `BothAdvanced` (view newer + canonical has files view does not): bail
     with `--prefer view|canonical` required.
   - `AdoptCandidate` (skill name has no canonical sibling): no-op unless
     `--adopt-new`; then promote.

Return a `ReconcileReport` per view, with per-entry outcomes and per-entry
sha+mtime evidence so the CLI can print and dry-run cleanly. Per-file
granularity is **not** worth pursuing: skills are authored as a unit; the
catalog treats them as units; the existing materialisation atomicity is
per-skill. Per-file granularity would also explode the conflict matrix.

Tests in [tests/view_sync.rs](../../../tests/view_sync.rs) should grow
fixtures for each outcome class — tempdir-based, mtime-controlled with
`filetime::set_file_times` to avoid sleep-based flake.

### Phase B — Canonical-side write with extended dirty gating

1. Generalise `ensure_destination_clean` to accept a target directory (so it
   can be pointed at `<project>/<canonical_rel>` as well as `mirror_root`).
2. Add `ensure_target_clean(&Utf8Path)` to
   [src/commands/context.rs](../../../src/commands/context.rs); existing
   call sites switch to use it with `ctx.mirror_root`.
3. Reuse the deleted `write_skill_set` shape:
   stage in `<canonical>/.skillnet-tmp/<skill>`, then `fs_ops::replace_dir`
   atomically. The old code in `git show 15a1352^:src/reconcile.rs` lines
   197-264 is a template — copy the structure, drop the multi-source
   `Choice` machinery, keep the staging + atomic-rename + manifest-omission
   parts.
4. After a successful pull, re-run `materialize_view_with_options` with
   `force = true` for the affected skill only — the entry was already a
   real dir, demotion to symlink is unambiguous now that canonical holds
   identical content.

### Phase C — CLI surface

Two-tier surface, no breaking changes:

1. **New per-scope subcommand** `skillnet view reconcile` and
   `skillnet project reconcile` (parallel to `sync`/`status`/`diff`):
   ```
   skillnet view reconcile --all
       [--allow-delete] [--force-canonical] [--prefer view|canonical]
       [--adopt-new]
   ```

   - Default (no flags): pull `ViewNewer` outcomes only; bail on anything
     that needs a decision; print a status table for the rest.
   - `--prefer view` / `--prefer canonical`: resolve `EqualMtimeDifferentContent`
     and `BothAdvanced` deterministically.
   - `--force-canonical`: existing `--force` is a footgun (destroys view);
     rename it to `--force-canonical` here for clarity and accept the old
     name as a deprecated alias for one release.
   - `--adopt-new`: promote unknown view skills into canonical.
2. **Top-level convenience** `skillnet sync --pull`: runs `view reconcile
--all` then `project reconcile --all` then the existing
   `materialize_view`/`materialize_project` pass. Default of
   `skillnet sync` (no flag) stays exactly as today.
3. Standard JSON output via `--format json` for the report, matching the
   existing `view_status_with_options` pattern.
4. Honour `--dry-run` and `--allow-dirty-destination` from the global flag
   set already in
   [src/cli/args.rs:32-38](../../../src/cli/args.rs#L32-L38).

### Phase D — Doctor wiring

In [src/commands/doctor.rs](../../../src/commands/doctor.rs), extend
`check_global_view` and the project-view check to upgrade a `NonSymlink`
entry's severity when it would be a pull candidate:

- `NonSymlink` with `view newer mtime` → `Severity::Warn` with hint
  `"run skillnet view reconcile --scope <scope> to pull view-side edits before sync"`.
- `NonSymlink` with `canonical newer mtime` or `Identical` → keep
  `Severity::Error` (today's behaviour); these are unambiguously
  reproducible by `sync --force`.

This way the new behaviour is _discoverable_ without changing default
mutation.

### Phase E — Docs, tests, release

- [docs/src/commands.md](../commands.md): add a `reconcile` section
  explaining the comparator ladder, the conflict outcomes, and the
  intentional asymmetry (view → canonical only).
- [docs/src/migration/option-b.md](../migration/option-b.md): append a
  "Reconcile-pull (post-0.5.x)" note clarifying that this is _not_ a return
  to multi-source arbitration.
- [CHANGELOG.md](../../../CHANGELOG.md): record the new commands and flag
  defaults under `[Unreleased]`; cut a `0.6.0` release because the new
  behaviour rewords the v0.5.0 advertised "no reconcile" stance.
- [nix/hm-module.nix](../../../nix/hm-module.nix): expose an opt-in
  `programs.skillnet.activation.reconcile = true|false` so users can wire
  reconcile-pull into `home-manager switch` activation if desired. Default
  must be `false`.
- [tests/](../../../tests/): add `tests/reconcile_pull.rs` covering each
  outcome class; extend `tests/cli.rs` to assert the new `--pull` flag is
  parsed; add a CLI default-shape test analogous to
  `sync_command_defaults_to_safe_flags` to pin the new subcommand defaults.

### Sequencing

A → B can be done in one cohesive phase (both touch the library). C and D
depend on A+B. E ships alongside C/D. No parallelism is needed; one
implementer end-to-end is the cheapest path.

## Open Decisions For The User

1. **Adoption policy for unknown view skills.** Recommended default: never
   adopt without `--adopt-new`. Alternative: refuse (`--allow-delete` and
   nothing else). Pick one — it shapes Phase A's outcome enum.
2. **CLI shape for the new behaviour.** Recommended: per-scope
   `skillnet view reconcile` + `skillnet project reconcile` and top-level
   `skillnet sync --pull`. Alternative: a top-level `skillnet pull`
   subcommand. The recommended path keeps the per-scope verbs aligned with
   the existing `sync`/`status`/`diff` triad.
3. **`--force` rename.** Recommended: rename current `--force` to
   `--force-canonical` on the new reconcile commands (kept as
   `--force` on the existing `sync` commands to avoid breaking). Reject if
   you prefer flag stability over clarity.
4. **HM activation behaviour.** Recommended: ship the HM toggle in Phase E
   but leave it `false` by default. Confirm or pick "do not expose at all
   until requested".
5. **Per-target dirty-destination policy.** Recommended: extend the global
   `--allow-dirty-destination` flag to gate _every_ canonical write,
   including project canonicals. Alternative: per-scope flag like
   `--allow-dirty <scope>`. The recommended path keeps the flag surface
   minimal.
