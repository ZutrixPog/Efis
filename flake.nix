{
  description = "Efis development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.rust-bin.stable.latest.default
            pkgs.just
            pkgs.docker
            pkgs.python3
            pkgs.python3Packages.numpy
          ];

          shellHook = ''
            echo "Efis development environment"
            cargo --version
            just --version
          '';
        };
      }
    );
}
