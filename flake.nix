{
  description = "Mail development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    ...
  }: let
    systems = ["aarch64-darwin" "aarch64-linux" "x86_64-linux"];
    forAllSystems = nixpkgs.lib.genAttrs systems;
  in {
    packages = forAllSystems (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };
      toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      rustPlatform = pkgs.makeRustPlatform {
        cargo = toolchain;
        rustc = toolchain;
      };
      cli = rustPlatform.buildRustPackage {
        pname = "ha-mail";
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
        src = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
            ./migrations
            ./web
            ./mail
            ./custom_components
            ./tests
          ];
        };
        cargoLock = {
          lockFile = ./Cargo.lock;
          outputHashes."ha-app-ui-0.1.0" = "sha256-tgBPoIds8v3V1wzKon8uvMrWh01enQWDuBz7FwsYx20=";
          outputHashes."sqlx-0.9.0" = "sha256-kLvUfUy6vmL9LYU7nzHv+iB6cIJm2CjDf9Yo6wpWrWg=";
        };
        cargoBuildFlags = ["--bin" "ha-mail"];
        # The real-binary HTTP tests run in the package build, too.
        cargoTestFlags = ["--test" "api_cli"];
        # The fixture servers bind only to loopback during the check phase.
        __darwinAllowLocalNetworking = true;
        nativeBuildInputs = [pkgs.pkg-config];
        meta = {
          description = "Native JSON client for the Mail API";
          license = pkgs.lib.licenses.mit;
          mainProgram = "ha-mail";
          platforms = systems;
        };
      };
    in {
      ha-mail = cli;
      default = cli;
    });
    apps = forAllSystems (system: let
      app = {
        type = "app";
        meta.description = "Native JSON client for the Mail API";
        program = "${self.packages.${system}.ha-mail}/bin/ha-mail";
      };
    in {
      ha-mail = app;
      default = app;
    });
    devShells = forAllSystems (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };
    in {
      default = pkgs.mkShell {
        packages = with pkgs; [
          (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
          python314
          nodejs
          alejandra
          uv
          cmake
          pkg-config
          sqlite
          jq
          actionlint
          shellcheck
          cargo-deny
        ];
      };
    });
  };
}
