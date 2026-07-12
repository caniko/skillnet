# Centralised Config (0.6.0)

`0.6.0` keeps skillnet's canonical/view layout but moves the normal config
story to XDG and Home Manager. The old working-directory pickup still works
for one release, but it is deprecated and will be removed in `0.7.0`.

The target layout is:

- `$XDG_CONFIG_HOME/skillnet/skillnet.toml`
- `$XDG_CONFIG_HOME/skillnet/skillnet.catalog.toml`

When `XDG_CONFIG_HOME` is unset, skillnet uses
`~/.config/skillnet/skillnet.toml` and
`~/.config/skillnet/skillnet.catalog.toml`.

## Why Move

Before `0.6.0`, many workflows relied on running `skillnet` from a checkout
that happened to contain `./skillnet.toml`. That is rank 4 in the discovery
ladder, after explicit flags, environment variables, and XDG config files.

That fallback is convenient for one machine but fragile for automation and
fresh hosts. A shell started in another directory does not see the config, and
Home Manager cannot own the files cleanly while they live as mutable checkout
state.

`0.6.0` therefore prints a deprecation warning whenever it falls through to
legacy cwd discovery. The fallback remains available through the `0.6.x`
series so users can migrate deliberately. `0.7.0` removes rank 4.

## One-Shot Migration

Run the migration from the directory that currently contains
`skillnet.toml` and `skillnet.catalog.toml`:

```sh
skillnet config migrate --dry-run
skillnet config migrate
```

The command makes one decision per file:

| Legacy cwd file | XDG file                   | Default action                                                                 |
| --------------- | -------------------------- | ------------------------------------------------------------------------------ |
| absent          | absent                     | No-op; prints that there is no config to migrate.                              |
| absent          | present                    | No-op; prints that the file is already centralised.                            |
| present         | absent                     | Moves the cwd file to XDG and writes a breadcrumb in the old directory.        |
| present         | present, same content      | Deletes the cwd file and writes a breadcrumb.                                  |
| present         | present, different content | Refuses; pass `--force` only after deciding the cwd file should overwrite XDG. |

Breadcrumb files are plain text:

- `.skillnet.toml.moved-to-xdg`
- `.skillnet.catalog.toml.moved-to-xdg`

Each breadcrumb contains the absolute XDG destination path. After every host
has migrated, remove stale breadcrumbs with:

```sh
skillnet config migrate --remove-breadcrumbs
```

## Home Manager Pattern

For Home Manager users, the final state is to render the XDG files from the
module. `programs.skillnet.settings` becomes `skillnet.toml`, and
`programs.skillnet.catalogSettings` becomes `skillnet.catalog.toml`.

The current `skillnet.toml` from the ai-skills checkout translates to:

```nix
programs.skillnet = {
  enable = true;
  skillsRoot = "/data/nvme0/can/canix/projects/repos/owned/codeberg.org/caniko/ai-skills";
  mirrorRoot = "/data/nvme0/can/canix/projects/repos/owned/codeberg.org/caniko/ai-skills";

  settings = {
    global = {
      views = [
        {
          label = "claude";
          path = "~/.claude/skills";
        }
        {
          label = "agents";
          path = "~/.agents/skills";
        }
      ];
    };

    projects = [
      {
        name = "SynDB";
        path = "/data/nvme0/can/canix/projects/repos/owned/github.com/memorycircuits/SynDB";
      }
      {
        name = "ai-yolo-nix";
        path = "/data/nvme0/can/canix/projects/repos/owned/codeberg.org/caniko/yee-haw";
      }
      {
        name = "canix";
        path = "/data/nvme0/can/canix";
      }
      {
        name = "codex";
        path = "/data/nvme0/can/canix/projects/repos/forks/openai/codex";
      }
      {
        name = "CourseOfLife";
        path = "/data/nvme0/can/canix/projects/personal/professional/CourseOfLife";
      }
      {
        name = "fragpipe-mcp";
        path = "/data/nvme0/can/canix/projects/repos/owned/codeberg.org/caniko/fragpipe-mcp";
      }
      {
        name = "goose";
        path = "/data/nvme0/can/canix/projects/repos/forks/aaif-goose/goose";
      }
      {
        name = "nix-crossbow";
        path = "/data/nvme0/can/canix/projects/repos/owned/codeberg.org/caniko/nix-crossbow";
      }
      {
        name = "plinth";
        path = "/data/nvme0/can/canix/projects/repos/owned/codeberg.org/caniko/plinth";
      }
      {
        name = "regicide";
        path = "/data/nvme0/can/canix/projects/repos/owned/gitlab.com/clg-gaming/regicide";
      }
      {
        name = "rs-modde";
        path = "/data/nvme0/can/canix/projects/repos/owned/codeberg.org/caniko/rs-modde";
      }
      {
        name = "rs_bouldy";
        path = "/data/nvme0/can/canix/projects/repos/owned/codeberg.org/caniko/rs-bouldy";
      }
    ];
  };
};
```

