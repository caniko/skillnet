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
  declarativeAgentsView = "${homeDirectory}/.agents/skills";
  declarativeProject = "${homeDirectory}/Projects/myproject";
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
        views = [
          {
            label = "claude";
            path = declarativeSource;
            scope = "global";
          }
          {
            label = "agents";
            path = declarativeAgentsView;
            scope = "global";
          }
        ];
      };
      projects = [
        {
          name = "myproject";
          path = declarativeProject;
        }
      ];
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
  hooksSettingsFile = "${homeDirectory}/.claude/settings.json";
  hooksConfig = mkHmConfig {
    database.backend = "sqlite";
    hooks = {
      enable = true;
      settingsFile = hooksSettingsFile;
      events = ["PostToolUse"];
      matchers = ["Skill"];
    };
  };
  hooksDisabledMasterConfig = home-manager.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      module
      {
        home.username = "skillnet-test";
        home.homeDirectory = homeDirectory;
        home.stateVersion = "24.11";

        programs.skillnet = {
          enable = false;
          package = package;
          hooks.enable = true;
        };
      }
    ];
  };
  promoteConfig = mkHmConfig {
    database.backend = "sqlite";
    activation.promote = true;
  };
  quietConflictConfig = mkHmConfig {
    database.backend = "sqlite";
    activation.failOnConflict = false;
  };
  noDeleteConfig = mkHmConfig {
    database.backend = "sqlite";
    activation.allowDelete = false;
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
    mkdir -p ${declarativeAgentsView}
    mkdir -p ${declarativeProject}/.skills

    export HOME=${homeDirectory}
    export USER=skillnet-test
    export PATH="${package}/bin:$PATH"

    grep -F 'Activating %s" "skillnet-data-dir"' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'mkdir -p ${dataDir}' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'Activating %s" "skillnet-skills-root"' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'Activating %s" "skillnet-views"' ${sqliteConfig.activationPackage}/activate >/dev/null
    test "$(grep -F '${package}/bin/skillnet sync' ${sqliteConfig.activationPackage}/activate | wc -l)" = 1
    grep -F '${package}/bin/skillnet sync \' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F -- '--no-promote \' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F -- '--allow-delete' ${sqliteConfig.activationPackage}/activate >/dev/null
    ! grep -F -- '--apply-promote' ${sqliteConfig.activationPackage}/activate >/dev/null
    ! grep -F '|| true' ${sqliteConfig.activationPackage}/activate >/dev/null
    ! grep -F 'skillnet view sync' ${sqliteConfig.activationPackage}/activate >/dev/null
    ! grep -F 'skillnet project sync' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'skipping for now.' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'mirror not found at' ${sqliteConfig.activationPackage}/activate >/dev/null
    mkdir -p ${dataDir}
    test -d ${dataDir}

    grep -F '${package}/bin/skillnet sync \' ${promoteConfig.activationPackage}/activate >/dev/null
    grep -F -- '--apply-promote \' ${promoteConfig.activationPackage}/activate >/dev/null
    ! grep -F -- '--no-promote' ${promoteConfig.activationPackage}/activate >/dev/null

    grep -F -- '--allow-delete || true' ${quietConflictConfig.activationPackage}/activate >/dev/null
    ! grep -F '|| true' ${sqliteConfig.activationPackage}/activate >/dev/null

    ! grep -F -- '--allow-delete' ${noDeleteConfig.activationPackage}/activate >/dev/null

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

    grep -F 'Activating %s" "skillnetInstallHook"' ${hooksConfig.activationPackage}/activate >/dev/null
    grep -F '$DRY_RUN_CMD ${package}/bin/skillnet hook install' ${hooksConfig.activationPackage}/activate >/dev/null
    grep -F -- '--settings ${hooksSettingsFile}' ${hooksConfig.activationPackage}/activate >/dev/null
    grep -F -- '--events PostToolUse' ${hooksConfig.activationPackage}/activate >/dev/null
    grep -F -- '--matchers Skill' ${hooksConfig.activationPackage}/activate >/dev/null
    ! grep -F 'skillnet hook install' ${hooksDisabledMasterConfig.activationPackage}/activate >/dev/null

    rm -rf ${skillsRoot}
    DRY_RUN=1 ${declarativeConfig.activationPackage}/activate --driver-version 1 2>activation-stderr.log
    grep -F 'skipping for now.' activation-stderr.log >/dev/null
    grep -F 'mirror not found at ${skillsRoot}; skipping sync' activation-stderr.log >/dev/null

    unset __HM_SESS_VARS_SOURCED
    . ${declarativeConfig.activationPackage}/home-path/etc/profile.d/hm-session-vars.sh
    test -z "''${SKILLNET_CONFIG:-}"
    test -z "''${SKILLNET_CATALOG_CONFIG:-}"
    test "''${SKILLNET_MIRROR_ROOT:-}" = "${skillsRoot}"
    mkdir -p ${homeDirectory}/.config/skillnet
    ln -sf ${declarativeConfig.activationPackage}/home-files/.config/skillnet/skillnet.toml ${homeDirectory}/.config/skillnet/skillnet.toml
    ln -sf ${declarativeConfig.activationPackage}/home-files/.config/skillnet/skillnet.catalog.toml ${homeDirectory}/.config/skillnet/skillnet.catalog.toml
    test -f ${homeDirectory}/.config/skillnet/skillnet.toml
    test -f ${homeDirectory}/.config/skillnet/skillnet.catalog.toml
    grep -F "mirror_root = '${skillsRoot}'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    grep -F "skills_root = '${skillsRoot}'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    grep -F "backend = 'sqlite'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    grep -F "label = 'claude'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    grep -F "label = 'agents'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    grep -F "name = 'myproject'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    grep -F "path = '${declarativeProject}'" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    ! grep -F "sync_paths" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null
    ! grep -F "stale_codex_skill_paths" ${homeDirectory}/.config/skillnet/skillnet.toml >/dev/null

    export PATH="${declarativeConfig.activationPackage}/home-path/bin:$PATH"
    mkdir -p ${skillsRoot}/global
    mkdir -p ${declarativeSource}
    mkdir -p ${declarativeAgentsView}
    mkdir -p ${declarativeProject}/.skills
    cd /tmp
    test ! -e skillnet.toml
    unset SKILLNET_CONFIG
    unset SKILLNET_CATALOG_CONFIG
    unset SKILLNET_MIRROR_ROOT
    skillnet status --all >/dev/null

    touch $out
  ''
