# Agents Canonical Project Layout

Skillnet stores project canonical skills in the mirror checkout:

```text
<mirror_root>/projects/<name>
```

The default project-local working copy is hardlinked from canonical:

```text
<project>/.agents/skills
```

The default Claude view remains per-skill relative symlinks into that working
copy:

```text
<project>/.claude/skills
```

`canonical_rel` still controls the project-local working-copy path and defaults
to `.agents/skills`. Project sync can bootstrap the mirror canonical from
legacy project-local real skill directories, then refresh `.agents/skills` as a
hardlinked copy and `.claude/skills` as relative symlinks.

## Keeping `.skills`

If a project should keep the generated working copy at `.skills`, make that
explicit:

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

## Migrating

Skillnet preserves legacy `.skills` as a backup. To migrate deliberately, run
sync and then inspect both repositories:

```sh
git status --short
skillnet sync --scope demo
skillnet doctor
git status --short
```

If `<mirror_root>/projects/<name>` is missing, empty, or contains only copied
symlinks that match legacy `.skills` entries, sync imports real skill
directories from `.skills`. If the mirror already contains divergent real
content while the project is still in the old symlink-only shape, sync stops
and requires manual reconciliation.

After sync:

- `<mirror_root>/projects/<name>` should contain real skill directories;
- `<project>/.agents/skills` should be hardlinked to the mirror canonical;
- `<project>/.claude/skills` should contain relative symlinks into
  `.agents/skills`;
- `<project>/.skills` may remain as an informational backup until removed
  deliberately.

If the project checkout and mirror checkout are on different filesystems,
hardlinking fails instead of silently copying. Move one checkout or choose
`link_strategy = "symlink"` for that project.
