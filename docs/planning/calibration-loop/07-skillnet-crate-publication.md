# Phase 07 — skillnet crate publication

> **Recommended Codex model: GPT 5.5 high**
>
> First-ever crates.io publish of `skillnet`, plus Codeberg/Forgejo
> CI on the self-hosted atlas runner. Mechanically large but
> well-trodden: this repo's own skills (`rust-crate-release-prep`,
> `rust-crate-release-chaperone`, `rust-crate-publish-workflow`,
> `rust-crate-forgejo-release-ci`, `forgejo-atlas-ci`) cover most of
> the work. The substantive design content is the public-API
> stability commitment (what's `pub`, what's `pub(crate)`), the
> license choice, the MSRV, and the first version number. A
> non-reversible action (crates.io publish) at the end —
> `chaperone` mode mitigates by running every gate before allowing
> the publish. `medium` would underspec the API surface review;
> `max` is overkill given the released-software-doctrine skills are
> already battle-tested.

## Working tree

`/data/nvme0/can/Projects/skillnet` (the new standalone crate).
This phase **moves** the Rust sources currently in
`/data/nvme0/can/Projects/ai-skills/src/` into the new repo as
part of step 1 below; ai-skills' src/ tree is left behind only
after Phase 09 deletes it (so the consumer can keep building until
the migration completes).

## Goal

`skillnet` is a published crate on `crates.io`:

- Source repo at `ssh://git@codeberg.org/caniko/skillnet.git`,
  initial commit reflects the moved sources from ai-skills.
- `Cargo.toml` has complete crates.io metadata (description, license,
  repository, homepage, documentation, readme, keywords, categories,
  rust-version, package include/exclude).
- `README.md` at the crate root with crates.io-facing content:
  install instructions, brief example, link to docs.rs.
- `LICENSE-MIT` and `LICENSE-APACHE` files; SPDX `MIT OR Apache-2.0`
  in Cargo.toml.
- `CHANGELOG.md` with a `0.1.0` entry.
- Public-API surface decision: every item under
  `src/calibration/` is `pub(crate)` by default; only the CLI
  entrypoints (`main`) and a small `pub` surface for embedded use
  (TBD; default: keep everything internal in 0.1.0 and re-evaluate
  if downstream demand appears).
- Rustdoc passes `cargo doc --no-deps -- -D warnings`.
- `cargo package` produces a clean tarball under 10 MB.
- Codeberg CI workflow at `.forgejo/workflows/` runs on every push
  and PR: fmt, clippy `-D warnings`, test (including the
  calibration round-trip tests from Phases 01–04), doc, package,
  audit, deny. Runs on the self-hosted atlas runner (single label
  `atlas`).
- Tag-triggered publish workflow that runs `cargo publish` when a
  `v*` tag is pushed.
- First published version: `0.1.0`.

## Why this matters now

Phases 01–04 built the calibration subsystem inside ai-skills'
Rust crate. Phases 06, 08, 09 all need the binary installed on
user machines, which requires a publishable crate, which requires
this phase. The `crates.io` publish is non-reversible (yanking is
possible but the version number is burnt); we lean on the
`rust-crate-release-chaperone` skill to ensure every quality gate
passes before the publish runs. Codeberg CI is the same atlas
runner the rest of caniko's projects use; the existing
`rust-crate-forgejo-release-ci` skill knows how to wire it.

## Out of scope

- Nix HM module — Phase 08.
- ai-skills flake input pointing at the published crate — Phase 09.
- Removing the moved sources from ai-skills' `src/` — Phase 09 (the
  consumer needs to keep building until consumption flips over).
- Docs site on Codeberg Pages — defer to a follow-up after 0.1.0
  ships and demand is real (`rust-crate-forgejo-docs` covers it
  when needed).
- Migration to `simit`-managed release infrastructure — if simit is
  the preferred path in this org, invoke `simit-project-init`
  during step 4 below; otherwise stick with the hand-rolled
  `rust-crate-*` skills.

## Plan

