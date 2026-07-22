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
    ((cfg.settings or {}).database or {})
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
    ((cfg.settings or {}) // lib.optionalAttrs (cfg.settings == null) {
      global = { views = []; };
    })
    // {
      data_dir = cfg.dataDir;
      database = generatedDatabaseSettings;
    }
    // lib.optionalAttrs (cfg.mirrorRoot != null) {
      mirror_root = cfg.mirrorRoot;
    }
    // lib.optionalAttrs (cfg.skillsRoot != null) {
      skills_root = cfg.skillsRoot;
    }
    // lib.optionalAttrs (cfg.externalManifests != []) {
      external_manifests = cfg.externalManifests;
    }
    // lib.optionalAttrs (cfg.subscriptions != {}) {
      subscriptions =
        lib.mapAttrs
        (_: subscription: {
          inherit (subscription) url target source;
          ref = subscription.ref;
          delete_policy = subscription.deletePolicy;
        })
        cfg.subscriptions;
    };
in {
  options.programs.skillnet = {
    enable =
      lib.mkEnableOption "skillnet, the AI skill mirror and calibration CLI"
      // {
        description = ''
          Enable skillnet, the AI skill mirror and calibration CLI.

          The Home Manager module installs skillnet, renders optional
          configuration, and exports session variables. It does not run
          skillnet commands during activation; materialisation, hook
          installation, and calibration migrations are explicit CLI workflows.
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
      description = "Root data directory for skillnet; generated bundles and local calibration data live below it.";
    };

    mirrorRoot = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Optional root directory containing the global/ and projects/ skill mirror directories. Written as mirror_root when settings is declared and exported as SKILLNET_MIRROR_ROOT for CLI commands.";
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
            {
              name = "myproject";
              path = "/home/alice/Projects/myproject";
              link_strategy = "hardlink";
            }
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
        skillnet defaults it to ".agents/skills". Link strategy is set here as
        top-level link_strategy or per-project link_strategy; there is no
        separate Nix option because settings is a TOML pass-through.

        Leave null, and leave configFile null, to use a user-managed config
        file.
      '';
    };

    subscriptions = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          url = lib.mkOption {
            type = lib.types.str;
            description = "Git URL for the subscribed skill repository.";
          };

          ref = lib.mkOption {
            type = lib.types.str;
            default = "main";
            description = "Git ref to fetch and materialise for this subscription.";
          };

          target = lib.mkOption {
            type = lib.types.str;
            description = "Local skill directory that receives this subscription's skills.";
          };

          source = lib.mkOption {
            type = lib.types.str;
            default = "global_skills";
            description = "Path inside the subscribed repository containing skill directories.";
          };

          deletePolicy = lib.mkOption {
            type = lib.types.enum ["keep" "prune"];
            default = "keep";
            description = "Whether target skills deleted upstream are kept locally or pruned.";
          };
        };
      });
      default = {};
      description = ''
        Declarative skill repository subscriptions rendered into
        skillnet.toml. Home Manager only writes config; run
        `skillnet subscription sync --all` explicitly to materialise them.
      '';
    };

    externalManifests = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = ''
        Immutable Pkl Skillnet manifests supplied by downstream flakes.
        Their roots are evaluated as capability boundaries and their skills
        are materialised into generated bundles without modifying the
        canonical mirror.
      '';
      example = ["${config.xdg.dataHome}/openpencil/Skillnet.pkl"];
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

    (lib.mkIf (cfg.settings != null || cfg.subscriptions != {} || cfg.externalManifests != []) {
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
    })

    (lib.mkIf (cfg.externalManifests != []) {
      home.activation.skillnetExternalBundles = lib.hm.dag.entryAfter ["writeBoundary"] ''
        ${cfg.package}/bin/skillnet --allow-dirty-destination sync --scope global --no-promote --allow-delete
      '';
    })
  ]);
}
