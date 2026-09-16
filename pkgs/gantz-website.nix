# The gantz website. The app is built for cpal's AudioWorklet backend, which runs audio on a
# dedicated Web Audio thread via WASM threads and SharedArrayBuffer. It needs a nightly
# toolchain, since `-Z build-std` recompiles `std` with atomics. It also needs the
# shared-memory build flags from `wasm-threads-env.nix`.
#
# crane's `buildTrunkPackage` builds the site from an explicit deps-only artifact derivation.
# The dependency closure and the build-std `std` rebuild then compile once and survive
# workspace source edits. `-Z build-std` recompiles `std` from the rust-src component. The
# sandbox has no network, so `std`'s own crates.io deps must be vendored alongside the app's.
# crane's `vendorMultipleCargoDeps` merges both lockfiles into one vendor dir.
{
  craneLib,
  lib,
  lld,
  llvmPackages,
  rustToolchainWasmNightly,
  wasm-bindgen-cli,
}:
let
  workspace = import ./workspace-src.nix { inherit craneLib lib; };
  # The workspace source plus the web page assets and hooks.
  src = lib.fileset.toSource {
    inherit (workspace) root;
    fileset = lib.fileset.unions [
      workspace.fileset
      ../crates/gantz/web
    ];
  };

  commonArgs =
    # The WASM-threads RUSTFLAGS, CARGO_UNSTABLE_BUILD_STD, and the wasm-capable
    # CC/AR for `ring`.
    (import ./wasm-threads-env.nix { inherit llvmPackages; }) // {
      inherit src;
      pname = "gantz-website";
      inherit (craneLib.crateNameFromCargoToml { cargoToml = ../crates/gantz/Cargo.toml; })
        version
        ;
      strictDeps = true;
      CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
      cargoExtraArgs = "--locked -p gantz --bin gantz";
      cargoVendorDir = craneLib.vendorMultipleCargoDeps {
        inherit (craneLib.findCargoFiles src) cargoConfigs;
        cargoLockList = [
          ../Cargo.lock
          "${rustToolchainWasmNightly.passthru.availableComponents.rust-src}/lib/rustlib/src/rust/library/Cargo.lock"
        ];
      };
      doCheck = false;
      # trunk, binaryen and the pinned wasm-bindgen-cli come via buildTrunkPackage.
      nativeBuildInputs = [ lld ];
    };

  # Trunk compiles with `--profile wasm_release`, set by index.html's
  # data-cargo-profile-release, so deps must use the same profile. Set the
  # profile only here. buildTrunkPackage passes `--release` to trunk only when
  # CARGO_PROFILE is the default "release", and trunk applies
  # data-cargo-profile-release only under `--release`.
  cargoArtifacts = craneLib.buildDepsOnly (commonArgs // { CARGO_PROFILE = "wasm_release"; });
in
craneLib.buildTrunkPackage (
  commonArgs
  // {
    inherit cargoArtifacts wasm-bindgen-cli;
    trunkIndexPath = "crates/gantz/web/index.html";
    trunkExtraBuildArgs = "--dist crates/gantz/web/dist";
  }
)
