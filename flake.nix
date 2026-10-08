{
  description = "skillnet AI skill mirror manager";

  inputs = {
    harbor-rs.url = "git+https://github.com/caniko/harbor-rs.git?ref=trunk&rev=ed89d0b13fc61dd1b2217bf4bba97f32cec27ba7";

    nixpkgs.follows = "harbor-rs/nixpkgs";
    rust-overlay.follows = "harbor-rs/rust-overlay";
    crane.follows = "harbor-rs/crane";
    flake-utils.url = "github:numtide/flake-utils";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    advisory-db = {
      url = "git+https://github.com/RustSec/advisory-db.git?ref=main";
      flake = false;
    };
    plinth = {
      # Includes Harbor's private build-scoped cache fallback for hosted docs.
      url = "git+https://github.com/caniko/plinth.git?ref=trunk&rev=ed2424518f888bfb06b3cf4f11101bffe1b740e4";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    advisory-db,
    home-manager,
    nixpkgs,
    harbor-rs,
    plinth,
    flake-utils,
    rust-overlay,
    treefmt-nix,
    git-hooks,
    ...
  }: let
    hmModule = import ./nix/hm-module.nix;
  in
    flake-utils.lib.eachSystem ["x86_64-linux"] (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };

      toolchain = harbor-rs.lib.mkToolchain {inherit pkgs;};
      cross = harbor-rs.lib.mkCross {inherit pkgs system;};
      inherit (toolchain) craneLib rustToolchain;

      src = pkgs.lib.cleanSourceWith {
        src = ./.;
        filter = path: type:
          (craneLib.filterCargoSources path type)
          || pkgs.lib.hasSuffix ".sql" path
          || pkgs.lib.hasSuffix ".json" path
          || pkgs.lib.hasSuffix ".snap" path;
      };

      commonArgs = {
        inherit src;
        strictDeps = true;
        cargoExtraArgs = "--all-features";
      };

      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

      package = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
          nativeCheckInputs = [pkgs.git pkgs.gnupg];
        });

      treefmtEval = treefmt-nix.lib.evalModule pkgs (import ./nix/treefmt.nix);
      pre-commit-check = git-hooks.lib.${system}.run {
        src = ./.;
        hooks = import ./nix/pre-commit.nix {
          inherit pkgs rustToolchain;
          treefmtWrapper = treefmtEval.config.build.wrapper;
        };
      };

      clippyCheck = craneLib.cargoClippy (commonArgs
        // {
          inherit cargoArtifacts;
          cargoClippyExtraArgs = "--all-targets -- --deny warnings";
        });

      fmtCheck = craneLib.cargoFmt {inherit src;};

      nextestCheck = craneLib.cargoNextest (commonArgs
        // {
          inherit cargoArtifacts;
          nativeCheckInputs = [pkgs.git pkgs.gnupg];
        });

      docCheck = craneLib.cargoDoc (commonArgs
        // {
          inherit cargoArtifacts;
          cargoDocExtraArgs = "--no-deps";
          env.RUSTDOCFLAGS = "--deny warnings";
        });

      auditCheck = craneLib.cargoAudit {
        inherit advisory-db src;
      };

      denyCheck = craneLib.cargoDeny {
        inherit src;
      };

      docs = pkgs.stdenv.mkDerivation {
        pname = "skillnet-docs";
        inherit (package) version;
        src = ./docs;
        nativeBuildInputs = [pkgs.mdbook];
        phases = ["buildPhase" "installPhase"];
        buildPhase = ''
          cp -r --no-preserve=mode $src docs
          mdbook build docs
        '';
        installPhase = ''
          cp -r docs/book $out
        '';
      };
      website = plinth.lib.${system}.mkProjectSite {
        pname = "skillnet-website";
        domain = "skillnet.tartanoglu.com";
        configPath = ./website/plinth-project.toml;
        docsPackage = docs;
      };

      hmModuleTest = import ./nix/test-hm-module.nix {
        inherit home-manager package pkgs;
        module = hmModule;
      };
      mkBundle = import ./nix/bundle.nix {
        inherit package pkgs;
      };
      greptileSkills = import ./nix/greptile-bundle.nix {
        inherit mkBundle pkgs;
        skillnetVersion = package.version;
      };
      bundleCheck =
        pkgs.runCommand "skillnet-bundle-check" {
          bundle = mkBundle {
            canonical = builtins.path {
              path = ./nix/test-bundle-source;
              name = "skillnet-test-bundle-source";
            };
            user = "can";
          };
        } ''
          test -L "$bundle/view/demo"
          test ! -e "$bundle/view/shared"
          test -f "$bundle/bundles/global/demo/SKILL.md"
          test ! -L "$bundle/bundles/global/demo/SKILL.md"
          test -L "$bundle/bundles/global/demo/.skillnet/deps/shared"
          touch "$out"
        '';
    in {
      packages = {
        default = package;
        skillnet = package;
        greptile-skills = greptileSkills.bundle;
        greptile-skills-archive = greptileSkills.archive;
        docs = docs;
        website = website;
        site = website;
      };

      apps.deploy-pages = plinth.lib.${system}.mkDeployPagesApp {
        domain = "skillnet.tartanoglu.com";
      };

      formatter = treefmtEval.config.build.wrapper;

      hmModules = {
        default = hmModule;
        skillnet = hmModule;
      };

      lib = {
        externalManifestSupport = true;
        inherit mkBundle;
      };

      checks = {
        default = package;
        formatting = fmtCheck;
        clippy = clippyCheck;
        fmt = fmtCheck;
        test = nextestCheck;
        nextest = nextestCheck;
        doc = docCheck;
        docs = docs;
        audit = auditCheck;
        deny = denyCheck;
        hm-module = hmModuleTest;
        bundle = bundleCheck;
        greptile-skills = greptileSkills.check;
        # A CI-only runtime hook must not make the composition a dev-shell input.
        dev-shell-composition-isolation = assert !(builtins.hasAttr greptileSkills.check.drvPath
          (builtins.getContext self.devShells.${system}.default.shellHook));
          pkgs.runCommand "skillnet-dev-shell-composition-isolation" {} ''touch "$out"'';
        # Fail if flake inputs ever point at the retired Codeberg/Codefloe
        # mirrors again (fleet migrated to github.com/caniko/*).
        # sourceUrl package metadata is excluded: informational only, not fetched.
        host-pinning = let
          # Split across literals so this file never matches its own pattern.
          staleHosts = "cod" + "eberg|cod" + "efloe";
        in
          pkgs.runCommand "skillnet-host-pinning" {} ''
            if ${pkgs.lib.getExe pkgs.ripgrep} -v "sourceUrl" ${./flake.nix} ${./flake.lock} \
              | ${pkgs.lib.getExe pkgs.ripgrep} -q "${staleHosts}"; then
              echo "ERROR: retired forge host in flake inputs:" >&2
              ${pkgs.lib.getExe pkgs.ripgrep} -v "sourceUrl" ${./flake.nix} ${./flake.lock} \
                | ${pkgs.lib.getExe pkgs.ripgrep} -n "${staleHosts}" >&2 || true
              exit 1
            fi
            touch $out
          '';
      };

      devShells = {
        msrv = pkgs.mkShell {
          inputsFrom = [(self.devShells.${system}.default.overrideAttrs (_: {shellHook = "";}))];
          packages = [pkgs.rust-bin.stable."1.88.0".minimal];
          RUSTFLAGS = "";
          CARGO_ENCODED_RUSTFLAGS = "";
          RUSTC_WRAPPER = "";
        };
        default = craneLib.devShell {
          packages = with pkgs;
            [
              alejandra
              cargo-audit
              cargo-deny
              cargo-nextest
              git
              mdbook
              prettier
              pre-commit
              rust-analyzer
              taplo
            ]
            ++ [harbor-rs.packages.${system}.harbor-ci]
            ++ pre-commit-check.enabledPackages;
          shellHook =
            pre-commit-check.shellHook
            + ''
              # Keep the qualified portable consumer export retrievable from the
              # existing hosted logs until the artifact-upload workflow is installed.
              if [ "''${CI:-}" = true ] && [ "''${GITHUB_REPOSITORY:-}" = caniko/skillnet ] \
                && [ -d "''${RUNNER_TEMP:-}" ]; then
                # Resolve only inside hosted CI, not via a derivation interpolation
                # that Nix would realize before the runtime condition is evaluated.
                consumer=$(nix build --no-link --print-out-paths .#checks.${system}.greptile-skills) || exit 1
                marker="$RUNNER_TEMP/$(basename "$consumer").exported"
                if [ ! -e "$marker" ]; then
                  cat "$consumer/consumer-archive.log"
                  touch "$marker"
                fi
              fi
            '';
        };

        docs = harbor-rs.lib.mkDocsShell {
          inherit pkgs cross;
          inherit (toolchain) craneLib;
          packages = with pkgs;
            [
              mdbook
              plinth.packages.${system}.plinth-project
              pre-commit
              rust-analyzer
            ]
            ++ pre-commit-check.enabledPackages;
          extraShellHook = pre-commit-check.shellHook;
        };
      };
    })
    // {
      hmModules = {
        default = hmModule;
        skillnet = hmModule;
      };
    };
}