1. **Bootstrap the new repo.**
   - On Codeberg, create the empty `caniko/skillnet` repository
     (manual step: web UI or `berg repo create`).
   - Locally:
     ```sh
     git clone ssh://git@codeberg.org/caniko/skillnet.git /data/nvme0/can/Projects/skillnet
     cd /data/nvme0/can/Projects/skillnet
     ```
   - Move sources from ai-skills:
     ```sh
     cp -r /data/nvme0/can/Projects/ai-skills/src .
     cp -r /data/nvme0/can/Projects/ai-skills/tests .
     cp /data/nvme0/can/Projects/ai-skills/Cargo.toml .
     cp /data/nvme0/can/Projects/ai-skills/Cargo.lock .
     cp -r /data/nvme0/can/Projects/ai-skills/data .
     ```
   - Initial commit: "Initial skillnet extraction from ai-skills".

2. **Embed the schema migrations into the binary.** In
   `src/calibration/db.rs` (delivered by Phase 01), change the
   migration runner to load SQL via `include_str!` rather than
   reading from disk at runtime:
   ```rust
   const MIGRATIONS: &[(u32, &str)] = &[
       (1, include_str!("../../data/multi-phase-plan/schema/001-initial.sql")),
   ];
   ```
   This makes the binary self-contained — users don't need the
   schema files on disk. Update the runner accordingly. Add a test
   that exercises a fresh-db migration from the embedded source.

3. **Decide the public API surface.** For 0.1.0:
   - Keep every calibration item `pub(crate)`. The CLI is the
     public surface; the library is an internal implementation
     detail. This minimizes the SemVer commitment and leaves room
     to redesign without breaking changes.
   - Document this in `src/lib.rs` (if a library target exists) or
     `src/main.rs` with a comment: "no public Rust API in 0.1.0;
     use the `skillnet` binary".
   - Re-evaluate when downstream Rust callers (other crates wanting
     to embed calibration) appear.

4. **Run `rust-crate-release-prep`** to orchestrate the readiness
   work. This skill chains:
   - `rust-crate-manifest-metadata` (Cargo.toml fields).
   - `rust-crate-legal-readme` (LICENSE files, README content,
     crates.io install/example snippets).
   - `rust-crate-rustdoc` (crate-level docs, public-item rustdoc,
     `cargo doc -- -D warnings`).
   - `rust-crate-nix-release-tooling` (flake.nix with crane,
     devShells, packages, checks — defer the HM module to Phase 08).
   - `rust-crate-quality-gates` (fmt, clippy, test, doctests, doc,
     package dry-run, cargo-deny, cargo-audit).
   - `rust-crate-forgejo-release-ci` (Codeberg CI workflow).

   If `simit-project-init` is the preferred path for caniko's
   projects, run it instead — it sets up `simit init-flake` and
   `simit init-ci` with the same scope.

5. **Cargo.toml metadata** (the `rust-crate-manifest-metadata` skill
   writes most of this; the values are):
   ```toml
   [package]
   name = "skillnet"
   version = "0.1.0"
   edition = "2021"
   rust-version = "1.88"        # bump if any dep requires newer
   description = "Reconcile and manage local AI skill mirrors; calibration data for the multi-phase-plan skill."
   license = "MIT OR Apache-2.0"
   repository = "https://codeberg.org/caniko/skillnet"
   homepage = "https://codeberg.org/caniko/skillnet"
   documentation = "https://docs.rs/skillnet"
   readme = "README.md"
   keywords = ["ai", "skills", "calibration", "claude", "codex"]
   categories = ["command-line-utilities", "development-tools"]
   include = [
     "src/**/*",
     "data/multi-phase-plan/schema/**/*.sql",
     "Cargo.toml",
     "README.md",
     "LICENSE-MIT",
     "LICENSE-APACHE",
     "CHANGELOG.md",
   ]
   ```