`skillsRoot` and `mirrorRoot` fold the current top-level TOML keys into the
generated file. The module requires them to match because split repository and
mirror roots are not supported yet.

The current catalog file is large. The exact declarative translation can be
kept next to your Home Manager module and evaluated into the attrset that
`catalogSettings` expects:

```nix
programs.skillnet.catalogSettings =
  builtins.fromTOML (builtins.readFile ./skillnet.catalog.toml);
```

That still renders
`$XDG_CONFIG_HOME/skillnet/skillnet.catalog.toml` through Home Manager. It
does not rely on skillnet's legacy cwd discovery path.

If you want the catalog fully inline as Nix, translate each `[[rules]]` table
to one attrset in `rules`. The beginning of the current catalog becomes:

```nix
programs.skillnet.catalogSettings = {
  settings = {
    large_skill_line_threshold = 350;
  };

  rules = [
    {
      path_prefix = "global/";
      scope = "global";
      category = "agent-tools";
      status = "active";
      tags = ["global"];
    }
    {
      path_prefix = "projects/";
      scope = "project";
      category = "domain-workflow";
      status = "active";
      tags = ["project"];
    }
    {
      path_prefix = "global/rust-";
      category = "rust";
      tags = ["rust"];
    }
    {
      path_prefix = "global/rust-crate-release-reference";
      status = "reference";
      tags = ["rust" "reference"];
    }
    {
      name = "nixpkgs-pr";
      status = "retired";
      tags = ["nix" "nixpkgs" "retired"];
      related_skills = ["global/nixpkgs-init-pr"];
      collision_note = "Retired legacy entrypoint; use nixpkgs-init-pr for general nixpkgs PR work or nixpkgs-build-failure-pr for build-log-driven fixes.";
    }
  ];
};
```

Only use fields that the catalog schema already accepts: `path_prefix`,
`name`, `name_prefix`, `name_suffix`, `project`, `scope`, `category`,
`status`, `tags`, `related_skills`, `collision_note`, and the existing
`settings.large_skill_line_threshold`.

## CLI-Only Workflows

Home Manager installs the binary, renders optional config files, and exports
session variables. It does not run `skillnet sync`, `skillnet hook install`, or
other `skillnet` commands during activation. After `home-manager switch`, run
materialisation and hook workflows explicitly:

```sh
skillnet sync
skillnet hook install
```

## HM-Managed Config Caveat

When `programs.skillnet.settings` renders `skillnet.toml`, the resolved config
path is under `/nix/store/` and is read-only. Commands that mutate the config,
including `skillnet project add` and `skillnet project remove`, refuse to write
that file.

For HM-managed configs, edit `programs.skillnet.settings.projects` in the Nix
expression and run `home-manager switch` instead.

## Promotion And Catalog Rules

Promotion changes canonical skill content. The next `skillnet catalog lint` or
`skillnet catalog generate` reads the promoted canonical version, so catalog
rules may classify the skill differently. For example, promoted frontmatter can
move a skill from `active` to `retired` if your rules recognize that status.

Run this before applying promotion on a host with drift:

```sh
skillnet doctor
skillnet sync
skillnet sync --apply-promote
skillnet catalog lint
```

## Troubleshooting

If the deprecation warning still appears after a successful migration, check
which path is being discovered:

```sh
pwd
ls -la skillnet.toml skillnet.catalog.toml \
  .skillnet.toml.moved-to-xdg .skillnet.catalog.toml.moved-to-xdg
ls -la "$XDG_CONFIG_HOME/skillnet"
```

Remove any leftover cwd TOML files after confirming their XDG copies are
correct. Breadcrumbs are harmless, but stale cwd TOMLs keep rank 4 discovery
alive when no XDG file is present.
