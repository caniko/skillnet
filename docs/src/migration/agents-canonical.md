# Agents Canonical Project Layout

Skillnet now defaults project canonical skill stores to:

```text
<project>/.agents/skills
```

The default Claude view remains per-skill relative symlinks under:

```text
<project>/.claude/skills
```

Projects that already set `canonical_rel = ".skills"` keep the old layout. The
default only changes projects that omitted `canonical_rel`.

Project sync also changed the mirror-side aggregator. By default,
`<mirror_root>/projects/<name>` is now a directory of hardlinked files whose
regular files share device and inode with the canonical files under
`.agents/skills`. The `ai-skills` repository therefore sees real files under
`projects/<name>/` that can be committed. It is no longer a directory symlink
unless the project explicitly sets `link_strategy = "symlink"` or a sync run
passes `--link symlink`.

## Keeping `.skills`

If a project should keep using `.skills`, make that explicit:

```toml
[[projects]]
name = "demo"
path = "/path/to/demo"
canonical_rel = ".skills"
```

Then validate without mutating:

```sh
skillnet status --scope demo
skillnet doctor
```

## Migrating Manually

Skillnet does not move project files during `sync`, `status`, or `doctor`.
Migrate deliberately from inside the project repository:

```sh
git status --short
mkdir -p .agents
git mv .skills .agents/skills
skillnet project sync --name demo
git status --short
```

If `.agents/skills` already exists as an old symlink view, remove that symlink
before `git mv`:

```sh
rm .agents/skills
git mv .skills .agents/skills
```

The validation point is:

```sh
skillnet status --scope demo
skillnet doctor
```

Both commands should stop warning about the legacy `.skills` canonical after
`.agents/skills` exists and the config does not override `canonical_rel`.

After migration, run one project sync to refresh the hardlinked aggregator:

```sh
skillnet project sync --name demo
```

If the project checkout and mirror checkout are on different filesystems, this
fails instead of silently copying. Move one checkout, choose
`link_strategy = "symlink"` for that project, or keep the old layout explicitly
until both paths can share a filesystem.
