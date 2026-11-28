# skillnet

<!-- simit:badges:start -->

[![CI](https://img.shields.io/badge/CI-managed-2088ff)](.github/workflows/ci.yaml) [![Nix](https://img.shields.io/badge/Nix-drift-5277c3)](flake.nix) [![docs](https://img.shields.io/badge/docs-enabled-6f42c1)](docs) [![crates.io](https://img.shields.io/badge/crates.io-publishable-f46623)](https://crates.io/crates/skillnet) [![artifacts](https://img.shields.io/badge/artifacts-configured-2ea44f)](.github/workflows/release.yml)

<!-- simit:badges:end -->

`skillnet` is a CLI for managing canonical AI skill stores, materialising derived agent views, and recording `multi-phase-plan` calibration data.

Global views can use `link_strategy = "hardlink"` to share immutable skill files across agent roots without duplicating contents. Hardlink views are derived from the canonical mirror; edit skills in the canonical store.

Since `0.6.0`, the supported interface is the `skillnet` binary; no stable embeddable Rust API is promised yet.

Project scopes keep their canonical store at `<project>/.skills`, materialise a hardlinked working copy at `<project>/.agents/skills` by default, and default the Claude view at `<project>/.claude/skills` to per-skill relative symlinks into `.agents/skills`.

## Install

```sh
cargo install skillnet
```

For a source checkout:

```sh
cargo install --path .
```

## Hook ingestion

`skillnet hook install` configures Claude Code to record skill invocations via `skillnet hook ingest` into `skill_invocations`. Run `skillnet hook status` to verify the managed entries. See [docs/src/hook-ingestion.md](docs/src/hook-ingestion.md) for Postgres, SQLite, and CLI setup.

Normalized harness adapters can record directly with:

```sh
skillnet usage record --skill global/example --harness codex \
  --session SESSION_HASH --event-id UNIQUE_EVENT_ID
skillnet usage report --since 90d --format table
```

### Code-triggered cards

```sh
skillnet fingerprint --cwd . --json
skillnet trigger --event post-edit --session "$SESSION_ID" --payload-file edit.json --json
skillnet trigger --event audit --no-stdin --json
skillnet card list --pack rust
skillnet card show rust/unwrap-in-lib --full
skillnet card validate
```

Cards load from `--packs-dir`, `$AI_SKILLS_REPO/packs`, or the `packs/` directory next to the configured global skills source, in that order; the current repository's packs are not auto-discovered. `card show --full` reads only references that resolve to files inside the packs' source tree.

`trigger` reads JSON from stdin or `--payload-file` (for example, `{"file":"src/lib.rs","diff":"+load().unwrap()"}`). Without event details, `audit` selects advice from the fingerprint's recommended packs; pass `--no-stdin` for an empty audit without reading stdin. Defaults cap output at 20 cards for audit and 2 otherwise; JSON reports the count omitted by the budget as `budget_dropped`. Globs match full repo-relative paths using `globset` defaults (`*` can span `/`). Pass `--session` (defaulting to `CLAUDE_SESSION_ID` when set) to persist the advice budget and cooldown in `${XDG_STATE_HOME:-$HOME/.local/state}/skillnet/trigger-state`, or override with `--state-dir`. Callers inspect the JSON `deny` field rather than the exit code to block an action. `card validate` checks positive and negative fixtures without consuming cooldowns or budget.

Usage reports include every current catalog skill, including zero-use entries; a zero count is only evidence of non-use when harness coverage stayed healthy across the full window. Events store metadata only, and source-event IDs make retries idempotent.

Harness integrations such as Infernix own adapter and plugin installation. Skillnet owns canonical skills, materialised views, catalogs, and normalized usage events, not external plugin payloads or harness installation policy.

### Nix Home Manager

Add the flake input and import the module:

```nix
inputs.skillnet.url = "git+ssh://git@github.com/caniko/skillnet.git";

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

Exported as `hmModules.default` and `hmModules.skillnet`, the module puts `skillnet` on `PATH`. Declaring `settings` renders `$XDG_CONFIG_HOME/skillnet/skillnet.toml` (discovered by default) with `skillsRoot`, `mirrorRoot`, and declarative database options folded in, avoiding shell-specific env imports. Declaring `catalogSettings` renders `$XDG_CONFIG_HOME/skillnet/skillnet.catalog.toml`. Without `settings`, you can keep user-managed TOMLs in the standard XDG location or optionally point `programs.skillnet.configFile` and `programs.skillnet.catalogConfigFile` at custom paths. Under SQLite, the module exports `skillnet_DATA_DIR` and `SKILLNET_DATA_DIR` for compatibility; the CLI creates runtime database directories on demand.

To declare the `ai-skills` checkout path, set `skillsRoot` (on atlas):

```nix
programs.skillnet.skillsRoot = "/data/nvme0/can/canix/projects/repos/owned/github.com/caniko/ai-skills";
```

Setting `skillsRoot` writes `skills_root` into `skillnet.toml` and exports `AI_SKILLS_REPO` for compatibility. `skillsRoot` points at the skills checkout and VCS working tree, while `dataDir` holds runtime bundles and local calibration state.

Postgres is the default calibration backend and requires a connection URL. Select SQLite explicitly when needed:

```nix
programs.skillnet = {
  enable = true;
  package = inputs.skillnet.packages.${pkgs.system}.skillnet;

  database = {
    backend = "sqlite";
  };
};
```

Default builds include Postgres support. If you override `programs.skillnet.package` with a `--no-default-features` build, keep the `postgres` feature enabled when using Postgres.

Because `database.url` lands in the Nix store, prefer `database.urlFile` for production secrets so the URL is read from a file at shell initialization time.

Home Manager does not run `skillnet` commands during activation. After a rebuild or `home-manager switch`, run materialisation and hook workflows explicitly:

```sh
skillnet sync
skillnet hook install
```

Options:

- `user`: principal for schema-version-2 manifest access filtering (optional for legacy unrestricted manifests).
- `programs.skillnet.dataDir`: runtime data directory (written as `data_dir` for both SQLite and Postgres bundles); defaults to `${config.xdg.dataHome}/skillnet`.
- `programs.skillnet.skillsRoot`: canonical mirror destination/VCS working tree; writes `skills_root` in generated `skillnet.toml` and exports `AI_SKILLS_REPO` for compatibility (atlas uses `/data/nvme0/can/canix/projects/repos/owned/github.com/caniko/ai-skills`).
- `programs.skillnet.mirrorRoot`: legacy mirror destination option; must match `skillsRoot` when both are set.
- `programs.skillnet.database.backend`: `sqlite` or `postgres` (default `postgres`).
- `programs.skillnet.database.path`: optional SQLite path; defaults to `<dataDir>/multi-phase-plan/calibration.sqlite`.
- `programs.skillnet.database.url`: written into generated `skillnet.toml` when `settings` is declared, or exported as `SKILLNET_DATABASE_URL` for user-managed config.
- `programs.skillnet.database.urlFile`: reads a Postgres URL from a file at shell initialization time.
- `programs.skillnet.settings` and `programs.skillnet.catalogSettings`: render `skillnet.toml` and `skillnet.catalog.toml` from Nix.
- `programs.skillnet.configFile` and `programs.skillnet.catalogConfigFile`: point the CLI at user-managed TOML files.
- `programs.skillnet.package`: package override; use `inputs.skillnet.packages.${pkgs.system}.skillnet` if `pkgs.skillnet` is not in your package set.

### External Git providers

Provider subscriptions fetch a repository and compose every immediate `SKILL.md` directory under `source` into generated global views without copying into or modifying the canonical store:

```nix
programs.skillnet = {
  subscriptions.external = {
    url = "https://github.com/example/agent-skills.git";
    ref = "main";
    source = "skills";
    provider = true;
  };
  subscriptionSyncInterval = "1h";
};
```

When an interval is set, Home Manager runs a user service at login and a timer thereafter. Updates validate against the full bundle before publication; a conflicting or invalid update restores the last working checkout and views. Without an interval, run `skillnet subscription sync --all` explicitly. Copy subscriptions keep their existing behavior and require `target` instead of `provider = true`.

## Storage backends

Postgres is the default calibration backend:

```toml
[database]
url = "postgres://skillnet@localhost/skillnet"
```

Select SQLite explicitly when needed:

```toml
[database]
backend = "sqlite"
path = "/home/alice/.local/share/skillnet/multi-phase-plan/calibration.sqlite"
```

Without `path`, `skillnet` uses the first set variable — `$skillnet_DATA_DIR/multi-phase-plan/calibration.sqlite`, then `$SKILLNET_DATA_DIR/multi-phase-plan/calibration.sqlite` — or falls back to `$XDG_DATA_HOME/skillnet/multi-phase-plan/calibration.sqlite`.

## Pkl skill manifests and bundles

`packages.x86_64-linux.greptile-skills` composes `check-pr`, `greploop`, and `cli-review` from the pinned `caniko/ai-skills` revision in `nix/greptile-bundle.nix`. Its `view/` directory exposes consumer entrypoints with repository contracts and transitive Skillnet dependencies. `checks.x86_64-linux.greptile-skills` verifies that composition in hosted CI, including license retention and reference-only dependency visibility.

`packages.x86_64-linux.greptile-skills-archive` packages those three entrypoints into `greptile-consumer-skills.tar.gz` alongside `provenance.json` and `SHA256SUMS`. Tar entries are dereferenced with normalized order, ownership, and timestamps so the archive outlives the producing runner and Nix store. The check rejects links, special files, and paths outside the three entrypoints.

Until the artifact-upload workflow is available, the hosted development shell emits the qualified archive once per runner as `SKILLNET_CONSUMER_ARCHIVE_V1` JSON frames. Each frame carries the file's SHA-256, byte count, chunk count, and index; consumers must reassemble every chunk and verify those fields plus `SHA256SUMS` before extracting (1 MiB per-file transport limit). Ordinary development shells do not emit these frames.

Placing `Skillnet.pkl` at a canonical store root enables manifest-driven composition via the Rust `pklr` integration. Local imports stay confined to that store; environment, network, temp-dir, and glob access are disabled.

A manifest declares `schemaVersion = 1`, `schemaVersion = 2`, or `schemaVersion = 3` and a `skills` mapping. Unlisted skills stay entrypoints; listed skills may set `role = "entrypoint"` or `role = "reference"` and declare `dependencies`. `defaultDependencies` applies to canonical skills without a custom dependency list. Canonical entries backed by `source` use only their explicit `dependencies`, which keeps reference-only source entries from depending on themselves through a global default; external manifests keep their own `defaultDependencies` for source-backed entries.

```pkl
class Skill {
  role: String = "entrypoint"
  dependencies: Listing<String> = new {}
  source: String? = null
}

schemaVersion = 1
defaultDependencies = List("graphify-policy")
skills: Mapping<String, Skill> = new {
  ["fix-loop"] = new { dependencies = List("fix-loop-ref") }
  ["fix-loop-ref"] = new {
    role = "reference"
    source = "fix-loop/references/repair-contract.md"
  }
}
```

Manifest-enabled scopes materialise bundles under `<dataDir>/bundles/<scope>`, expose only entrypoints in views, and link dependencies inside each bundle at `.skillnet/deps/<skill>`. Bundle scopes require symlink views; stores without `Skillnet.pkl` keep direct canonical-to-view linking.

Schema version 2 adds optional per-user filtering. `defaultUsers` applies when a skill omits `users`; omitting the default leaves the manifest unrestricted for backwards compatibility, while an explicit empty list denies every user. When a manifest declares access, `skillnet.toml` must set `user` (Home Manager writes `home.username`), and every dependency of a materialised skill must also be granted to that user.

```pkl
class Skill {
  role: String = "entrypoint"
  dependencies: Listing<String> = new {}
  users: Listing<String>? = null
}

schemaVersion = 2
defaultUsers = List("can", "dejana")
skills: Mapping<String, Skill> = new {
  ["shared"] = new {}
  ["admin-only"] = new { users = List("can") }
}
```

Schema version 3 adds `defaultHosts` and per-skill `hosts` with the same inheritance rules: omitted lists are unrestricted and empty lists deny all hosts. Configure `host = "atlas"` in `skillnet.toml` or `programs.skillnet.host = "atlas"` in Home Manager. Both user and host grants must match for every skill and dependency; a host policy without an explicit host fails before materialisation (destination host is always configured, never read from the builder hostname). Versions 1 and 2 remain supported.

Select Postgres via environment variable:

```sh
export SKILLNET_DATABASE_URL='postgres://skillnet@localhost/skillnet'
skillnet calibration migrate
```

Or via `skillnet.toml`:

```toml
[database]
url = "postgres://skillnet@localhost/skillnet"
```

Or via Home Manager:

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

`--database-url <URL>` overrides both environment and config for one command. Use `programs.skillnet.database.urlFile` with `sops-nix`, `agenix`, or another secret-backed file for production credentials instead of writing plain URLs into the Nix store via `programs.skillnet.database.url`.

## Model-bound skill templates

Global canonical skills containing `SKILL.md.tmpl` instead of `SKILL.md` render into a regular `SKILL.md` file during global bundle sync while leaving canonical sources untouched. Other child entries link into the bundle, excluding the template and optional `skillnet-template.toml` metadata.

Supported tokens are `{{name}}`, `{{seat_count}}`, and `{{seat_labels}}`. Each `{{#seats}}` … `{{/seats}}` block (sequential blocks allowed, nesting disallowed) repeats per seat with `{{seat.name}}`, `{{seat.label}}`, and `{{seat.dispatch}}`; standalone block tags emit no blank lines. Set `min = 1` and `max = 1` under `[seats]` in `skillnet-template.toml` to require one seat; without metadata, the minimum is one with no maximum.

```toml
[templates]
fleet = "~/canix/lib/ai/SkillModels.pkl"                # shared, versioned
local = "~/.config/skillnet/skill-models.local.pkl"     # this host only

[models]
t3_caches = "~/.local/share/t3code-native/caches"
```

Both sections are optional. `local` loads after `fleet`, replacing any skill it names and replacing `ui` if supplied. Missing binding files are skipped, while a template without a binding fails global sync. Any unexcluded canonical template triggers bundle mode during global sync. Keep binding files outside Skillnet's data directory: sync prunes the bundle and refuses binding paths inside the bundle (as written or resolved), and symlink chains passing through the bundle are unsupported. Set `ui.providers` to named provider instance IDs to filter the model list, or omit / set it to `null` to list all enabled providers.

```sh
skillnet template render sol-loop                          # preview without writing
skillnet template render sol-loop --bindings ./models.pkl # override configured files
skillnet models list --json                              # use configured or default T3 caches
skillnet models list --caches ./snapshots                 # override the cache directory
```

## Quick Start

Inspect configured scopes and view drift:

```sh
skillnet status
skillnet scope list
```

Materialise views and regenerate catalog output:

```sh
skillnet sync
skillnet skill list --scope global
skillnet catalog generate
```

`skillnet sync` is dry-run-on-conflict for promotion candidates: a real view directory newer than canonical prints `would promote ...` and exits 2. Rerun with `--apply-promote` to pull it into canonical. See [docs/src/commands.md](docs/src/commands.md) for the full flag table.

Run calibration queries under the `calibration` group:

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

`skillnet` separates canonical skill stores from derived agent views:

- Global canonical skills default to `global/`; project canonical stores live at `<project>/.skills`.
- Declarative project discovery includes Git repositories under an enabled primary project-tree class containing a real `.skills` directory; worktrees and protected roots are excluded.
- Home Manager supplies project-tree JSON and policy via `project_discovery`; do not maintain discovered entries with `skillnet project add`. Protected projects and compatibility scope names can still be declared as explicit `[[projects]]` entries when needed.
- Project working copies materialise from canonical at `canonical_rel` (default `.agents/skills`).
- Global and project views (such as `.claude/skills`) are generated symlink views; project views use per-skill relative symlinks into the project working copy.
- `link_strategy` defaults to `symlink` for global scopes and `hardlink` for project working copies. Override globally, per project, or per run with `--link symlink|hardlink`.

Hardlink project working copies require `.skills` and the working copy on the same filesystem; cross-device failures error without copying or symlinking. When a Git pull or editor rewrite severs a hardlink with matching content, `skillnet sync` or `skillnet project sync` re-links it; drifted working copies are replaced from canonical.

Config lives at `$XDG_CONFIG_HOME/skillnet/skillnet.toml` and catalog metadata at `$XDG_CONFIG_HOME/skillnet/skillnet.catalog.toml`. Legacy cwd fallbacks `./skillnet.toml` and `./skillnet.catalog.toml` remain active with a deprecation warning (deprecated since `0.6.x`, originally scheduled for removal in `0.7.0`); run `skillnet config migrate` from the old directory to centralise them.

## Command Surface

Top-level commands:

- `status`
- `completions`
- `sync`
- `export`
- `view`
- `skill`
- `scope`
- `project`
- `catalog`
- `calibration`

Many scope-aware commands (such as `sync`, `project sync`, and `status`) accept `--scope <name>`, `--scope projects` (every configured project), or `--scope all` (global plus every project); `view` manages only the global scope. `skillnet sync --all` aliases `skillnet sync --scope all`.

`skillnet project list` prints resolved explicit and discovered projects; `skillnet project list --format json` includes `source` on every row and a workspace-relative path for discovery matches (null when unmatched).

Run `skillnet export` from a repo root to copy repo-stored global skills from `skills/` into the global canonical store and sync global views (for repos such as `visual-rubric`, `fragpipe`, and `tzu`). `.skills/` directories remain project-scoped unless passed explicitly with `--source .skills`.

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
- Source: <https://github.com/caniko/skillnet>

## Release Validation

The release-prep flow validates the repository with:

```sh
simit init flake --check --diff
simit release trust check
simit init ci --platform github --maintainer-key 818D507F1E62139F8A17EAA64623DEA06FDACFE1 --check --diff
nix flake check --keep-going --print-build-logs
treefmt --ci
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
