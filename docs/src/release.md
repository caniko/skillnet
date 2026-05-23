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

The canonical source repository is `https://codeberg.org/caniko/skillnet`. Generated Forgejo workflows live under `.forgejo/workflows/`.
