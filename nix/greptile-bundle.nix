# Real consumer composition, qualified by the existing hosted flake workflow.
{
  pkgs,
  mkBundle,
}: let
  revision = "34df2d68a163f2ba400ef070622fb4b8ef81f6f2";
  archive = pkgs.fetchurl {
    url = "https://codeload.github.com/caniko/ai-skills/tar.gz/${revision}";
    hash = "sha256-pxLLLoSwPlvz8hMll6ey4tjurqIw6kIT7gyAVLOFKnc=";
  };
  source =
    pkgs.runCommand "ai-skills-greptile-${builtins.substring 0 12 revision}" {
      nativeBuildInputs = [pkgs.gnutar pkgs.gzip];
    } ''
      mkdir -p "$out"
      tar -xzf ${archive} --strip-components=1 -C "$out"
    '';
  bundle = mkBundle {
    canonical = "${source}/global_skills";
    user = "can";
  };
in {
  inherit bundle;
  check = pkgs.runCommand "skillnet-greptile-composition" {inherit bundle;} ''
    for skill in check-pr greploop cli-review; do
      test -s "$bundle/view/$skill/SKILL.md"
      test -s "$bundle/view/$skill/LICENSE"
      test -s "$bundle/view/$skill/references/repository-contract.md"
    done
    test -s "$bundle/view/check-pr/.skillnet/deps/fix-loop/SKILL.md"
    test -s "$bundle/view/greploop/.skillnet/deps/check-pr/SKILL.md"
    test -s "$bundle/view/cli-review/.skillnet/deps/write-human-style/SKILL.md"
    test -s "$bundle/view/check-pr/.skillnet/deps/fix-loop/.skillnet/deps/fix-loop-ref/SKILL.md"
    # Reference-only packages belong in dependency closures, not entrypoint views.
    test ! -e "$bundle/view/fix-loop-ref"
    touch "$out"
  '';
}
