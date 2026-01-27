{
  description = "vector benchmark environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in {
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          pkgs.python313
          pkgs.python313Packages.numpy
        ];

        shellHook = ''
          echo "vector benchmark environment"
          echo "Python: $(python --version)"
        '';
      };
    };
}
