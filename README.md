# skillnet

`skillnet` is a CLI for reconciling local AI skill directories into a checked-in mirror, editing mirrored skills, syncing selected mirror state back to live agent directories, and recording calibration data for `multi-phase-plan`.

The supported interface in `0.1.0` is the `skillnet` binary. This crate does not commit to a stable embeddable Rust API yet.

## Install

```sh
cargo install skillnet
```

For a source checkout:

```sh
cargo install --path .
```

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
