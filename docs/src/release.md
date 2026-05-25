# Release And Maintenance

Release validation for this crate uses the same checks documented in the repository root:

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

The release tag must be an exact SemVer version and must match
`Cargo.toml`'s package version. The publish workflow verifies the signed tag
against `keys/maintainers.gpg`, checks that the version is not already present
on crates.io, runs the release checks, and then publishes with
`CRATES_IO_API_TOKEN` or `CARGO_REGISTRY_TOKEN`.

The canonical source repository is `https://codeberg.org/caniko/skillnet`. Generated Forgejo workflows live under `.forgejo/workflows/`.
