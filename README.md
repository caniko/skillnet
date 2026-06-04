# skillnet

`skillnet` is a CLI for managing canonical AI skill stores, materialising derived agent views, and recording calibration data for `multi-phase-plan`.

The supported interface in `0.6.0` is the `skillnet` binary. This crate does not commit to a stable embeddable Rust API yet.

For project scopes, the canonical store is
`<mirror_root>/projects/<name>`. The default project-local working copy is
`<project>/.agents/skills`, hardlinked from canonical, and the default Claude
view is `<project>/.claude/skills` as per-skill relative symlinks into
`.agents/skills`. Legacy `.skills` migration steps are documented in
`docs/src/migration/agents-canonical.md`.

## Install

```sh
cargo install skillnet
```

For a source checkout:

```sh
cargo install --path .
```

## Hook ingestion

`skillnet hook install` wires Claude Code to record skill invocations through
`skillnet hook ingest`, storing rows in `skill_invocations`. Use
`skillnet hook status` to confirm the managed hook entries are installed. See
the mdBook chapter at [docs/src/hook-ingestion.md](docs/src/hook-ingestion.md)
for Postgres, SQLite, and CLI setup.

### Nix Home Manager

Add the flake input and import the module:

```nix
inputs.skillnet.url = "git+ssh://git@codeberg.org/caniko/skillnet.git";

# In your Home Manager config:
imports = [ inputs.skillnet.hmModules.default ];
programs.skillnet = {
  enable = true;
  settings = {
    global = {
      views = [
        { label = "claude"; path = "/home/alice/.claude/skills"; scope = "global"; }
        { label = "agents"; path = "/home/alice/.agents/skills"; scope = "global"; }
      ];
    };
  };
  mirrorRoot = "/home/alice/skills-mirror";
  database = {
    backend = "postgres";
    urlFile = config.age.secrets.skillnet-pg-url.path;
  };
};
```

The module is exported as both `hmModules.default` and `hmModules.skillnet`.
It installs `skillnet` on `PATH`. When `settings` is declared, the module
renders `$XDG_CONFIG_HOME/skillnet/skillnet.toml`, which the CLI discovers by
default. When `catalogSettings` is declared, it renders
`$XDG_CONFIG_HOME/skillnet/skillnet.catalog.toml`. `skillsRoot`, `mirrorRoot`, and the
declarative database options are folded into generated `skillnet.toml`, so a
normal declarative Home Manager install does not depend on shell-specific env
imports. Without `settings`, you can still drop your own TOMLs and point
`programs.skillnet.configFile` at them. When SQLite is selected, the module
exports `skillnet_DATA_DIR` and `SKILLNET_DATA_DIR` for compatibility; the CLI
creates runtime database directories when commands need them.

If you also want the module to define where the `ai-skills` checkout lives,
set `skillsRoot`. On atlas, that path is:

```nix
programs.skillnet.skillsRoot = "/data/nvme0/can/Projects/ai-skills";
```

When configured, the module writes `skills_root` into `skillnet.toml`, exports
`AI_SKILLS_REPO` for compatibility. `skillsRoot` points at the skills checkout
and VCS working tree; `dataDir` remains skillnet's runtime database location.

Postgres is the default calibration backend and requires a connection URL.
SQLite is also supported by selecting it explicitly:

```nix
programs.skillnet = {
  enable = true;
  package = inputs.skillnet.packages.${pkgs.system}.skillnet;

  database = {
    backend = "sqlite";
  };
};
```

The Postgres backend is included in default builds. If you override
`programs.skillnet.package` with a custom `--no-default-features` build, choose
a package that includes the `postgres` feature when using the Postgres backend.

The `database.url` value is written into the Nix store. For production secrets,
prefer `database.urlFile`, which reads the Postgres URL from a file at shell
initialization time.

Home Manager does not run `skillnet` commands during activation. After a rebuild
or `home-manager switch`, run materialisation and hook workflows explicitly:

```sh
skillnet sync
skillnet hook install
```

Options:

- `programs.skillnet.dataDir` defaults to `${config.xdg.dataHome}/skillnet`.
  This is skillnet's runtime database location.
- `programs.skillnet.skillsRoot` sets `skills_root` in generated
  `skillnet.toml`, exports `AI_SKILLS_REPO` for compatibility, and is the
  canonical mirror destination/VCS working tree; atlas uses
  `/data/nvme0/can/Projects/ai-skills`.
- `programs.skillnet.mirrorRoot` is the legacy mirror destination option. When
  both `skillsRoot` and `mirrorRoot` are set, they must match.
- `programs.skillnet.database.backend` selects `sqlite` or `postgres` and
  defaults to `postgres`.
- `programs.skillnet.database.path` optionally sets the SQLite database path;
  when unset, calibration data lives at
  `<dataDir>/multi-phase-plan/calibration.sqlite`.
- `programs.skillnet.database.url` is written into generated `skillnet.toml`
  when `settings` is declared, or exported as `SKILLNET_DATABASE_URL` for
  user-managed config.
- `programs.skillnet.database.urlFile` reads a Postgres URL from a file at
  shell initialization time.
- `programs.skillnet.settings` and `programs.skillnet.catalogSettings` render
  `skillnet.toml` and `skillnet.catalog.toml` from Nix.
