{
  home-manager,
  module,
  package,
  pkgs,
}: let
  homeDirectory = "/tmp/skillnet-hm-module-test-home";
  dataDir = "${homeDirectory}/.local/share/skillnet";
  skillsRoot = "${homeDirectory}/ai-skills";
  postgresUrl = "postgres://skillnet-test@example.invalid/skillnet";
  declarativeSource = "${homeDirectory}/.claude/skills";
  urlFile = "/run/secrets/pg-url";

  mkHmConfig = extraSkillnetConfig:
    home-manager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        module
        {
          home.username = "skillnet-test";
          home.homeDirectory = homeDirectory;
          home.stateVersion = "24.11";

          programs.bash.enable = true;

          programs.skillnet =
            {
              enable = true;
              package = package;
              inherit dataDir skillsRoot;
            }
            // extraSkillnetConfig;
        }
      ];
    };

  sqliteConfig = mkHmConfig {
    database.backend = "sqlite";
  };
  postgresConfig = mkHmConfig {
    database = {
      url = postgresUrl;
    };
  };
  declarativeConfig = mkHmConfig {
    database.backend = "sqlite";
    settings = {
      database.backend = "sqlite";
      global = {
        sources = [
          {
            label = "claude";
            path = declarativeSource;
            priority = 1;
          }
        ];
        sync_paths = [];
        stale_codex_skill_paths = [];
      };
    };
    mirrorRoot = skillsRoot;
    catalogSettings = {
      settings = {};
      rules = [];
    };
  };
  urlFileConfig = mkHmConfig {
    database = {
      backend = "postgres";
      urlFile = urlFile;
    };
  };
in
  pkgs.runCommand "skillnet-hm-module-test"
  {
    nativeBuildInputs = [
      pkgs.nix
      package
    ];
  }
  ''
    set -eu

    rm -rf ${homeDirectory}
    mkdir -p ${homeDirectory}
    mkdir -p ${homeDirectory}/.local/state/nix/profiles
    mkdir -p ${skillsRoot}
    mkdir -p ${skillsRoot}/global
    mkdir -p ${declarativeSource}

    export HOME=${homeDirectory}
    export USER=skillnet-test
    export PATH="${package}/bin:$PATH"

    grep -F 'Activating %s" "skillnet-data-dir"' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'mkdir -p ${dataDir}' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'Activating %s" "skillnet-skills-root"' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'skipping for now.' ${sqliteConfig.activationPackage}/activate >/dev/null
    mkdir -p ${dataDir}
    test -d ${dataDir}

    test -x ${sqliteConfig.activationPackage}/home-path/bin/skillnet
    unset __HM_SESS_VARS_SOURCED
    . ${sqliteConfig.activationPackage}/home-path/etc/profile.d/hm-session-vars.sh
    test "''${skillnet_DATA_DIR:-}" = "${dataDir}"
    test "''${SKILLNET_DATA_DIR:-}" = "${dataDir}"
    test "''${AI_SKILLS_REPO:-}" = "${skillsRoot}"
    test -z "''${SKILLNET_DATABASE_URL:-}"

    export PATH="${sqliteConfig.activationPackage}/home-path/bin:$PATH"
    skillnet --help >/dev/null
    skillnet calibration migrate
    test -f ${dataDir}/multi-phase-plan/calibration.sqlite

    unset skillnet_DATA_DIR
    unset SKILLNET_DATA_DIR
    unset AI_SKILLS_REPO
    unset SKILLNET_DATABASE_URL

    ! grep -F 'Activating %s" "skillnet-data-dir"' ${postgresConfig.activationPackage}/activate >/dev/null
    ! grep -F 'mkdir -p ${dataDir}' ${postgresConfig.activationPackage}/activate >/dev/null
    grep -F 'Activating %s" "skillnet-skills-root"' ${postgresConfig.activationPackage}/activate >/dev/null
    grep -F 'skipping for now.' ${postgresConfig.activationPackage}/activate >/dev/null
    test -x ${postgresConfig.activationPackage}/home-path/bin/skillnet
    unset __HM_SESS_VARS_SOURCED
    . ${postgresConfig.activationPackage}/home-path/etc/profile.d/hm-session-vars.sh
    test -z "''${skillnet_DATA_DIR:-}"
    test -z "''${SKILLNET_DATA_DIR:-}"
    test "''${AI_SKILLS_REPO:-}" = "${skillsRoot}"
    test "''${SKILLNET_DATABASE_URL:-}" = "${postgresUrl}"

    unset skillnet_DATA_DIR
    unset SKILLNET_DATA_DIR
    unset SKILLNET_CONFIG
    unset SKILLNET_CATALOG_CONFIG
    unset SKILLNET_DATABASE_URL
    unset AI_SKILLS_REPO

    ! grep -F 'SKILLNET_DATABASE_URL=' ${urlFileConfig.activationPackage}/home-path/etc/profile.d/hm-session-vars.sh >/dev/null
    grep -R -F 'SKILLNET_DATABASE_URL' ${urlFileConfig.activationPackage}/home-files >/dev/null
    grep -R -F '${urlFile}' ${urlFileConfig.activationPackage}/home-files >/dev/null

    rm -rf ${skillsRoot}
    DRY_RUN=1 ${declarativeConfig.activationPackage}/activate --driver-version 1 2>activation-stderr.log
    grep -F 'skipping for now.' activation-stderr.log >/dev/null

    unset __HM_SESS_VARS_SOURCED
    . ${declarativeConfig.activationPackage}/home-path/etc/profile.d/hm-session-vars.sh
    test -z "''${SKILLNET_CONFIG:-}"
    test -z "''${SKILLNET_CATALOG_CONFIG:-}"
    test -z "''${SKILLNET_MIRROR_ROOT:-}"
    mkdir -p ${homeDirectory}/.config/skillnet
    ln -sf ${declarativeConfig.activationPackage}/home-files/.config/skillnet/skillnet.toml ${homeDirectory}/.config/skillnet/skillnet.toml
    ln -sf ${declarativeConfig.activationPackage}/home-files/.config/skillnet/skillnet.catalog.toml ${homeDirectory}/.config/skillnet/skillnet.catalog.toml
    test -f ${homeDirectory}/.config/skillnet/skillnet.toml
    test -f ${homeDirectory}/.config/skillnet/skillnet.catalog.toml
    grep -F "mirror_root = '${skillsRoot}'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    grep -F "skills_root = '${skillsRoot}'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    grep -F "backend = 'sqlite'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null

    export PATH="${declarativeConfig.activationPackage}/home-path/bin:$PATH"
    cd /tmp
    test ! -e skillnet.toml
    unset SKILLNET_CONFIG
    unset SKILLNET_CATALOG_CONFIG
    unset SKILLNET_MIRROR_ROOT
    skillnet status >/dev/null

    touch $out
  ''