6. **README content** for crates.io (the `rust-crate-legal-readme`
   skill drafts most of it; key sections):
   - One-paragraph what-it-does.
   - Install: `cargo install skillnet` and `nix run codeberg.org:caniko/skillnet`
     and (forward-reference) "Nix Home Manager users see the
     module in `nix/hm-module.nix` (Phase 08)".
   - Brief example: `skillnet calibration --help`.
   - Link to docs.rs.
   - License + Codeberg repo link.

7. **CHANGELOG.md** with one entry:
   ```markdown
   ## 0.1.0 — YYYY-MM-DD

   Initial release.

   - SQLite-backed calibration data subsystem with sidecar
     (`.calibration.json`) ingest.
   - CLI: `record`, `verify`, `tag`, `untag`, `show`, `query`,
     `migrate`, `vacuum`, `export`, `analyze`, `propose`,
     `proposals`, `decide`, `export-changelog`.
   - Schema migrations embedded in the binary.
   - Codeberg/Forgejo CI on the self-hosted atlas runner.

   No public Rust library API in this release; the binary is the
   surface. Re-evaluate if downstream embedders appear.
   ```

8. **Codeberg CI workflow** at
   `.forgejo/workflows/ci.yml` (the
   `rust-crate-forgejo-release-ci` skill writes this — confirm it
   uses the `atlas` runner label exclusively). Gate jobs:
   - `fmt` — `cargo fmt --check`.
   - `clippy` — `cargo clippy --all-targets -- -D warnings`.
   - `test` — `cargo test` (includes the calibration round-trip).
   - `doc` — `cargo doc --no-deps -- -D warnings`.
   - `package` — `cargo package --no-verify` then `cargo package`.
   - `audit` — `cargo audit`.
   - `deny` — `cargo deny check`.
   - `nix-check` — `nix flake check`.
   All must pass on every push/PR to `main`.

9. **Tag-triggered publish workflow** at
   `.forgejo/workflows/release.yml`. Triggered on `push` of a tag
   matching `v*`. Runs `cargo publish` with the crates.io token
   from secrets. The `rust-crate-publish-workflow` skill specifies
   the exact YAML; follow it.

10. **Run the chaperone**. Invoke `rust-crate-release-chaperone` to
    babysit the first publish: it runs every gate, surfaces
    blockers, and only allows the `git tag v0.1.0 && git push --tags`
    step once everything is green. The chaperone is the safety net
    for the non-reversible publish.

11. **Verify on a clean machine**:
    ```sh
    # In a fresh shell, outside the dev shell, on any machine:
    cargo install skillnet
    skillnet --help
    skillnet calibration --help
    # First run creates the data dir:
    skillnet calibration migrate
    ```
    All should succeed.

12. **Push the initial readme to Codeberg** so the repo's web page
    isn't empty.

## Acceptance criteria

- [ ] `/data/nvme0/can/Projects/skillnet` exists, is initialized,
      contains the moved sources from ai-skills, and is pushed to
      `ssh://git@codeberg.org/caniko/skillnet.git`.
- [ ] Schema migrations are embedded in the binary via
      `include_str!`; running `skillnet calibration migrate` on a
      machine with no `data/` dir succeeds.
- [ ] Cargo.toml has every field listed in Plan step 5.
- [ ] `LICENSE-MIT` and `LICENSE-APACHE` exist at repo root with
      correct boilerplate.
- [ ] `README.md` covers install, example, docs.rs link, license.
- [ ] `CHANGELOG.md` has a `0.1.0` entry.
- [ ] `cargo doc --no-deps -- -D warnings` is clean.
- [ ] `cargo package` produces a tarball under 10 MB.
- [ ] `.forgejo/workflows/ci.yml` exists and runs all gates on the
      `atlas` runner.
- [ ] `.forgejo/workflows/release.yml` triggers `cargo publish` on
      `v*` tags.
- [ ] Codeberg CI is green on `main` after the initial push.
- [ ] `cargo install skillnet` works on a clean machine after the
      `v0.1.0` publish.
- [ ] `crates.io/crates/skillnet/0.1.0` resolves.
- [ ] `docs.rs/skillnet/0.1.0` renders the rustdoc (may take a few
      minutes after publish).

