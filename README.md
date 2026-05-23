# skillnet

`skillnet` is a CLI for reconciling local AI skill directories into a checked-in mirror, editing mirrored skills, syncing selected mirror state back to live agent directories, and recording calibration data for `multi-phase-plan`.

The supported interface in `0.2.0` is the `skillnet` binary. This crate does not commit to a stable embeddable Rust API yet.

## Install

```sh
cargo install skillnet
```

For a source checkout:

```sh
cargo install --path .
```

### Nix Home Manager

Add the flake input and import the module:

```nix
inputs.skillnet.url = "git+ssh://git@codeberg.org/caniko/skillnet.git";

# In your Home Manager config:
imports = [ inputs.skillnet.hmModules.default ];
programs.skillnet.enable = true;
```

The module is exported as both `hmModules.default` and `hmModules.skillnet`.
It installs `skillnet` on `PATH`, creates the runtime data directory, and
exports `skillnet_DATA_DIR` and `SKILLNET_DATA_DIR` for the CLI.

SQLite is the default calibration backend. To use Postgres, select the backend
and provide a connection URL:

```nix
programs.skillnet = {
  enable = true;
  package = inputs.skillnet.packages.${pkgs.system}.skillnet;

  database = {
    backend = "postgres";
    url = "postgres://user:password@db.example.com/skillnet";
  };
};
```

The Postgres backend requires a `skillnet` package built with the `postgres`
feature. The module does not rewrite `programs.skillnet.package`; choose a
package that matches the backend you enable.

The `database.url` value is written into the Nix store. For production secrets,
prefer setting `SKILLNET_DATABASE_URL` through your usual secret mechanism, such
as `sops-nix`, `agenix`, or a shell-sourced environment file.

Options:

- `programs.skillnet.dataDir` defaults to `${config.xdg.dataHome}/skillnet`.
- `programs.skillnet.database.backend` selects `sqlite` or `postgres` and
  defaults to `sqlite`.
- `programs.skillnet.database.path` optionally sets the SQLite database path;
  when unset, calibration data lives at
  `<dataDir>/multi-phase-plan/calibration.sqlite`.
- `programs.skillnet.database.url` sets `SKILLNET_DATABASE_URL` and is required
  when `programs.skillnet.database.backend = "postgres"`.
- `programs.skillnet.package` overrides the package. If `pkgs.skillnet` is not
  available in your package set, use
  `inputs.skillnet.packages.${pkgs.system}.skillnet`.
- `programs.skillnet.extraConfig` is reserved for future declarative config.

## Storage backends

SQLite is the default calibration backend and needs no configuration:

```toml
[database]
backend = "sqlite"
path = "/home/alice/.local/share/skillnet/multi-phase-plan/calibration.sqlite"
```

The `path` key is optional. Without it, `skillnet` uses
`$skillnet_DATA_DIR/multi-phase-plan/calibration.sqlite`,
`$SKILLNET_DATA_DIR/multi-phase-plan/calibration.sqlite`, or
`$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`.

Postgres support is optional and requires a binary built with the `postgres`
feature:

```sh
cargo install skillnet --features postgres
```

Select Postgres with an environment variable:

```sh
export SKILLNET_DATABASE_URL='postgres://skillnet@localhost/skillnet'
skillnet calibration migrate
```

Or with `skillnet.toml`:

```toml
[database]
backend = "postgres"
url = "postgres://skillnet@localhost/skillnet"
```

Or through the Home Manager module:

```nix
programs.skillnet = {
  enable = true;
  package = inputs.skillnet.packages.${pkgs.system}.skillnet;
  database = {
    backend = "postgres";
    url = "postgres://skillnet@localhost/skillnet";
  };
};
```

`--database-url <URL>` overrides both environment and config for one command.
Plain URLs in `programs.skillnet.database.url` are written into the Nix store;
use `sops-nix`, `agenix`, or another secret-backed environment mechanism for
production credentials.

## Quick Start

Inspect the configured scopes and current divergence:

```sh
skillnet status
skillnet scope list
skillnet scope sources
```

Pull live skills into the mirror and then inspect or regenerate catalog output:

```sh
skillnet sync pull --scope global
skillnet skill list --scope global
skillnet catalog generate
```

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

`skillnet` keeps the mirror separate from live agent directories:

- `global/` stores the reconciled global skill mirror.
- `projects/<name>/` stores reconciled per-project mirrors.
- Live global sources typically come from `~/.agents/skills`, `~/.claude/skills`, and `~/.codex/skills`.
- Project scopes can add `.agents/skills`, `.claude/skills`, `.codex/skills`, root `skills`, plugin skill directories, and other configured paths.

Configuration lives in `skillnet.toml`. Catalog metadata lives in `skillnet.catalog.toml`.

## Command Surface

The current top-level commands are:

- `status`
- `completions`
- `sync`
- `skill`
- `scope`
- `project`
- `catalog`
- `calibration`

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