- `programs.skillnet.configFile` and `programs.skillnet.catalogConfigFile`
  point the CLI at user-managed TOML files.
- `programs.skillnet.package` overrides the package. If `pkgs.skillnet` is not
  available in your package set, use
  `inputs.skillnet.packages.${pkgs.system}.skillnet`.

## Storage backends

Postgres is the default calibration backend:

```toml
[database]
url = "postgres://skillnet@localhost/skillnet"
```

SQLite is available by selecting it explicitly:

```toml
[database]
backend = "sqlite"
path = "/home/alice/.local/share/skillnet/multi-phase-plan/calibration.sqlite"
```

The `path` key is optional. Without it, `skillnet` uses
`$skillnet_DATA_DIR/multi-phase-plan/calibration.sqlite`,
`$SKILLNET_DATA_DIR/multi-phase-plan/calibration.sqlite`, or
`$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`.

Select Postgres with an environment variable:

```sh
export SKILLNET_DATABASE_URL='postgres://skillnet@localhost/skillnet'
skillnet calibration migrate
```

Or with `skillnet.toml`:

```toml
[database]
url = "postgres://skillnet@localhost/skillnet"
```

Or through the Home Manager module:

```nix
programs.skillnet = {
  enable = true;
  package = inputs.skillnet.packages.${pkgs.system}.skillnet;
  database = {
    backend = "postgres";
    urlFile = config.age.secrets.skillnet-pg-url.path;
  };
};
```

`--database-url <URL>` overrides both environment and config for one command.
Plain URLs in `programs.skillnet.database.url` are written into the Nix store;
use `programs.skillnet.database.urlFile` with `sops-nix`, `agenix`, or another
secret-backed file for production credentials.

## Quick Start

Inspect configured scopes and current view drift:

```sh
skillnet status
skillnet scope list
```

Materialise derived views and then inspect or regenerate catalog output:

```sh
skillnet sync
skillnet skill list --scope global
skillnet catalog generate
```

`skillnet sync` is dry-run-on-conflict for promotion candidates: a real
view-side directory newer than canonical prints `would promote ...` and exits 2. Rerun with `--apply-promote` to pull that content into canonical. See
[docs/src/commands.md](docs/src/commands.md) for the full flag table.

Calibration commands are available under the dedicated command group:

```sh
skillnet calibration analyze --format table
skillnet calibration proposals --pending
```

## Development

Run the default test suite with:

```sh
cargo test --all-targets
```

Postgres parity tests run when `SKILLNET_TEST_PG_URL` points at a test database:

```sh
export SKILLNET_TEST_PG_URL='postgres://skillnet@localhost/skillnet_test'
cargo test-pg
```

## Configuration Model

`skillnet` keeps canonical skill stores separate from generated agent views:

- `global/` stores the canonical global skills by default.
- Project canonical stores live under `<mirror_root>/projects/<name>`.
- Project-local working copies live at each project's `canonical_rel`,
  defaulting to `.agents/skills`, and are materialised from canonical.
- Global and project views such as `.claude/skills` are generated symlink
  views. Project views use per-skill relative symlinks into the project
  working copy.
- Link strategy defaults are `symlink` for global scopes and `hardlink` for
  project working copies. Set top-level `link_strategy`, per-project
  `link_strategy`, or pass `--link symlink|hardlink` to materialisation
  commands to override a run.

Hardlink project working copies require the project checkout and mirror checkout to live
on the same filesystem. Cross-device hardlink failures are reported as errors;
skillnet does not silently fall back to copying or symlinking. If a Git pull or
editor rewrite severs a hardlink but leaves matching content, `skillnet sync`
or `skillnet project sync` re-links it. Project working copies are generated
from canonical and are replaced when they drift.

Configuration lives in `$XDG_CONFIG_HOME/skillnet/skillnet.toml`. Catalog
metadata uses `$XDG_CONFIG_HOME/skillnet/skillnet.catalog.toml`. The legacy
cwd fallbacks `./skillnet.toml` and `./skillnet.catalog.toml` still work in
`0.6.x` with a deprecation warning and are scheduled for removal in `0.7.0`.
Run `skillnet config migrate` from the old config directory to centralise
existing files.

## Command Surface

The current top-level commands are:

- `status`
- `completions`
- `sync`
- `view`
- `skill`
- `scope`
- `project`
- `catalog`
- `calibration`

Many scope-aware commands accept `--scope <name>`. The selector
`--scope projects` expands to every configured project, and `--scope all`
selects global plus every project. `skillnet sync --all` is an alias for
`skillnet sync --scope all`.

Generate shell completions with:

```sh
skillnet completions bash
skillnet completions zsh
skillnet completions fish
skillnet completions elvish
skillnet completions powershell
```

## Documentation

- Docs: <https://docs.rs/skillnet>
- Source: <https://codeberg.org/caniko/skillnet>

## Release Validation

The release-prep flow validates the repository with:

```sh
simit init flake --check --diff
simit release trust check
simit init ci --platform forgejo --check --diff
nix flake check --keep-going --print-build-logs
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- --deny warnings
cargo test --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo package --list
cargo publish --dry-run
```

## License

Licensed under either of:

- MIT ([LICENSE-MIT](LICENSE-MIT))
- Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE))
