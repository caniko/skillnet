# Real consumer composition, qualified by the existing hosted flake workflow.
{
  pkgs,
  mkBundle,
  skillnetVersion,
}: let
  revision = "ee19812658db84ed2893dac855e8e82101800c32";
  archive = pkgs.fetchurl {
    url = "https://codeload.github.com/caniko/ai-skills/tar.gz/${revision}";
    hash = "sha256-bNMEKhz12wf2//Np8zxhX8mMdzjKXm3/p46JyKMsIwk=";
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
  hostBundles = pkgs.lib.concatMap (host:
    map (user: {
      inherit host user;
      bundle = mkBundle {
        canonical = "${source}/global_skills";
        externalManifests = ["${source}/host_skills/Skillnet.pkl"];
        inherit host user;
      };
    }) ["can" "dejana"]) ["atlas" "nomad"];
  provenance = pkgs.writeText "greptile-consumer-provenance.json" (builtins.toJSON {
    schemaVersion = 1;
    repository = "caniko/ai-skills";
    head = revision;
    inherit skillnetVersion;
    trust = "qualification-only";
    consumable = false;
    skills = ["check-pr" "greploop" "cli-review"];
  });
  portableArchive =
    pkgs.runCommand "skillnet-greptile-consumer-archive" {
      nativeBuildInputs = [pkgs.gnutar pkgs.gzip];
    } ''
      set -euo pipefail
      mkdir -p "$out"
      tar --dereference --hard-dereference --sort=name --mtime=@1 --owner=0 --group=0 --numeric-owner \
        --format=gnu -cf - -C ${bundle}/view check-pr greploop cli-review \
        | gzip -n > "$out/greptile-consumer-skills.tar.gz"
      cp ${provenance} "$out/provenance.json"
      (cd "$out" && sha256sum greptile-consumer-skills.tar.gz provenance.json > SHA256SUMS)
    '';
in {
  inherit bundle;
  archive = portableArchive;
  check =
    pkgs.runCommand "skillnet-greptile-composition" {
      inherit bundle;
      nativeBuildInputs = [pkgs.bash pkgs.jq];
    } ''
      ${pkgs.python3}/bin/python ${source}/ci/test_review_contracts.py
      for skill in check-pr greploop cli-review; do
        test -s "$bundle/view/$skill/SKILL.md"
        test -s "$bundle/view/$skill/LICENSE"
        test -s "$bundle/view/$skill/references/repository-contract.md"
      done
      test -s "$bundle/view/check-pr/.skillnet/deps/fix-loop/SKILL.md"
      test -s "$bundle/view/greploop/.skillnet/deps/check-pr/SKILL.md"
      test -s "$bundle/view/cli-review/.skillnet/deps/write-human-style/SKILL.md"
      test -s "$bundle/view/check-pr/.skillnet/deps/pr-review-reply-style/.skillnet/deps/write-human-style/SKILL.md"
      test -s "$bundle/view/check-pr/.skillnet/deps/fix-loop/.skillnet/deps/fix-loop-ref/SKILL.md"
      test -s "$bundle/view/check-pr/.skillnet/deps/fix-loop/.skillnet/deps/graphify/SKILL.md"
      test -s "$bundle/view/greploop/.skillnet/deps/fix-loop/.skillnet/deps/graphify/SKILL.md"
      test -s "$bundle/view/check-pr/.skillnet/deps/grouped-git-commits/.skillnet/deps/chaosbox-policy/SKILL.md"
      test -s "$bundle/view/check-pr/.skillnet/deps/chaosbox-policy/SKILL.md"
      test -s "$bundle/view/check-pr/.skillnet/deps/fix-loop/.skillnet/deps/solution-placement-policy/SKILL.md"
      test -s "$bundle/view/cli-review/.skillnet/deps/write-human-style/.skillnet/deps/solution-placement-policy/SKILL.md"
      test -s "$bundle/view/gitlab-pages/.skillnet/deps/repo-pages/SKILL.md"
      test -s "$bundle/view/forgejo-pages/.skillnet/deps/repo-pages/SKILL.md"
      # Reference-only packages belong in dependency closures, not entrypoint views.
      test ! -e "$bundle/view/fix-loop-ref"
      test ! -e "$bundle/view/chaosbox-policy"
      test ! -e "$bundle/view/solution-placement-policy"
      test ! -e "$bundle/view/repo-pages"
      ${pkgs.lib.concatMapStringsSep "\n" ({
          host,
          user,
          bundle,
        }: ''
          echo "host-bundle ${host}/${user}: ${bundle}"
          test -f ${bundle}/view/multi-host-agent-orchestration/SKILL.md
          diff -u ${source}/global_skills/multi-host-agent-orchestration/SKILL.md ${bundle}/view/multi-host-agent-orchestration/SKILL.md
          ${
            if host == "atlas"
            then ''
              adapter=${bundle}/view/atlas-nomad-orchestration
              test -f "$adapter/SKILL.md"
              diff -u ${source}/host_skills/atlas-nomad-orchestration/SKILL.md "$adapter/SKILL.md"
              test -f "$adapter/.skillnet/deps/multi-host-agent-orchestration/SKILL.md"
              test -f "$adapter/.skillnet/deps/canix-cli/SKILL.md"
              test -f "$adapter/.skillnet/deps/canix-structure-reference/SKILL.md"
              test -f "$adapter/.skillnet/deps/canix-cli/.skillnet/deps/canix-structure-reference/SKILL.md"
              # references is a source-store symlink; interpret the documented
              # dependency path from its assembled package root, never via ../.
              ${pkgs.python3}/bin/python - "$adapter/.skillnet/deps/canix-cli" <<'PY'
              from pathlib import Path
              import re
              import sys
              root = Path(sys.argv[1])
              reference = (root / "references/secrets-and-registry.md").read_text()
              dependency = re.search(r"from `([^`]+)`", reference).group(1)
              assert (root / dependency).is_file(), dependency
              assert "**canix-cli package root**" in reference
              PY
            ''
            else ''
              test ! -e ${bundle}/view/atlas-nomad-orchestration
              test ! -L ${bundle}/view/atlas-nomad-orchestration
            ''
          }
        '')
        hostBundles}
      mkdir -p "$out"
      cp ${portableArchive}/* "$out/"
      # The current hosted workflow retains logs but has no artifact upload step.
      # Validate portability before emitting bounded, digest-bound transport frames.
      ${pkgs.python3}/bin/python - "$out" > "$out/consumer-archive.log" <<'PY'
      import base64
      import hashlib
      import json
      from pathlib import Path, PurePosixPath
      import sys
      import tarfile

      root = Path(sys.argv[1])
      with tarfile.open(root / "greptile-consumer-skills.tar.gz") as archive:
          for member in archive.getmembers():
              path = PurePosixPath(member.name)
              if (path.is_absolute() or ".." in path.parts
                      or not path.parts or path.parts[0] not in {"check-pr", "greploop", "cli-review"}
                      or not (member.isfile() or member.isdir())):
                  raise ValueError("Consumer archive contains a nonportable entry")
      for name in ["greptile-consumer-skills.tar.gz", "provenance.json", "SHA256SUMS"]:
          raw = (root / name).read_bytes()
          if not 0 < len(raw) <= 1024 * 1024:
              raise ValueError("Consumer archive transport exceeds its byte bound")
          encoded = base64.b64encode(raw).decode("ascii")
          chunks = [encoded[i:i + 1024] for i in range(0, len(encoded), 1024)]
          for index, chunk in enumerate(chunks):
              print("SKILLNET_CONSUMER_ARCHIVE_V1 " + json.dumps({
                  "file": name, "sha256": hashlib.sha256(raw).hexdigest(),
                  "bytes": len(raw), "chunks": len(chunks), "index": index, "data": chunk,
              }, separators=(",", ":")))
      PY
    '';
}
