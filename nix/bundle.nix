{
  pkgs,
  package,
}: {
  canonical,
  user,
  externalManifests ? [],
}: let
  configTemplate = (pkgs.formats.toml {}).generate "skillnet-bundle.toml.in" {
    data_dir = "@bundle@";
    link_strategy = "symlink";
    external_manifests = externalManifests;
    inherit user;
    database = {
      backend = "sqlite";
      path = "@bundle@/skillnet.sqlite";
    };
    global = {
      canonical_path = toString canonical;
      views = [
        {
          label = "view";
          path = "@bundle@/view";
          scope = "global";
        }
      ];
    };
  };
in
  pkgs.runCommandLocal "skillnet-bundle-${user}" {
    nativeBuildInputs = [package];
  } ''
    substitute ${configTemplate} skillnet.toml --replace-fail @bundle@ "$out"
    skillnet --config skillnet.toml view sync --all --allow-delete
    skillnet --config skillnet.toml doctor
    rm -f "$out/skillnet.sqlite"
  ''
