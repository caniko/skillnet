# Migration: Option B canonical stores

`skillnet 0.5.0` removes reconcile source arbitration and the `skillnet sync`
command group. Each scope has one canonical store. Agent directories are
derived views materialised from that store.

## Removed legacy config fields

These fields are rejected when loading `skillnet.toml`:

- `[global].sources`
- `[global].sync_paths`
- `[global].stale_codex_skill_paths`
- `project_source_rules`
- `extra_sources`
- `[sync]` auto-commit settings for removed pull workflows

## Replacement schema

Global skills live in `global/` under `mirror_root` by default, or in
`[global].canonical_path` when set. The `ai-skills` repository uses
`global_skills/` as its configured global canonical directory. Configure every
generated global view under `[global].views`:

```toml
[global]
views = [
  { label = "claude", path = "/home/alice/.claude/skills", scope = "global" },
  { label = "agents", path = "/home/alice/.agents/skills", scope = "global" },
]
```

Project skills live under each project root at `.skills`. `canonical_rel`
defaults to `.agents/skills` and names the generated working copy; project
views default to `.claude/skills`:

```toml
[[projects]]
name = "demo"
path = "/home/alice/Projects/demo"
canonical_rel = ".agents/skills"
views = [
  { rel = ".claude/skills", label = "claude" },
]
```

## Workflow changes

- Use `skillnet view sync --all` to materialise global views.
- Use `skillnet project sync --all` to materialise project working copies and
  views from project `.skills` stores.
- Use `skillnet skill new|delete|rename|move` to mutate canonical stores.
  These commands sync affected views by default.
- `mirror_root/.skillnet/cache.toml` is obsolete. Delete it if present.

The full migration guide lives at `docs/src/migration/option-b.md`.
