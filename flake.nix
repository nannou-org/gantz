{
  description = "An environment for creative systems.";

  inputs = {
    # No `follows` for crane: it has no inputs of its own.
    crane.url = "github:ipetkov/crane";
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
        # crane with nixpkgs' stable rustc: the same toolchain `buildRustPackage`
        # used, now driving a deps-only artifact derivation so source edits stop
        # rebuilding the whole dependency closure.
        gantzCraneLib = inputs.crane.mkLib final;
        # crane with the nightly wasm toolchain (rust-src + wasm32) for the
        # website's `-Z build-std` build.
        gantzCraneLibWasm = final.gantzCraneLib.overrideToolchain (p: p.rustToolchainWasmNightly);

        gantz-unwrapped = prev.callPackage ./pkgs/gantz-unwrapped.nix { };
        gantz = final.callPackage ./pkgs/gantz.nix { };
        gantz-website = final.callPackage ./pkgs/gantz-website.nix { };
        serve-gantz-website = final.callPackage ./pkgs/serve-gantz-website.nix { };
        wasm-bindgen-cli = prev.callPackage ./pkgs/wasm-bindgen-cli.nix { };
        # Nightly wasm toolchain for the AudioWorklet website build: `-Z build-std`
        # (recompiling `std` with atomics for WASM threads) is nightly-only, and
        # `rust-src` supplies the `std` sources it rebuilds from.
        rustToolchainWasmNightly = final.rust-bin.selectLatestNightlyWith (
          toolchain:
          toolchain.default.override {
            extensions = [ "rust-src" ];
            targets = [ "wasm32-unknown-unknown" ];
          }
        );
      };

      packages = perSystemPkgs (pkgs: {
        gantz = pkgs.gantz;
        gantz-website = pkgs.gantz-website;
        serve-gantz-website = pkgs.serve-gantz-website;
        wasm-bindgen-cli = pkgs.wasm-bindgen-cli;
        default = pkgs.gantz;
      });

      devShells = perSystemPkgs (pkgs: {
        gantz-dev = pkgs.callPackage ./shell.nix { };
        gantz-web = pkgs.callPackage ./shell-web.nix { };
        default = inputs.self.devShells.${pkgs.stdenv.hostPlatform.system}.gantz-dev;
      });

      formatter = perSystemPkgs (pkgs: pkgs.nixfmt-tree);
    };
}
