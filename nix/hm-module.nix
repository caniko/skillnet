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
in {
  options.programs.skillnet = {
    enable = lib.mkEnableOption "skillnet, the AI skill mirror and calibration CLI";

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

    settings = lib.mkOption {
      type = lib.types.nullOr tomlFormat.type;
      default = null;
      example = lib.literalExpression ''
        {
          global = {
            sources = [
              {
                label = "claude";
                path = "/home/alice/.claude/skills";
                priority = 2;
              }
            ];
            sync_paths = [ "/home/alice/.agents/skills" ];
            stale_codex_skill_paths = [ "/home/alice/.codex/skills" ];
          };
        }
      '';
      description = ''
        Declarative content of skillnet.toml, written to
        $XDG_CONFIG_HOME/skillnet/skillnet.toml and exported via
        SKILLNET_CONFIG when configFile is unset. Pass-through:
        skillnet validates the schema at runtime. Leave null, and leave
        configFile null, to keep the binary's cwd-based default behaviour.
      '';
    };

    catalogSettings = lib.mkOption {
      type = lib.types.nullOr tomlFormat.type;
      default = null;
      description = ''
        Declarative content of skillnet.catalog.toml, written to
        $XDG_CONFIG_HOME/skillnet/skillnet.catalog.toml and exported via
        SKILLNET_CATALOG_CONFIG when catalogConfigFile is unset.
        Pass-through: skillnet validates the schema at runtime.
      '';
    };

    configFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Absolute path to skillnet.toml. When set, exported as
        SKILLNET_CONFIG so the binary can be invoked from any directory,
        overriding the generated path from settings. Leave null with
        settings unset to keep the cwd-based default behaviour.
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
      description = "Optional root of the ai-skills checkout containing the skill mirror and calibration artifacts.";
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
      ];

      home.packages = [cfg.package];
    }

    (lib.mkIf (cfg.settings != null) {
      programs.skillnet.configFile = lib.mkDefault generatedConfigFile;
      xdg.enable = lib.mkDefault true;
      xdg.configFile."skillnet/skillnet.toml".source =
        tomlFormat.generate "skillnet.toml" cfg.settings;
    })

    (lib.mkIf (cfg.catalogSettings != null) {
      programs.skillnet.catalogConfigFile = lib.mkDefault generatedCatalogConfigFile;
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

    (lib.mkIf (cfg.database.backend == "postgres" && cfg.database.urlFile == null) {
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
  ]);
}
