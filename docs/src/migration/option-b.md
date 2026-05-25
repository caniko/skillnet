# Option B Migration

`skillnet 0.5.0` removed reconcile source arbitration. `skillnet 0.5.1`
finishes the migration by making `doctor` validate the new canonical/view
layout and by adding a fresh-host clone helper.

## Schema

Removed legacy fields:

- `[global].sources`
- `[global].sync_paths`
- `[global].stale_codex_skill_paths`
- `project_source_rules`
- `extra_sources`

Replacement global schema:

```toml
[global]
views = [
  { label = "claude", path = "/home/alice/.claude/skills", scope = "global" },
  { label = "agents", path = "/home/alice/.agents/skills", scope = "global" },
]
```

Replacement project schema:

```toml
[[projects]]
name = "demo"
path = "/home/alice/Projects/demo"
origin = "git@codeberg.org:alice/demo.git"
canonical_rel = ".skills"
views = [
  { rel = ".claude/skills", label = "claude" },
  { rel = ".agents/skills", label = "agents" },
]
```

`origin` is optional and is used only by `skillnet project clone --all`.
`canonical_rel` defaults to `.skills`; project views default to
`.claude/skills` and `.agents/skills`.

## Layout

- Global canonical store: `<mirror_root>/global/`
- Global views: configured `[global].views[*].path`
- Project canonical store: `<project>/<canonical_rel>/`
- Project committed views: `<project>/<view.rel>/`
- Project mirror aggregator: `<mirror_root>/projects/<project-name>` symlink
  to `<project>/<canonical_rel>/`

Global views use absolute symlinks to canonical skills. Project views use
relative symlinks so they can be committed to project repositories.

## Fresh Host Bootstrap

On a new host:

```sh
git clone git@codeberg.org:caniko/ai-skills.git /path/to/ai-skills
skillnet project clone --all --dry-run
skillnet project clone --all
home-manager switch
skillnet doctor
```

`project clone --all` skips existing project paths, warns for projects without
`origin`, clones missing projects with configured origins, then runs
`skillnet project sync --all` to materialise in-repo views and mirror
aggregator symlinks.

By default, HTTPS origins are refused. Use `--ssh-strict=false` only when an
HTTPS clone is intentional.

## Doctor

`skillnet doctor` now reports:

- missing canonical stores;
- view entries that are not symlinks;
- view symlinks that are broken or point outside the canonical store;
- canonical skills missing from a configured view;
- orphan view entries not matching any canonical skill name;
- missing, non-symlink, broken, or unexpected project aggregator symlinks.

Missing project repository paths are warnings, not errors, because a host may
be partially bootstrapped before all project repositories are cloned.
