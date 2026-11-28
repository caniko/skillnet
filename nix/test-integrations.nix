{
  pkgs,
  integrations,
}: let
  inherit (pkgs.lib) getExe;
  inherit (integrations) mkHarnessIntegration mkIntegrationCheck renderHarnessCommand;
  description = "Read: \"two files\"\nthen follow the steps";
  source = pkgs.runCommandLocal "skillnet-test-skill-source" {} ''
    mkdir -p "$out/references"
    cat > "$out/SKILL.md" <<'EOF'
    ---
    name: copied
    description: A copied skill.
    ---
    Contents.
    EOF
    echo reference > "$out/references/details.md"
  '';
  config = {
    name = "test";
    harnesses = integrations.supportedHarnesses;
    commands = {
      run = {
        inherit description;
        preflight = "test -e readiness";
        readFirst = ["one.md" "two.md"];
        follow = "guide.md";
        extra = "Keep a record.";
      };
      obsolete = {
        description = "A command to prune";
        body = "Do this.";
        harnesses = ["agents" "opencode"];
      };
    };
    skills = {
      helper.relativeTarget = "../../.claude/skills/helper";
      copied = {inherit source;};
    };
  };
  integration = mkHarnessIntegration config;
  reduced = mkHarnessIntegration (config
    // {
      commands = builtins.removeAttrs config.commands ["obsolete"];
      skills = config.skills // {copied.relativeTarget = "../../.claude/skills/copied";};
    });
  fixture =
    pkgs.runCommandLocal "skillnet-synced-test-workspace" {
      nativeBuildInputs = [(pkgs.python3.withPackages (ps: [ps.pyyaml]))];
    } ''
      workspace="$PWD/work space"
      portal="$workspace/.agents/skills/unmanaged-portal/SKILL.md"
      opencode="$workspace/.opencode/commands/run.md"
      mkdir -p "$(dirname "$portal")" "$(dirname "$opencode")" "$workspace/.claude/skills/helper"
      echo unmanaged > "$portal"
      echo hand-written > "$opencode"
      if ${getExe integration.sync} --root "$workspace" 2> refusal; then exit 1; fi
      grep -F 'refusing to overwrite an unmanaged path' refusal
      test ! -e "$workspace/.agents/.skillnet-integrations.json"
      ${getExe integration.sync} --force --root "$workspace"
      (cd "$workspace"; ${getExe integration.sync} --check)
      cmp "$workspace/.agents/.skillnet-integrations.json" ${integration.manifest}
      test "$(readlink "$workspace/.agents/skills/helper")" = ../../.claude/skills/helper
      test "$(cat "$workspace/.agents/skills/copied/references/details.md")" = reference
      ${getExe integration.sync} --print-gitignore > gitignore
      cmp gitignore ${integration.gitignoreSnippet}
      grep -Fx '!/.opencode/commands/*.md' gitignore
      grep -Fx '!/.agents/skills/helper' gitignore
      grep -Fx '!/.claude/commands/*.md' gitignore
      grep -Fx 'Run `test -e readiness`. If it fails, stop.' "$opencode"
      grep -Fx 'Read `one.md`, `two.md`, then follow `guide.md` with $ARGUMENTS.' "$opencode"
      grep -Fx 'Keep a record.' "$opencode"
      grep -Fx '# /run' "$workspace/.claude/commands/run.md"
      python3 - ${integration.manifest} "$opencode" <<'PY'
      import json, pathlib, sys, yaml
      manifest = json.loads(pathlib.Path(sys.argv[1]).read_text())
      assert manifest["harnesses"] == ["opencode", "antigravity", "agents", "claude"]
      assert sum(f["path"] == ".agents/skills/run/SKILL.md" for f in manifest["files"]) == 1
      assert yaml.safe_load(pathlib.Path(sys.argv[2]).read_text().split("---")[1])["description"] == \
          'Read: "two files"\nthen follow the steps'
      PY

      echo changed > "$opencode"
      if ${getExe integration.sync} --check --root "$workspace" 2> drift; then exit 1; fi
      grep -Fx '.opencode/commands/run.md: differs' drift
      grep -Fx changed "$opencode"
      ${getExe integration.sync} --root "$workspace"
      ${getExe integration.sync} --check --root "$workspace"

      # A symlinked parent is not permission to write outside the workspace.
      mkdir "$PWD/unsafe" "$PWD/outside"
      ln -s "$PWD/outside" "$PWD/unsafe/.agents"
      if ${getExe integration.sync} --force --root "$PWD/unsafe" 2> unsafe.log; then exit 1; fi
      grep -F 'refusing to follow it' unsafe.log
      test -z "$(ls -A "$PWD/outside")"

      if ${getExe reduced.sync} --check --root "$workspace" 2> stale; then exit 1; fi
      grep -Fx '.agents/skills/obsolete/SKILL.md: stale managed entry' stale
      grep -Fx '.opencode/commands/obsolete.md: stale managed entry' stale
      copied="$workspace/.agents/skills/copied"
      echo local > "$copied/local.md"
      if ${getExe reduced.sync} --root "$workspace" 2> local.log; then exit 1; fi
      grep -F 'refusing to overwrite an unmanaged path' local.log
      grep -Fx local "$copied/local.md"
      test -f "$copied/SKILL.md"
      rm "$copied/local.md"
      ${getExe reduced.sync} --root "$workspace"
      ${getExe reduced.sync} --check --root "$workspace"
      test ! -e "$workspace/.agents/skills/obsolete"
      test ! -e "$workspace/.opencode/commands/obsolete.md"
      test "$(cat "$portal")" = unmanaged
      test "$(readlink "$workspace/.agents/skills/copied")" = ../../.claude/skills/copied
      cp -R "$workspace" "$out"
    '';
  sourceCheck = mkIntegrationCheck {
    integration = reduced;
    src = fixture;
  };
in
  assert renderHarnessCommand {
    name = "run";
    inherit description;
    harness = "claude";
    follow = "guide.md";
    arguments = false;
  }
  == "# /run\n\nFollow `guide.md`.\n";
  assert !(builtins.tryEval (mkHarnessIntegration (config // {skills.run = config.skills.helper;})).drvPath).success;
  assert !(builtins.tryEval (mkHarnessIntegration (config // {managedManifest = "../escape";})).drvPath).success;
    pkgs.runCommandLocal "skillnet-harness-integrations-test" {} ''
      test -f ${integration.check}
      test -f ${sourceCheck}
      echo "harness integration checks passed"
      touch "$out"
    ''
