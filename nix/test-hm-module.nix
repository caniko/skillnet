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

  mkHmConfig = extraSkillnetConfig:
    home-manager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        module
        {
          home.username = "skillnet-test";
          home.homeDirectory = homeDirectory;
          home.stateVersion = "24.11";

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
in
  pkgs.runCommand "skillnet-hm-module-test"
  {
    nativeBuildInputs = [
      package
    ];
  }
  ''
    set -eu

    rm -rf ${homeDirectory}
    mkdir -p ${homeDirectory}
    mkdir -p ${skillsRoot}

    export HOME=${homeDirectory}
    export USER=skillnet-test
    export PATH="${package}/bin:$PATH"

    grep -F 'Activating %s" "skillnet-data-dir"' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'mkdir -p ${dataDir}' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'Activating %s" "skillnet-skills-root"' ${sqliteConfig.activationPackage}/activate >/dev/null
    grep -F 'configured programs.skillnet.skillsRoot does not exist: ${skillsRoot}' ${sqliteConfig.activationPackage}/activate >/dev/null
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
    grep -F 'configured programs.skillnet.skillsRoot does not exist: ${skillsRoot}' ${postgresConfig.activationPackage}/activate >/dev/null
    test -x ${postgresConfig.activationPackage}/home-path/bin/skillnet
    unset __HM_SESS_VARS_SOURCED
    . ${postgresConfig.activationPackage}/home-path/etc/profile.d/hm-session-vars.sh
    test -z "''${skillnet_DATA_DIR:-}"
    test -z "''${SKILLNET_DATA_DIR:-}"
    test "''${AI_SKILLS_REPO:-}" = "${skillsRoot}"
    test "''${SKILLNET_DATABASE_URL:-}" = "${postgresUrl}"

    touch $out
  ''
