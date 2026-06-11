# Project `.skills` Canonical Layout

Skillnet stores project canonical skills in each project repository:

```text
<project>/.skills/<skill-name>
```

The default generated working copy is hardlinked from canonical:

```text
<project>/.agents/skills
```

The default Claude view remains per-skill relative symlinks into that working
copy:

```text
<project>/.claude/skills
```

`canonical_rel` is retained as the configured working-copy path and defaults to
`.agents/skills`. Do not set project views to `.skills` or to the working-copy
path; views must be separate generated symlink directories.

## Validation

After changing project skills or materialising views, validate with:

```sh
skillnet project status --all
skillnet doctor
```

Expected shape:

- `<project>/.skills` contains real skill directories and no symlink skill entries.
- `<project>/.agents/skills` is generated from `.skills`.
- `<project>/.claude/skills` contains relative symlinks into `.agents/skills`.

If the canonical store and working copy are on different filesystems,
hardlinking fails instead of silently copying. Move one path or choose
`link_strategy = "symlink"` for that project.
