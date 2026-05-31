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
views = [
  { rel = ".claude/skills", label = "claude" },
]
```

`origin` is optional and is used only by `skillnet project clone --all`.
`canonical_rel` defaults to `.agents/skills`; project views default to
`.claude/skills`. Projects keeping the older `.skills` canonical layout should
set `canonical_rel = ".skills"` explicitly.

## Layout

- Global canonical store: `<mirror_root>/global/`
- Global views: configured `[global].views[*].path`
- Project canonical store: `<mirror_root>/projects/<project-name>/`
- Project working copy: `<project>/<canonical_rel>/`
- Project committed views: `<project>/<view.rel>/`

Global views use absolute symlinks to canonical skills. Project views use
relative symlinks to the project working copy so they can be committed to
project repositories.

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
`skillnet project sync --all` to materialise mirror canonicals, project
working copies, and in-repo views. Project working copies default to hardlinked
directory twins of `projects/<name>/`.

By default, HTTPS origins are refused. Use `--ssh-strict=false` only when an
HTTPS clone is intentional.

## Doctor

`skillnet doctor` now reports:

- missing canonical stores;
- view entries that are not symlinks;
- view symlinks that are broken or point outside the canonical store;
- canonical skills missing from a configured view;
- orphan view entries not matching any canonical skill name;
- missing, foreign, legacy-symlink, severed, or diverged project working
  copies.

Missing project repository paths are warnings, not errors, because a host may
be partially bootstrapped before all project repositories are cloned.

## Reconcile-Pull And Centralised Config (`0.6.0`)

`0.6.0` partially reverses `0.5.0`'s "no reconcile" stance, but only in a
narrow promotion path. `skillnet sync` can promote a user-visible non-symlink
view entry back into canonical when the view content is newer, but the default
run only reports would-promote work and exits `2`. Mutation requires
`--apply-promote`; `--no-promote` keeps `0.5.x` behaviour; Home Manager
activation never promotes by default.

For the XDG migration, `skillnet config migrate`, and the Home Manager
declarative pattern, see
[Centralised config (0.6.0)](centralised-config.md).
