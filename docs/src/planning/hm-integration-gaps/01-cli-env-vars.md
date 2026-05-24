# Phase 01 — CLI env-var hooks for config paths

> **Recommended Codex model: GPT 5.5 low**
>
> Mechanical clap edit: add `env = "…"` to three global args and update a
> couple of unit tests. No design content, no log interpretation. A small
> model will not miss anything important here. Promote to `medium` only if
> the clap version in `Cargo.toml` is older than 4.x and the `env` feature
> has to be enabled (a one-line `Cargo.toml` change).

## Working tree

Same repo: `/data/nvme0/can/Projects/skillnet`.

## Goal

`skillnet --help` shows that `--config`, `--catalog-config`, and
`--mirror-root` each read a corresponding environment variable as a
fallback when the flag is not given. With `SKILLNET_CONFIG=/abs/path.toml`
exported, `skillnet status` no longer requires `./skillnet.toml` in the
cwd.

## Why this matters now

The HM module installs the binary into `/etc/profiles/per-user/<user>/bin`,
so the user runs it from any directory. Today the very first thing every
subcommand does is read `./skillnet.toml` relative to cwd, which causes:

```
$ skillnet status
Error: failed to read config file skillnet.toml

Caused by:
    No such file or directory (os error 2)
```

Without an env-var hook, the HM module cannot point the binary at user
TOMLs. Phase 02 depends on the env var names this phase introduces.

## Out of scope

- Writing the TOML files (phase 03).
- Touching the HM module (phase 02).
- Renaming or removing `--config` / `--catalog-config` flags. The flag
  remains the highest-precedence source; the env var is a fallback.
- Adding `SKILLNET_DRY_RUN` or other env hooks for non-path flags.

## Plan

1. Open [src/cli/args.rs](../../../../src/cli/args.rs). For each of the
   three global args, add the matching `env =` attribute. Clap supports
   this directly when the `env` feature is enabled. Concretely:

   ```rust
   #[arg(long, env = "SKILLNET_CONFIG", default_value = "skillnet.toml", global = true)]
   pub(super) config: Utf8PathBuf,

   #[arg(long, env = "SKILLNET_MIRROR_ROOT", default_value = ".", global = true)]
   pub(super) mirror_root: Utf8PathBuf,

   #[arg(long, env = "SKILLNET_CATALOG_CONFIG", default_value = "skillnet.catalog.toml", global = true)]
   pub(super) catalog_config: Utf8PathBuf,
   ```

2. Verify clap's `env` feature is enabled: `rg '^clap' Cargo.toml`. If the
   dependency is declared as plain `clap = { version = "4", features = ["derive"] }`,
   add `"env"` to the features list. Run `cargo build --no-default-features`
   if that's a supported build, to confirm we haven't broken the feature
   gating story.

3. Confirm precedence: explicit `--config` flag wins over `SKILLNET_CONFIG`
   which wins over the literal default `skillnet.toml`. This is clap's
   built-in ordering; no extra code required.

4. Add (or extend) a small unit test under `src/cli/args.rs` (or wherever
   args tests live — `rg 'fn .*parse.*Cli' src/`) that:
   - Sets `SKILLNET_CONFIG=/tmp/foo.toml` via `temp_env::with_var` (or
     manual `std::env::set_var` with a serial-test guard if the suite
     doesn't already use `temp_env`), parses `Cli::parse_from(["skillnet",
     "status"])`, and asserts `cli.config == "/tmp/foo.toml"`.
   - Asserts that passing `--config /tmp/bar.toml` *and* the env var
     resolves to `/tmp/bar.toml` (flag beats env).
   - Same for `SKILLNET_CATALOG_CONFIG` and `SKILLNET_MIRROR_ROOT`.

5. Run `cargo test -p skillnet` and `cargo fmt`. Then `cargo clippy
   --all-targets -- -D warnings`.

## Acceptance criteria

- [ ] `skillnet --help` output includes `[env: SKILLNET_CONFIG=]`,
  `[env: SKILLNET_CATALOG_CONFIG=]`, and `[env: SKILLNET_MIRROR_ROOT=]`
  next to the respective flag descriptions.
- [ ] With `SKILLNET_CONFIG=/abs/skillnet.toml` exported and the file
  present, `skillnet status` invoked from `/tmp` reads that file (verified
  manually or via the new unit test).
- [ ] Explicit `--config <path>` still overrides the env var.
- [ ] `cargo test -p skillnet`, `cargo fmt --check`, and `cargo clippy
  --all-targets -- -D warnings` are all clean.
- [ ] No change in behaviour when neither flag nor env var is set: the
  default `skillnet.toml` cwd resolution is preserved.

## Files likely touched

- [src/cli/args.rs](../../../../src/cli/args.rs) — three `env =` attributes,
  optional test module.
- `Cargo.toml` — possibly add `"env"` to clap features.
- A new or extended `#[cfg(test)] mod tests` block (in `args.rs` or
  `src/cli/mod.rs`).

## Pitfalls

- **`env` feature not enabled on clap.** Symptom: compile error on the
  `env =` attribute. Cause: feature gated behind `clap = { features = [...,
  "env"] }`. Recovery: add `"env"` to the features list in `Cargo.toml`
  and re-run `cargo build`.
- **Test contamination across threads.** Symptom: a flake where the test
  passes alone but fails in the suite because another test sets/unsets the
  same env var. Cause: `std::env::set_var` is process-wide. Recovery: use
  `temp_env::with_var` (already common in this codebase if you grep for
  it) or `#[serial]` from `serial_test`.
- **Default-value precedence surprise.** Clap treats `default_value` as the
  fallback when *neither* the flag nor the env var is set — that's what we
  want. Do **not** add `default_value_if` or `required` thinking it will
  enforce the env var; it won't, and it'll break cwd-based users.

## Reference

- Originating diagnosis: README of this plan set ("CLI does not read
  `SKILLNET_CONFIG`").
- Clap docs on env arg source:
  https://docs.rs/clap/latest/clap/_derive/index.html#arg-attributes (look
  for `env`).
- Phase 02 ([02-hm-env-and-fixes.md](./02-hm-env-and-fixes.md)) consumes
  the env var names introduced here.
