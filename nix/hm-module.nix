{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.skillnet;
in {
  options.programs.skillnet = {
    enable = lib.mkEnableOption "skillnet, the AI skill mirror and calibration CLI";

    package = lib.mkOption {
      type = lib.types.package;
      default =
        pkgs.skillnet
        or (throw "programs.skillnet.package not set and pkgs.skillnet unavailable; pass a package explicitly");
      defaultText = lib.literalExpression "pkgs.skillnet or (throw ...)";
      description = "The skillnet package to install. The Postgres database backend requires a skillnet build with the postgres feature enabled.";
    };

    dataDir = lib.mkOption {
      type = lib.types.str;
      default = "${config.xdg.dataHome}/skillnet";
      description = "Root data directory for skillnet; per-skill calibration databases live under <dataDir>/<skill>/.";
    };

    database = {
      backend = lib.mkOption {
        type = lib.types.enum ["sqlite" "postgres"];
        default = "sqlite";
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
    };

    extraConfig = lib.mkOption {
      type = lib.types.attrs;
      default = {};
      description = "Reserved for future declarative skillnet config; unused in 0.2.0.";
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {
      assertions = [
        {
          assertion = cfg.database.backend != "postgres" || cfg.database.url != null;
          message = "programs.skillnet.database.url is required when programs.skillnet.database.backend = \"postgres\".";
        }
      ];

      home.packages = [cfg.package];
    }

    (lib.mkIf (cfg.database.backend == "sqlite") {
      home.sessionVariables = {
        skillnet_DATA_DIR = cfg.dataDir;
        SKILLNET_DATA_DIR = cfg.dataDir;
      };

      home.activation.skillnet-data-dir = lib.hm.dag.entryAfter ["writeBoundary"] ''
        $DRY_RUN_CMD mkdir -p ${lib.escapeShellArg cfg.dataDir}
      '';
    })

    (lib.mkIf (cfg.database.backend == "postgres") {
      home.sessionVariables.SKILLNET_DATABASE_URL = cfg.database.url;
    })
  ]);
}
