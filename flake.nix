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
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml ./Cargo.lock ./LICENSE ./src ./static
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "rs") ./tests)
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl pkgs.sqlite ];
        };
      in {
        packages.default = ssse;
        apps.default = {
          type = "app";
          program = "${ssse}/bin/ssse";
          meta.description = "Super Simple Search Engine";
        };
        devShells.default = pkgs.mkShell {
          inputsFrom = [ ssse ];
          packages = [ pkgs.cargo pkgs.rustc pkgs.rustfmt pkgs.clippy pkgs.nodejs ];
        };
        checks = {
          package = ssse;
          frontend = pkgs.runCommand "ssse-javascript-check" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            for file in ${./static/js}/*.js; do node --check "$file"; done
            touch "$out"
          '';
        } // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          container = pkgs.testers.runNixOSTest (import ./tests/nixos.nix { inherit self pkgs; });
        };
      }) // {
      nixosModules.default = { config, lib, pkgs, ... }:
        let cfg = config.services.ssse; in {
          options.services.ssse = {
            enable = lib.mkEnableOption "ssse";
            package = lib.mkOption { type = lib.types.package; default = self.packages.${pkgs.stdenv.hostPlatform.system}.default; };
            bind = lib.mkOption { type = lib.types.str; default = "127.0.0.1:3000"; };
            dataDir = lib.mkOption { type = lib.types.path; default = "/var/lib/ssse"; };
            insecureCookie = lib.mkOption { type = lib.types.bool; default = false; description = "Allow admin cookies over HTTP for local development only."; };
          };
          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ cfg.package ];
            systemd.services.ssse = {
              wantedBy = [ "multi-user.target" ];
              after = [ "network-online.target" ];
              wants = [ "network-online.target" ];
              environment = {
                SSSE_DATA_DIR = toString cfg.dataDir;
                SSSE_BIND = cfg.bind;
                SSSE_SOCKET = "/run/ssse/ssse.sock";
              } // lib.optionalAttrs cfg.insecureCookie { SSSE_INSECURE_COOKIE = "1"; };
              serviceConfig = {
                ExecStart = "${cfg.package}/bin/ssse serve";
                Restart = "on-failure";
                User = "ssse";
                Group = "ssse";
                StateDirectory = "ssse";
                StateDirectoryMode = "0700";
                RuntimeDirectory = "ssse";
                RuntimeDirectoryMode = "0700";
                UMask = "0077";
                NoNewPrivileges = true;
                ProtectSystem = "strict";
                ProtectHome = true;
                PrivateTmp = true;
                RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
                ReadWritePaths = [ cfg.dataDir ];
                TimeoutStopSec = 120;
              };
            };
            users.users.ssse = { isSystemUser = true; group = "ssse"; };
            users.groups.ssse = {};
            systemd.tmpfiles.rules = [ "d ${cfg.dataDir} 0700 ssse ssse -" ];
          };
        };
      # Import this module to provision a complete persistent NixOS container.
      # The container shares the host network; a host reverse proxy can reach loopback.
      nixosModules.container = { config, lib, ... }:
        let cfg = config.services.ssseContainer; in {
          options.services.ssseContainer = {
            enable = lib.mkOption { type = lib.types.bool; default = true; };
            bind = lib.mkOption { type = lib.types.str; default = "127.0.0.1:3000"; };
            dataDir = lib.mkOption { type = lib.types.path; default = "/var/lib/ssse-container"; };
            insecureCookie = lib.mkOption { type = lib.types.bool; default = false; };
          };
          config = lib.mkIf cfg.enable {
            systemd.tmpfiles.rules = [ "d ${cfg.dataDir} 0700 root root -" ];
            containers.ssse = {
              autoStart = true;
              privateNetwork = false;
              bindMounts."/var/lib/ssse" = { hostPath = toString cfg.dataDir; isReadOnly = false; };
              config = { ... }: {
                imports = [ self.nixosModules.default ];
                system.stateVersion = "26.05";
                services.ssse = { enable = true; inherit (cfg) bind insecureCookie; };
              };
            };
          };
        };
    };
}
