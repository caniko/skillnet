{pkgs}: let
  inherit (pkgs) lib;
  inherit (builtins) attrNames concatLists elem map match toJSON;
  supportedHarnesses = ["opencode" "antigravity" "agents" "claude"];
  agentHarnesses = ["antigravity" "agents"];
  validName = name: match "[a-z0-9][a-z0-9_-]*" name != null;
  cleanPath = path:
    match "[^[:cntrl:]\\\\]+" path
    != null
    && lib.all (part: !(elem part ["" "." ".."])) (lib.splitString "/" path);
  selectHarnesses = allowed: selected:
    assert lib.assertMsg (lib.all (h: elem h allowed) selected)
    "Skillnet: expected harnesses from ${toJSON allowed}, got ${toJSON selected}";
      builtins.filter (h: elem h selected) allowed;

  renderHarnessCommand = {
    name,
    description,
    harness,
    body ? null,
    preflight ? null,
    readFirst ? [],
    follow ? null,
    extra ? null,
    arguments ? true,
  }:
    assert lib.assertMsg (validName name) "Skillnet: invalid command name ${toJSON name}";
    assert lib.assertMsg (match ".*[^[:space:]].*" description != null) "Skillnet: ${name} needs a description";
    assert lib.assertMsg (elem harness supportedHarnesses) "Skillnet: unsupported harness ${toJSON harness}";
    assert lib.assertMsg (body != null || follow != null) "Skillnet: ${name} needs body or follow"; let
      readPrefix =
        if readFirst == []
        then "Follow"
        else "Read ${lib.concatMapStringsSep ", " (file: "`${file}`") readFirst}, then follow";
      text = lib.concatStringsSep "\n" (lib.optional (preflight != null) "Run `${preflight}`. If it fails, stop."
        ++ [
          (
            if body != null
            then body
            else "${readPrefix} `${follow}`${lib.optionalString arguments " with $ARGUMENTS"}."
          )
        ]
        ++ lib.optional (extra != null) extra);
      descriptionLine = "description: ${toJSON description}";
      header =
        if harness == "claude"
        then "# /${name}"
        else lib.concatStringsSep "\n" (["---"] ++ lib.optional (elem harness agentHarnesses) "name: ${name}" ++ [descriptionLine "---"]);
    in "${header}\n\n${text}\n";

  mkIntegrationSync = {
    integration,
    name ? "skillnet-sync-integrations",
  }:
    assert lib.assertMsg (match "[A-Za-z0-9_+-][A-Za-z0-9_+.-]*" name != null) "Skillnet: invalid sync executable name";
      pkgs.writeShellApplication {
        inherit name;
        runtimeInputs = [pkgs.python3];
        text = ''exec python3 ${./integrations.py} sync ${lib.escapeShellArg "${integration}"} "$@"'';
      };

  mkIntegrationCheck = {
    integration,
    src ? null,
    name ? "${integration.name}-check",
  }:
    pkgs.runCommandLocal name {
      nativeBuildInputs = [(pkgs.python3.withPackages (ps: [ps.pyyaml]))];
    } ''
      python3 ${./integrations.py} check ${lib.escapeShellArg "${integration}"}
      ${lib.optionalString (src != null) ''
        ${lib.getExe integration.sync} --check --root ${lib.escapeShellArg "${src}"}
      ''}
      touch "$out"
    '';

  mkHarnessIntegration = {
    name ? "project",
    commands ? {},
    skills ? {},
    harnesses ? ["opencode" "antigravity"],
    managedManifest ? ".agents/.skillnet-integrations.json",
  }: let
    defaults = selectHarnesses supportedHarnesses harnesses;
    commandHarnesses = builtins.mapAttrs (_: command: selectHarnesses supportedHarnesses (command.harnesses or defaults)) commands;
    skillHarnesses = builtins.mapAttrs (_: skill: selectHarnesses agentHarnesses (skill.harnesses or agentHarnesses)) skills;
    agentSelection = selected: lib.take 1 (builtins.filter (h: elem h agentHarnesses) selected);
    includedHarnesses = selectHarnesses supportedHarnesses (defaults ++ concatLists (builtins.attrValues commandHarnesses ++ builtins.attrValues skillHarnesses));
    entries = lib.concatMap (cmd: let
      selected = commandHarnesses.${cmd};
      targets = builtins.filter (h: !(elem h agentHarnesses)) selected ++ agentSelection selected;
    in
      assert lib.assertMsg (validName cmd) "Skillnet: invalid command name ${toJSON cmd}";
        map (harness: {
          inherit harness;
          path =
            if elem harness agentHarnesses
            then ".agents/skills/${cmd}/SKILL.md"
            else ".${harness}/commands/${cmd}.md";
          text = renderHarnessCommand ((builtins.removeAttrs commands.${cmd} ["harnesses"])
            // {
              inherit harness;
              name = cmd;
            });
        })
        targets) (attrNames commands);
    skillEntries = lib.concatMap (skillName: let
      skill = skills.${skillName};
    in
      assert lib.assertMsg (validName skillName) "Skillnet: invalid skill name ${toJSON skillName}";
      assert lib.assertMsg ((skill ? relativeTarget) != (skill ? source)) "Skillnet: ${skillName} needs exactly one of relativeTarget or source";
        map (harness:
          {
            inherit harness;
            path = ".agents/skills/${skillName}";
          }
          // (
            if skill ? relativeTarget
            then {target = skill.relativeTarget;}
            else {source = "${skill.source}";}
          )) (agentSelection skillHarnesses.${skillName})) (attrNames skills);
    collisions = builtins.filter (cmd: skillHarnesses ? ${cmd} && agentSelection commandHarnesses.${cmd} != [] && agentSelection skillHarnesses.${cmd} != []) (attrNames commands);
    config = pkgs.writeText "skillnet-integrations.json" (toJSON {
      inherit name managedManifest;
      harnesses = includedHarnesses;
      inherit entries skillEntries;
    });
    drv =
      pkgs.runCommandLocal "skillnet-harness-integration-${name}" {
        nativeBuildInputs = [pkgs.python3];
        passthru = {
          manifest = "${drv}/manifest.json";
          gitignoreSnippet = "${drv}/gitignore.snippet";
          sync = mkIntegrationSync {
            integration = drv;
            name = "sync-${name}-integrations";
          };
          check = mkIntegrationCheck {integration = drv;};
        };
      } ''
        python3 ${./integrations.py} build ${config} "$out"
      '';
  in
    assert lib.assertMsg (cleanPath managedManifest) "Skillnet: managedManifest must be a clean relative path";
    assert lib.assertMsg (collisions == [])
    "Skillnet: command/skill collision for ${lib.concatStringsSep ", " collisions}; exclude antigravity/agents from the command's harnesses or remove the duplicate skill"; drv;
in {
  inherit supportedHarnesses renderHarnessCommand mkHarnessIntegration mkIntegrationSync mkIntegrationCheck;
}