## Files likely touched

- `/data/nvme0/can/Projects/skillnet/` (entire new repo)
- `Cargo.toml` (full metadata)
- `README.md` (new; crates.io-facing)
- `LICENSE-MIT` (new)
- `LICENSE-APACHE` (new)
- `CHANGELOG.md` (new)
- `src/calibration/db.rs` (switch to embedded migrations via
  `include_str!`)
- `data/multi-phase-plan/schema/*.sql` (preserved; still source of
  truth for `include_str!`)
- `.forgejo/workflows/ci.yml` (new)
- `.forgejo/workflows/release.yml` (new)
- `flake.nix` (new; crane-based)
- `.gitignore` (new; standard Rust)

## Pitfalls

- **`crates.io` publish is non-reversible.** Yank works, but the
  version number is consumed forever. Use the chaperone; don't
  improvise.
- **First version should be `0.1.0`, not `0.0.1` or `1.0.0`.**
  `0.1.0` signals "pre-1.0; expect changes" while remaining a
  Cargo-installable version. `1.0.0` implies an API stability
  commitment we shouldn't make yet.
- **Embedded migrations vs runtime schema files.** Phase 01's
  original design loaded SQL from disk. Step 2 changes this. The
  acceptance test from Phase 01 (round-trip on a fresh db) must
  still pass with the embedded version.
- **Cargo `include`/`exclude` is a trap.** Forgetting to include the
  schema SQL files means the published binary can't run migrations
  on a fresh install. Test by running `cargo package`, extracting
  the tarball, and verifying the `.sql` files are inside.
- **`rust-version` (MSRV).** Set deliberately; bumping it after
  publish is a minor-version bump. If you don't know, pin to the
  current stable Rust at publish time and bump as deps require.
- **License files must be the canonical boilerplate.** Don't
  hand-edit. The `rust-crate-legal-readme` skill bootstraps them.
- **Codeberg runner label.** The CI workflow runs on `atlas`. If
  you accidentally leave a `runs-on: ubuntu-latest` in any job,
  Codeberg's shared runners pick it up — works but defeats the
  point. Grep the workflows for non-`atlas` labels.
- **`cargo audit` / `cargo deny`** may flag rusqlite or its
  transitive deps. Triage: if the advisory is critical, switch
  versions or pin; if low-severity, document the exception in
  `deny.toml`.
- **Don't delete ai-skills' `src/` in this phase.** That repo
  needs to keep building (its catalog/config tooling lives there
  too). Phase 09 handles the consumption flip.
- **The chaperone may catch real blockers.** That's the point.
  Resist the urge to disable or skip a gate to get to publish
  faster.
- **`docs.rs` builds the published tarball in a sandbox.** If the
  build needs a non-standard feature (some `rusqlite` configs
  do), add a `[package.metadata.docs.rs]` section in Cargo.toml.

## Reference

- Parent plan: `docs/planning/calibration-loop/README.md`.
- Phases this phase ships (the CLI surface): `01`, `02`, `03`, `04`.
- Phase that consumes the published crate via HM module: `08-nix-hm-module.md`.
- Phase that points ai-skills at the published crate:
  `09-ai-skills-consumption.md`.
- Release skills to invoke:
  - `rust-crate-release-prep`
  - `rust-crate-release-chaperone`
  - `rust-crate-manifest-metadata`
  - `rust-crate-legal-readme`
  - `rust-crate-rustdoc`
  - `rust-crate-nix-release-tooling`
  - `rust-crate-quality-gates`
  - `rust-crate-forgejo-release-ci`
  - `rust-crate-publish-workflow`
  - `forgejo-atlas-ci`
  - `simit-project-init` (alternative if simit is preferred)
  - `mvp2prod` (general repo bootstrap)
  - `berg-codeberg-ci` (Codeberg CI troubleshooting).
- crates.io publish docs: <https://doc.rust-lang.org/cargo/reference/publishing.html>.
