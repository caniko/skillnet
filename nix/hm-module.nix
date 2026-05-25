{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.skillnet;
  tomlFormat = pkgs.formats.toml {};
  generatedConfigFile = "${config.xdg.configHome}/skillnet/skillnet.toml";
  generatedCatalogConfigFile = "${config.xdg.configHome}/skillnet/skillnet.catalog.toml";
  generatedDatabaseSettings =
    (cfg.settings.database or {})
    // {
      backend = cfg.database.backend;
    }
    // lib.optionalAttrs (cfg.database.path != null) {
      path = cfg.database.path;
    }
    // lib.optionalAttrs (cfg.database.url != null) {
      url = cfg.database.url;
    };
  generatedSettings =
    cfg.settings
    // {
      database = generatedDatabaseSettings;
    }
    // lib.optionalAttrs (cfg.mirrorRoot != null) {
      mirror_root = cfg.mirrorRoot;
    }
    // lib.optionalAttrs (cfg.skillsRoot != null) {
      skills_root = cfg.skillsRoot;
    };
in {
  options.programs.skillnet = {
    enable =
      lib.mkEnableOption "skillnet, the AI skill mirror and calibration CLI"
      // {
        description = ''
          Enable skillnet, the AI skill mirror and calibration CLI.

          During Home Manager activation, skillnet materialises configured
          global views and per-project aggregator symlinks. As described in
          the mirror canonical store dossier's "Fresh-host bootstrap order"
          section, hosts without every configured project cloned should see
          stderr warnings from `skillnet project sync --all` for missing
          projects instead of a failed activation.
        '';
      };

    package = lib.mkOption {
      type = lib.types.package;
      default =
        pkgs.skillnet
        or (throw "programs.skillnet.package not set and pkgs.skillnet unavailable; pass a package explicitly");
      defaultText = lib.literalExpression "pkgs.skillnet or (throw ...)";
      description = "The skillnet package to install. Custom no-default-features builds need the postgres feature when using the Postgres database backend.";
    };

    dataDir = lib.mkOption {
      type = lib.types.str;
      default = "${config.xdg.dataHome}/skillnet";
      description = "Root data directory for skillnet; per-skill calibration databases live under <dataDir>/<skill>/.";
    };

    mirrorRoot = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional root directory containing the global/ and projects/ skill mirror directories. Written as mirror_root when settings is declared and exported as SKILLNET_MIRROR_ROOT for activation-time commands.";
    };

    settings = lib.mkOption {
      type = lib.types.nullOr tomlFormat.type;
      default = null;
      example = lib.literalExpression ''
        {
          global = {
            views = [
              { label = "claude"; path = "/home/alice/.claude/skills"; scope = "global"; }
              { label = "agents"; path = "/home/alice/.agents/skills"; scope = "global"; }
            ];
          };
          projects = [
            { name = "myproject"; path = "/home/alice/Projects/myproject"; }
          ];
        }
      '';
      description = ''
        Declarative content of skillnet.toml, written to
        $XDG_CONFIG_HOME/skillnet/skillnet.toml. The CLI discovers this
        XDG path by default. The module also folds in programs.skillnet.database
        and mirrorRoot so shell-specific env import is not required for normal
        declarative installs.

        This is a TOML pass-through value. skillnet validates the schema at
        runtime; the removed pre-0.5.0 fields [global].sources, sync_paths,
        stale_codex_skill_paths, and project_source_rules are rejected by the
        CLI with a migration error. Project entries may omit canonical_rel;
        skillnet defaults it to ".skills".

        Leave null, and leave configFile null, to use a user-managed config
        file.
      '';
    };

    catalogSettings = lib.mkOption {
      type = lib.types.nullOr tomlFormat.type;
      default = null;
      description = ''
        Declarative content of skillnet.catalog.toml, written to
        $XDG_CONFIG_HOME/skillnet/skillnet.catalog.toml. The CLI discovers this
        XDG path by default. Pass-through: skillnet validates the schema at
        runtime.
      '';
    };

    configFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Absolute path to skillnet.toml. When set, exported as
        SKILLNET_CONFIG, overriding the generated XDG path from settings.
        Leave null to let the CLI use XDG config discovery.
      '';
    };

    catalogConfigFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Absolute path to skillnet.catalog.toml. Exported as
        SKILLNET_CATALOG_CONFIG when set, overriding the generated path
        from catalogSettings.
      '';
    };

    skillsRoot = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional root of the ai-skills checkout. Written as skills_root when settings is declared and used as the canonical mirror destination/VCS working tree.";
    };

    database = {
      backend = lib.mkOption {
        type = lib.types.enum ["sqlite" "postgres"];
        default = "postgres";
        description = "Calibration storage backend.";
      };

      path = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "SQLite database path; defaults to <dataDir>/multi-phase-plan/calibration.sqlite.";
      };

      url = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Postgres connection URL; required when backend = \"postgres\".";
      };

      urlFile = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = ''
          Path to a file containing the Postgres connection URL. Preferred
          over database.url because url ends up in the world-readable
          hm-session-vars.sh. Read at shell init and exported as
          SKILLNET_DATABASE_URL when backend = "postgres".
        '';
      };
    };

    hooks = {
      enable = lib.mkEnableOption "skillnet Claude Code hook installation";

      settingsFile = lib.mkOption {
        type = lib.types.path;
        default = "${config.home.homeDirectory}/.claude/settings.json";
        description = "Path to the Claude Code settings.json to manage.";
      };

      events = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = ["PostToolUse"];
        description = "Claude Code hook events to install skillnet ingest handlers for.";
      };

      matchers = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = ["Skill"];
        description = "Claude Code hook matchers to install. Multiple matchers produce multiple managed entries.";
      };
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {
      assertions = [
        {
          assertion = cfg.database.backend != "postgres" || cfg.database.url != null || cfg.database.urlFile != null;
          message = "programs.skillnet.database needs `url` or `urlFile` when backend = \"postgres\".";
        }
        {
          assertion = cfg.database.backend != "sqlite" || cfg.database.path == null || lib.hasPrefix "/" cfg.database.path;
          message = "programs.skillnet.database.path must be absolute when backend = \"sqlite\".";
        }
        {
          assertion = cfg.configFile == null || lib.hasPrefix "/" cfg.configFile;
          message = "programs.skillnet.configFile must be an absolute path.";
        }
        {
          assertion = cfg.catalogConfigFile == null || lib.hasPrefix "/" cfg.catalogConfigFile;
          message = "programs.skillnet.catalogConfigFile must be an absolute path.";
        }
        {
          assertion = cfg.database.urlFile == null || lib.hasPrefix "/" cfg.database.urlFile;
          message = "programs.skillnet.database.urlFile must be an absolute path.";
        }
        {
          assertion = cfg.mirrorRoot == null || lib.hasPrefix "/" cfg.mirrorRoot;
          message = "programs.skillnet.mirrorRoot must be an absolute path.";
        }
        {
          assertion = cfg.skillsRoot == null || lib.hasPrefix "/" cfg.skillsRoot;
          message = "programs.skillnet.skillsRoot must be an absolute path.";
        }
        {
          assertion = cfg.skillsRoot == null || cfg.mirrorRoot == null || cfg.skillsRoot == cfg.mirrorRoot;
          message = "programs.skillnet.skillsRoot and programs.skillnet.mirrorRoot must match; separate mirror and repository roots are not supported yet.";
        }
      ];

      home.packages = [cfg.package];
    }

    (lib.mkIf (cfg.settings != null) {
      xdg.enable = lib.mkDefault true;
      xdg.configFile."skillnet/skillnet.toml".source =
        tomlFormat.generate "skillnet.toml" generatedSettings;
    })

    (lib.mkIf (cfg.catalogSettings != null) {
      xdg.enable = lib.mkDefault true;
      xdg.configFile."skillnet/skillnet.catalog.toml".source =
        tomlFormat.generate "skillnet.catalog.toml" cfg.catalogSettings;
    })

    (lib.mkIf (cfg.configFile != null) {
      home.sessionVariables.SKILLNET_CONFIG = toString cfg.configFile;
    })

    (lib.mkIf (cfg.catalogConfigFile != null) {
      home.sessionVariables.SKILLNET_CATALOG_CONFIG = toString cfg.catalogConfigFile;
    })

    (lib.mkIf (cfg.mirrorRoot != null) {
      home.sessionVariables.SKILLNET_MIRROR_ROOT = cfg.mirrorRoot;
    })

    (lib.mkIf (cfg.database.backend == "sqlite") {
      home.sessionVariables = {
        # Deprecated compatibility alias; prefer SKILLNET_DATA_DIR.
        skillnet_DATA_DIR = cfg.dataDir;
        SKILLNET_DATA_DIR = cfg.dataDir;
      };

      home.activation.skillnet-data-dir = lib.hm.dag.entryAfter ["writeBoundary"] ''
        $DRY_RUN_CMD mkdir -p ${lib.escapeShellArg cfg.dataDir}
        ${lib.optionalString (cfg.database.path != null) ''
          $DRY_RUN_CMD mkdir -p ${lib.escapeShellArg (builtins.dirOf cfg.database.path)}
        ''}
      '';
    })

    (lib.mkIf (cfg.database.backend == "postgres" && cfg.database.urlFile == null && cfg.settings == null) {
      home.sessionVariables.SKILLNET_DATABASE_URL = cfg.database.url;
    })

    (lib.mkIf (cfg.database.backend == "postgres" && cfg.database.urlFile != null) {
      programs.bash.initExtra = ''
        if [ -r ${lib.escapeShellArg cfg.database.urlFile} ]; then
          export SKILLNET_DATABASE_URL="$(cat ${lib.escapeShellArg cfg.database.urlFile})"
        fi
      '';

      programs.zsh.initContent = ''
        if [ -r ${lib.escapeShellArg cfg.database.urlFile} ]; then
          export SKILLNET_DATABASE_URL="$(cat ${lib.escapeShellArg cfg.database.urlFile})"
        fi
      '';

      programs.fish.shellInit = ''
        if test -r ${lib.escapeShellArg cfg.database.urlFile}
          set -gx SKILLNET_DATABASE_URL (cat ${lib.escapeShellArg cfg.database.urlFile})
        end
      '';
    })

    (lib.mkIf (cfg.skillsRoot != null) {
      home.sessionVariables.AI_SKILLS_REPO = cfg.skillsRoot;

      home.activation.skillnet-skills-root = lib.hm.dag.entryAfter ["writeBoundary"] ''
        if [ ! -d ${lib.escapeShellArg cfg.skillsRoot} ]; then
          echo "WARNING: skillnet: programs.skillnet.skillsRoot does not exist: ${cfg.skillsRoot}" >&2
          echo "WARNING: skillnet: clone or restore the ai-skills checkout at that path; skipping for now." >&2
        fi
      '';
    })

    {
      home.activation.skillnet-views = lib.hm.dag.entryAfter ["writeBoundary" "skillnet-skills-root"] ''
        if [ -z "''${SKILLNET_MIRROR_ROOT-}" ]; then
          mirror=${lib.escapeShellArg (
          if cfg.mirrorRoot != null
          then cfg.mirrorRoot
          else ""
        )}
        else
          mirror="$SKILLNET_MIRROR_ROOT"
        fi
        if [ -z "$mirror" ] || [ ! -d "$mirror/global" ]; then
          echo "WARNING: skillnet: mirror not found at $mirror; skipping view materialisation" >&2
        else
          $DRY_RUN_CMD ${cfg.package}/bin/skillnet view sync --all --allow-delete
          $DRY_RUN_CMD ${cfg.package}/bin/skillnet project sync --all --allow-delete || true
        fi
      '';
    }

    (lib.mkIf cfg.hooks.enable {
      home.activation.skillnetInstallHook = lib.hm.dag.entryAfter ["writeBoundary"] ''
        $DRY_RUN_CMD ${cfg.package}/bin/skillnet hook install \
          --settings ${lib.escapeShellArg (toString cfg.hooks.settingsFile)} \
          --events ${lib.escapeShellArg (lib.concatStringsSep "," cfg.hooks.events)} \
          --matchers ${lib.escapeShellArg (lib.concatStringsSep "," cfg.hooks.matchers)}
      '';
    })
  ]);
}
