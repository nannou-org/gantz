{
  description = "An environment for creative systems.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay = {
      inputs.nixpkgs.follows = "nixpkgs";
      url = "github:oxalica/rust-overlay";
    };
    systems.url = "github:nix-systems/default";
  };

  outputs =
    inputs:
    let
      overlays = [
        inputs.rust-overlay.overlays.default
        inputs.self.overlays.default
      ];
      perSystemPkgs =
        f:
        inputs.nixpkgs.lib.genAttrs (import inputs.systems) (
          system: f (import inputs.nixpkgs { inherit overlays system; })
        );
    in
    {
      overlays.default = final: prev: {
        gantz-unwrapped = prev.callPackage ./pkgs/gantz-unwrapped.nix { };
        gantz = final.callPackage ./pkgs/gantz.nix { };
        gantz-wasm = prev.callPackage ./pkgs/gantz-wasm.nix { };
        gantz-wasm-bindgen = final.callPackage ./pkgs/gantz-wasm-bindgen.nix { };
        wasm-bindgen-cli = prev.callPackage ./pkgs/wasm-bindgen-cli.nix { };
      };

      packages = perSystemPkgs (pkgs: {
        gantz = pkgs.gantz;
        gantz-wasm = pkgs.gantz-wasm;
        gantz-wasm-bindgen = pkgs.gantz-wasm-bindgen;
        wasm-bindgen-cli = pkgs.wasm-bindgen-cli;
        default = pkgs.gantz;
      });

      devShells = perSystemPkgs (pkgs: {
        gantz-dev = pkgs.callPackage ./shell.nix { };
        default = inputs.self.devShells.${pkgs.stdenv.hostPlatform.system}.gantz-dev;
      });

      formatter = perSystemPkgs (pkgs: pkgs.nixfmt-tree);
    };
}
