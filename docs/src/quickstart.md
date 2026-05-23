# Quick Start

Install from crates.io:

```sh
cargo install skillnet
```

Common first commands:

```sh
skillnet status
skillnet scope list
skillnet scope sources
skillnet sync pull --scope global
skillnet skill list --scope global
skillnet catalog generate
```

`skillnet.toml` defines the mirror scopes and live source directories. `skillnet.catalog.toml` defines catalog metadata used by `catalog generate` and `catalog lint`.
