# Phase 03 — Config centralisation (`skillnet config migrate` + deprecation)

> **Recommended Codex model: GPT 5.5 medium**
>
> New subcommand with a six-row decision table over (legacy-cwd present
> × XDG present × content equal) plus a small deprecation warning at
> the discovery fallthrough point. The decision table is pinned by
> design § 6 and the discovery ladder already exists in
> `src/config.rs`. Sub-agent role: implements a contained workflow
> (read paths, classify state, mutate files, write breadcrumb) without
> touching the rest of the CLI. Moderate complexity from the matrix
> over file-state combinations, but no architectural decisions.
> 5.5 medium is the right tier.

## Working tree

`/data/nvme0/can/Projects/skillnet`. **Must start after Phase 02 has
landed on `main`** because both phases edit `src/cli/args.rs` and
`src/cli/mod.rs`. Rebase before starting.

## Goal

`skillnet config migrate` exists and implements the design § 6
decision table. The legacy-cwd config discovery rank (rank 4 of the
ladder in `src/config.rs`) prints a deprecation warning to stderr
exactly once per invocation when it fires.

## Why this matters now

The user's live configs at
`/data/nvme0/can/Projects/ai-skills/skillnet.toml` and
`/data/nvme0/can/Projects/ai-skills/skillnet.catalog.toml` only
resolve when `skillnet` is invoked from inside that directory. After
the user runs `skillnet config migrate`, the configs live at
`$XDG_CONFIG_HOME/skillnet/` and resolve from any cwd. This unlocks
the HM module work in Phase 04: declaring
`programs.skillnet.settings` writes the same content under the same
XDG path, and the CLI picks it up.

The deprecation warning is the announcement vector for the `0.7.0`
removal. Without it, users on `0.6.0` who keep using the cwd
fallthrough get no warning that their workflow is about to break.

## Out of scope

- Removing the legacy-cwd rank from the discovery ladder. That is
  `0.7.0` work, explicitly per design § 12.
- Auto-running `skillnet config migrate` from any other command.
  The user runs it once, deliberately.
- Detection of HM-managed configs as "do not migrate". Phase 04
  documents this in the HM quickstart; the CLI's HM-managed
  detection (design § 7) is in Phase 04, not here.
- Migration of any file other than `skillnet.toml` and
  `skillnet.catalog.toml`. Calibration databases, hook settings,
  etc. are not in scope.
- Changes to other CLI subcommands. This phase adds one new
  subcommand variant; the existing variants are untouched.

## Plan

