{
  pkgs,
  package,
}: {
  canonical,
  user,
  host ? null,
  externalManifests ? [],
  excludedSkills ? [],
  excludedProviderSkills ? [],
  # Pkl bindings for SKILL.md.tmpl templates in `canonical`.
  templateBindings ? null,
}: let
  configTemplate = (pkgs.formats.toml {}).generate "skillnet-bundle.toml.in" ({
      data_dir = "@bundle@";
      link_strategy = "symlink";
      external_manifests = externalManifests;
      excluded_skills = excludedSkills;
      excluded_provider_skills = excludedProviderSkills;
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
    }
    // pkgs.lib.optionalAttrs (host != null) {inherit host;}
    // pkgs.lib.optionalAttrs (templateBindings != null) {
      # Interpolate so a Nix path becomes a tracked store input.
      templates.fleet = "${templateBindings}";
    });
in
  pkgs.runCommandLocal "skillnet-bundle-${user}" {
    nativeBuildInputs = [package];
  } ''
    substitute ${configTemplate} skillnet.toml --replace-fail @bundle@ "$out"
    skillnet --config skillnet.toml view sync --all --allow-delete
    skillnet --config skillnet.toml doctor
    rm -f "$out/skillnet.sqlite"
  ''
