# Release And Maintenance

Release validation for this crate uses the same checks documented in the repository root:

```sh
simit init flake --check --diff
simit release trust check
simit init ci --platform github --check --diff
nix flake check --keep-going --print-build-logs
treefmt --ci
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

The canonical source repository is `https://github.com/caniko/skillnet`. Generated
GitHub workflows live under `.github/workflows/`; change their inputs in
`simit.toml` and regenerate with the pinned Simit generator, rather than editing
generated YAML.

## Pages deployment prerequisite

PR CI qualifies source and the site build; it does not provision a Pages site,
enable publishing, or migrate DNS. Before enabling deployment, configure the
repository's Pages source as GitHub Actions and set its custom domain to
`skillnet.tartanoglu.com`. Confirm that `ci.pages.source_branch` in `simit.toml`
matches the intended publishing branch, then regenerate the workflow. The
current explicit setting is `main`; merging into `trunk` alone does not trigger
that Pages workflow.

The build's `.domains` file checks the intended domain but does not configure
GitHub Pages. GitHub's [custom-domain documentation](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/managing-a-custom-domain-for-your-github-pages-site)
states that custom Actions publishing ignores `CNAME` files. Configure and verify
the domain through repository Pages settings before deployment; adding a CNAME
file to the artifact is not a substitute. Pages provisioning and DNS changes
require separate operator authorization.