1. **Read inputs.** Open
   [design § 6 `skillnet config migrate`](../two-way-sync-and-config-centralisation-research.md#6-skillnet-config-migrate)
   and [§ 12 Migration and release sequencing](../two-way-sync-and-config-centralisation-research.md#12-migration-and-release-sequencing).
   Open [src/config.rs](../../../src/config.rs), specifically:
   - `default_config_path` / `default_catalog_config_path` / `legacy_config_path` / `legacy_catalog_config_path` (lines 440-462).
   - `default_xdg_config_path` (lines 456-462).

   Open [src/cli/args.rs](../../../src/cli/args.rs) and re-read the
   `Command` enum to understand the existing subcommand pattern
   (e.g., `Project { command: ProjectCommand }`).

   Open [src/cli/mod.rs](../../../src/cli/mod.rs) and re-read
   `resolve_config_path` / `resolve_catalog_config_path` (lines
   102-138). The deprecation warning hooks into the rank-4 branch
   of those functions.

2. **Add the `Config` subcommand to `Command`.** In
   `src/cli/args.rs`, add a new variant after `Sync`:

   ```rust
   /// Manage skillnet's configuration files.
   Config {
       #[command(subcommand)]
       command: ConfigCommand,
   },
   ```

   Add the subcommand enum:

   ```rust
   #[derive(Debug, Subcommand)]
   #[command(disable_help_subcommand = true)]
   pub(super) enum ConfigCommand {
       /// Move skillnet.toml and skillnet.catalog.toml from the
       /// legacy working-directory location to $XDG_CONFIG_HOME/skillnet/.
       Migrate {
           /// Print decisions without touching the filesystem.
           #[arg(long)]
           dry_run: bool,
           /// Overwrite XDG when both locations exist and differ.
           #[arg(long)]
           force: bool,
           /// Delete .skillnet.toml.moved-to-xdg breadcrumb files at the
           /// rank-4 discovery location, if present, then exit.
           #[arg(long)]
           remove_breadcrumbs: bool,
       },
   }
   ```

   Notes:
   - `--remove-breadcrumbs` is mutually exclusive with the actual
     migrate behaviour. Use `conflicts_with_all = ["dry_run", "force"]`
     on it.
   - The subcommand operates on cwd, not on `--config` / `--catalog-config`
     overrides. Document this in the help string for `Migrate`.

3. **Add `Command::Config` arm to `cli::run`.** In `src/cli/mod.rs`,
   add the arm after `Command::Sync`:

   ```rust
   Command::Config { command } => {
       commands::config::run(command, dry_run)
   }
   ```

   The `Config` subcommand does **not** need a full `Context` — it
   operates on raw paths and does not load the skillnet config (it
   *is* the skillnet config). Bypass `Context::load`.

4. **Create `src/commands/config.rs`.** Module skeleton:

   ```rust
   use std::fs;
   use anyhow::{bail, Context as AnyhowContext, Result};
   use camino::{Utf8Path, Utf8PathBuf};
   use crate::cli::args::ConfigCommand;
   use crate::config::{
       default_config_path, default_catalog_config_path,
       legacy_config_path, legacy_catalog_config_path,
   };

   pub fn run(command: ConfigCommand, global_dry_run: bool) -> Result<()> {
       match command {
           ConfigCommand::Migrate { dry_run, force, remove_breadcrumbs } => {
               let dry_run = dry_run || global_dry_run;
               if remove_breadcrumbs {
                   return remove_breadcrumb_files(dry_run);
               }
               migrate_one("skillnet.toml", &legacy_config_path(), &default_config_path()?, dry_run, force)?;
               migrate_one("skillnet.catalog.toml", &legacy_catalog_config_path(), &default_catalog_config_path()?, dry_run, force)?;
               Ok(())
           }
       }
   }
   ```

   The `migrate_one` function implements the design § 6 decision
   table for one file at a time:

   ```rust
   enum MigrateAction {
       NoLegacyAbsent,            // legacy absent, XDG absent — no-op
       AlreadyCentralised,        // legacy absent, XDG present — no-op
       MoveLegacyToXdg,           // legacy present, XDG absent — move
       DeleteLegacyEqualContent,  // both present, content equal — delete cwd
       RefuseDifferent,           // both present, content differs — error unless --force
       ForceOverwrite,            // both present, content differs, --force — overwrite
   }

   fn classify(legacy: &Utf8Path, xdg: &Utf8Path, force: bool) -> Result<MigrateAction>;
   fn migrate_one(file_name: &str, legacy: &Utf8Path, xdg: &Utf8Path, dry_run: bool, force: bool) -> Result<()>;
   ```

   In `classify`:
   - Use `fs::read` for both files when both exist; compare with
     `sha2::Sha256` (already a dep) or raw byte comparison
     (cheaper). Raw byte comparison is fine; configs are small.
   - The "content equal" check must be byte-equal, not "parsed
     TOML semantically equal". Two whitespace-different files are
     not equal; the user can re-run after normalising.

   In `migrate_one`:
   - On `NoLegacyAbsent`: print
     `"<file_name>: no config to migrate"` and return `Ok(())`.
   - On `AlreadyCentralised`: print
     `"<file_name>: already centralised at <xdg>"` and return.
   - On `MoveLegacyToXdg`:
     - If `dry_run`: print
       `"<file_name>: would move <legacy> -> <xdg>"` and return.
     - Otherwise: `fs::create_dir_all(xdg.parent().unwrap())`,
       `fs::rename(legacy, xdg)` (atomic if same filesystem),
       then write the breadcrumb at
       `<legacy_parent>/.<file_name>.moved-to-xdg` with content
       `<xdg>\n`. Print `"<file_name>: moved to <xdg>"`.
   - On `DeleteLegacyEqualContent`:
     - If `dry_run`: print
       `"<file_name>: would delete <legacy> (XDG copy is byte-identical)"`.
     - Otherwise: `fs::remove_file(legacy)`, then write the
       breadcrumb. Print
       `"<file_name>: deleted <legacy>; XDG copy retained at <xdg>"`.
   - On `RefuseDifferent`:
     - `bail!("<file_name>: both <legacy> and <xdg> exist and differ; pass --force to overwrite XDG with legacy contents")`.
     - Exit code 1.
   - On `ForceOverwrite`:
     - If `dry_run`: print
       `"<file_name>: would overwrite <xdg> with <legacy> contents (--force)"`.
     - Otherwise: `fs::create_dir_all(xdg.parent().unwrap())`,
       `fs::rename(legacy, xdg)`, write breadcrumb.

   `remove_breadcrumb_files`:
   - For each of the two breadcrumb paths
     (`<legacy_parent>/.skillnet.toml.moved-to-xdg` and
     `<legacy_parent>/.skillnet.catalog.toml.moved-to-xdg`):
     - If absent: no-op (silent or one-line note, your call —
       prefer a one-line note for grep-ability).
     - If present and `!dry_run`: `fs::remove_file`. Print
       `"removed breadcrumb <path>"`.
     - If present and `dry_run`: print `"would remove breadcrumb <path>"`.

5. **Register the new module.** Add `pub mod config;` to
   [src/commands/mod.rs](../../../src/commands/mod.rs).

6. **Add the deprecation warning.** In `src/cli/mod.rs`, in
   `resolve_config_path` and `resolve_catalog_config_path`, when
   the resolver falls through to rank 4 (legacy cwd, found and
   used), print exactly once to stderr:

   ```
   warning: using legacy working-directory config at <path>;
            this discovery path is deprecated and will be removed in skillnet 0.7.0.
            Run `skillnet config migrate` to move it to $XDG_CONFIG_HOME/skillnet/.
   ```

   "Exactly once per invocation" — the warning fires once for the
   skillnet config and once for the catalog config if both fall
   through, which is fine. Do not gate behind an env var; the
   warning is the announcement vector and silencing it would
   defeat the purpose.

   Implementation: just add the `eprintln!` after the
   `legacy.exists()` branch returns `Ok(legacy)`. Be careful: the
   resolver currently returns early on the legacy branch only if
   the legacy file exists, so the warning is correctly
   conditioned on "we actually used the legacy path".

7. **Tests.** Add `tests/config_migrate.rs`:

   - `migrate_noop_when_neither_present`: tempdir with neither
     legacy nor XDG; assert stdout contains "no config to migrate"
     for both files; exit `0`.
   - `migrate_noop_when_only_xdg_present`: tempdir with only XDG;
     assert stdout contains "already centralised"; exit `0`.
   - `migrate_moves_legacy_to_xdg_when_xdg_absent`: tempdir with
     legacy only; run migrate; assert legacy is gone, XDG has the
     content, breadcrumb is written; exit `0`.
   - `migrate_deletes_legacy_when_xdg_equal`: tempdir with both
     and identical content; assert legacy gone, XDG unchanged,
     breadcrumb written; exit `0`.
   - `migrate_refuses_when_xdg_differs_without_force`: tempdir
     with both differing; assert exit `1`, stderr mentions
     `--force`, neither file mutated.
   - `migrate_overwrites_xdg_with_force`: same fixture with
     `--force`; assert legacy gone, XDG now contains legacy
     content.
   - `migrate_dry_run_makes_no_changes`: each of the six rows
     under `--dry-run`; assert filesystems unchanged.
   - `migrate_remove_breadcrumbs_deletes_them`: tempdir with two
     pre-existing breadcrumbs; assert removal.
   - `deprecation_warning_fires_on_legacy_pickup`: integration
     test via `assert_cmd` — run any read-only skillnet command
     (e.g., `skillnet scope list`) with cwd containing a legacy
     config and no XDG config; assert stderr contains
     `"deprecated"` and `"0.7.0"`.
   - `no_deprecation_warning_on_xdg_pickup`: same command with
     XDG config present and no legacy; assert stderr does not
     contain `"deprecated"`.

   Use `temp_env` (already a dev-dep) to set `XDG_CONFIG_HOME`
   to a tempdir path per test so the migration target is
   controllable.

8. **Run the local check loop.** `cargo fmt`, `cargo clippy
   --all-targets -- -D warnings`, `cargo test --workspace`.

9. **Commit.** One commit, message:
   `feat: skillnet config migrate and legacy-cwd deprecation warning`

## Acceptance criteria

- [ ] `skillnet config migrate --help` lists `--dry-run`,
      `--force`, `--remove-breadcrumbs` and describes that the
      command moves both `skillnet.toml` and
      `skillnet.catalog.toml`.
- [ ] All 10 tests in `tests/config_migrate.rs` pass.
- [ ] Running `skillnet scope list` with cwd containing a legacy
      `skillnet.toml` and no XDG `skillnet.toml` emits the
      deprecation warning to stderr exactly once.
- [ ] Running `skillnet scope list` with both XDG and legacy
      present prefers XDG and does **not** emit the warning
      (rank 3 fires; rank 4 never reached).
- [ ] `cargo test --workspace`, `cargo clippy --all-targets -- -D
      warnings`, `cargo fmt --check` clean.
- [ ] `git log -1` shows the single phase commit.
- [ ] No regression in any other CLI subcommand. All existing
      `tests/cli.rs` cases still pass.

## Files likely touched

- `src/cli/args.rs` — add `Command::Config` variant +
  `ConfigCommand` enum + `Migrate` subcommand.
- `src/cli/mod.rs` — add `Command::Config` arm; add deprecation
  `eprintln!` in `resolve_config_path` and
  `resolve_catalog_config_path`.
- `src/commands/mod.rs` — register `pub mod config;`.
- `src/commands/config.rs` — **new file**.
- `tests/config_migrate.rs` — **new file** with 10 tests.

## Pitfalls

- **S1: `fs::rename` across filesystems.** If `$XDG_CONFIG_HOME`
  and the legacy cwd are on different filesystems (e.g., `/tmp`
  and `~/.config`), `fs::rename` fails with `EXDEV`. The user's
  live setup has both on the same filesystem, but tests in `/tmp`
  may not. Recovery: catch `EXDEV` and fall back to
  `fs::copy + fs::remove_file`. Document the fallback in the
  source comments.
- **S2: Breadcrumb in dirty state.** If the move succeeds but the
  breadcrumb write fails (read-only fs, permission issue), the
  migration is complete but the user has no trail. Make breadcrumb
  failure a warning, not an error — the actual migration succeeded.
  Print to stderr: `"warning: could not write breadcrumb <path>: <err>"`.
- **S3: `$XDG_CONFIG_HOME` not set in tests.** The default falls
  back to `$HOME/.config` per `default_xdg_config_path`. In tests
  using `temp_env`, set `XDG_CONFIG_HOME` *and* `HOME` to tempdir
  paths so the resolver does not write into the user's real
  `~/.config`. The existing `tests/cli.rs` patterns may already do
  this; copy the pattern.
- **S4: Deprecation warning at rank 4 of the catalog resolver
  could double-fire.** Both `resolve_config_path` and
  `resolve_catalog_config_path` independently emit the warning.
  That is correct (each refers to a different file), but
  read carefully: a user might see two warning lines per
  invocation. That is fine; design accepts it.
- **S5: `Config` subcommand without explicit cwd handling.**
  Some users will run `skillnet config migrate` from a
  directory that contains neither legacy nor XDG configs (e.g.,
  their home directory). The "noop on neither present" row of the
  table handles this cleanly. Verify in
  `migrate_noop_when_neither_present`.
- **S6: Subcommand collision with existing `Command::Completions`.**
  Clap's subcommand resolution is alphabetical in help output; the
  new `Config` subcommand sorts before `Doctor`, after `Calibration`,
  etc. That is fine; just be aware that `--help` output order
  changes. The Phase 05 snapshot test pins the final order.
- **S7: Catalog file not always present.** A skillnet user may
  have `skillnet.toml` but no `skillnet.catalog.toml`. The
  per-file decision table runs independently, so the absent
  catalog falls into `NoLegacyAbsent` for that file — no error.
  Verify by writing a test fixture with only one of the two
  files.

## Reference

- Design dossier:
  - [§ 6 `skillnet config migrate`](../two-way-sync-and-config-centralisation-research.md#6-skillnet-config-migrate)
  - [§ 12 Migration and release sequencing](../two-way-sync-and-config-centralisation-research.md#12-migration-and-release-sequencing)
- Existing code:
  - [src/config.rs:440-462](../../../src/config.rs#L440-L462) for
    path resolution.
  - [src/cli/mod.rs:102-138](../../../src/cli/mod.rs#L102-L138) for
    the rank-4 fallthrough.
  - [src/commands/project.rs:22-77](../../../src/commands/project.rs#L22-L77)
    for the "subcommand mutates files" pattern (project add/remove
    is the closest analogue).
