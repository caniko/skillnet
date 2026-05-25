# Quick Start

Install from crates.io:

```sh
cargo install skillnet
```

Common first commands:

```sh
skillnet status
skillnet scope list
skillnet view sync --all
skillnet project sync --all
skillnet skill list --scope global
skillnet catalog generate
```

`skillnet.toml` defines canonical stores and derived views. `skillnet.catalog.toml` defines catalog metadata used by `catalog generate` and `catalog lint`.

Configuration is discovered from `$XDG_CONFIG_HOME/skillnet/skillnet.toml`
first, then from legacy `./skillnet.toml` when the XDG file is absent. Override
the path with `--config` or `SKILLNET_CONFIG`.

## Home Manager

The flake exports `hmModules.default` and `hmModules.skillnet`. A declarative
install can render `skillnet.toml` and `skillnet.catalog.toml` directly:

```nix
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
  skillsRoot = "/home/alice/ai-skills";
  database = {
    backend = "postgres";
    urlFile = config.age.secrets.skillnet-pg-url.path;
  };
};
```

Use `database.urlFile` for Postgres credentials so secrets are read at shell
initialization time instead of being written into `hm-session-vars.sh`.
