{
  description = "Keyboard remapping and mouse control";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      forAllSystems = lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
      ];
    in
    {
      nixosModules.default = import ./nix/module.nix self;

      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "fievel";
            inherit ((lib.importTOML ./Cargo.toml).package) version;

            # Only what the build reads, which leaves out the demo videos.
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.lock
                ./Cargo.toml
                ./build.rs
                ./fievel.alt.config
                ./fievel.config
                ./src
                ./tests
              ];
            };

            cargoLock.lockFile = ./Cargo.lock;

            # The config screen opens through xdg-open, which may not be on PATH
            # when Fievel runs as a user service.
            postPatch = ''
              substituteInPlace src/config_screen.rs \
                --replace-fail 'Command::new("xdg-open")' \
                'Command::new("${lib.getExe' pkgs.xdg-utils "xdg-open"}")'
            '';

            meta = {
              description = "Keyboard remapping and mouse control";
              homepage = "https://github.com/MontyTheSoftwareEngineer/fievel";
              license = lib.licenses.mit;
              mainProgram = "fievel";
              platforms = lib.platforms.linux;
            };
          };
        }
      );
    };
}
