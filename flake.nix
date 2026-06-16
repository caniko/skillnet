{
  description = "skillnet AI skill mirror manager";

  inputs = {
    rs-harbor.url = "git+https://codeberg.org/caniko/rs-harbor.git";

    nixpkgs.follows = "rs-harbor/nixpkgs";
    rust-overlay.follows = "rs-harbor/rust-overlay";
    crane.follows = "rs-harbor/crane";
    flake-utils.follows = "rs-harbor/flake-utils";
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
      url = "git+https://codeberg.org/caniko/plinth.git";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs = {
    advisory-db,
    home-manager,
    nixpkgs,
    rs-harbor,
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

      toolchain = rs-harbor.lib.mkToolchain {inherit pkgs;};
      cross = rs-harbor.lib.mkCross {inherit pkgs system;};
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
    in {
      packages = {
        default = package;
        skillnet = package;
        docs = docs;
        website = website;
        site = website;
      };

      apps.deploy-pages = plinth.lib.${system}.mkDeployPagesApp {
        domain = "skillnet.tartanoglu.com";
      };

      formatter = treefmtEval.config.build.wrapper;

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
      };

      devShells = {
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
            ++ pre-commit-check.enabledPackages;
          shellHook = pre-commit-check.shellHook;
        };

        docs = rs-harbor.lib.mkDocsShell {
          inherit pkgs cross;
          inherit (toolchain) craneLib;
          packages = with pkgs; [
            mdbook
            plinth.packages.${system}.plinth-project
            pre-commit
            rust-analyzer
          ] ++ pre-commit-check.enabledPackages;
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
