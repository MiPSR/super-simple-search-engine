{
  description = "Super Simple Search Engine";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        ssse = pkgs.rustPlatform.buildRustPackage {
          pname = "ssse";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl pkgs.sqlite ];
        };
      in {
        packages.default = ssse;
        apps.default = {
          type = "app";
          program = "${ssse}/bin/ssse";
        };
        devShells.default = pkgs.mkShell {
          inputsFrom = [ ssse ];
          packages = [ pkgs.cargo pkgs.rustc pkgs.rustfmt pkgs.clippy ];
        };
      }) // {
      nixosModules.default = { config, lib, pkgs, ... }:
        let cfg = config.services.ssse; in {
          options.services.ssse = {
            enable = lib.mkEnableOption "ssse";
            package = lib.mkOption { type = lib.types.package; default = self.packages.${pkgs.system}.default; };
            bind = lib.mkOption { type = lib.types.str; default = "127.0.0.1:3000"; };
            dataDir = lib.mkOption { type = lib.types.path; default = "/var/lib/ssse"; };
          };
          config = lib.mkIf cfg.enable {
            systemd.services.ssse = {
              wantedBy = [ "multi-user.target" ];
              after = [ "network.target" ];
              serviceConfig = {
                ExecStart = "${cfg.package}/bin/ssse serve --data-dir ${cfg.dataDir} --bind ${cfg.bind}";
                Restart = "on-failure";
                DynamicUser = true;
                StateDirectory = "ssse";
              };
            };
          };
        };
    };
}
