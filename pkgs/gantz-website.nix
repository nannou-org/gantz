# The gantz website (the site that ships): the app built for cpal's AudioWorklet backend, which
# runs audio on a dedicated Web Audio thread via WASM threads (SharedArrayBuffer). It needs a
# *nightly* toolchain (`-Z build-std` to recompile `std` with atomics) and the shared-memory
# build flags from `wasm-threads-env.nix`.
#
# Built with crane's `buildTrunkPackage` plus an explicit deps-only artifact derivation, so the
# dependency closure AND the build-std `std` rebuild compile once and survive workspace source
# edits. `-Z build-std` recompiles `std` from the rust-src component, so `std`'s own crates.io
# deps must be vendored alongside the app's (the sandbox has no network) - crane's
# `vendorMultipleCargoDeps` merges both lockfiles into one vendor dir.
{
  craneLib,
  lib,
  lld,
  llvmPackages,
  rustToolchainWasmNightly,
  wasm-bindgen-cli,
}:
let
  root = ../.;
  # Cargo-relevant files (incl. Trunk.toml and .cargo/config.toml via the *.toml
  # rule), the compile-time .gantz assets, and the web page assets/hooks.
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (craneLib.fileset.commonCargoSources root)
      (lib.fileset.fileFilter (file: file.hasExt "gantz") root)
      ../crates/gantz/web
    ];
  };

  # Shared verbatim between the deps and website derivations: any divergence in
  # cargo flags, RUSTFLAGS or profile between the two invalidates cargo's
  # fingerprints and silently recompiles everything in the final derivation.
  commonArgs =
    # RUSTFLAGS (atomics + shared memory), CARGO_UNSTABLE_BUILD_STD, and the
    # wasm-capable CC/AR for `ring`.
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
          # `std`'s own lock, from the rust-src component `build-std` rebuilds from.
          "${rustToolchainWasmNightly.passthru.availableComponents.rust-src}/lib/rustlib/src/rust/library/Cargo.lock"
        ];
      };
      doCheck = false;
      # trunk, binaryen and the pinned wasm-bindgen-cli come via buildTrunkPackage.
      nativeBuildInputs = [ lld ];
    };

  # Trunk compiles with `--profile wasm_release` (index.html's
  # data-cargo-profile-release), so deps must be compiled under the same
  # profile. Only here: buildTrunkPackage passes `--release` to trunk exactly
  # when CARGO_PROFILE is the default "release", and trunk applies
  # data-cargo-profile-release only under `--release`.
  cargoArtifacts = craneLib.buildDepsOnly (commonArgs // { CARGO_PROFILE = "wasm_release"; });
in
craneLib.buildTrunkPackage (
  commonArgs
  // {
    inherit cargoArtifacts wasm-bindgen-cli;
    trunkIndexPath = "crates/gantz/web/index.html";
    # buildTrunkPackage installs "$(dirname trunkIndexPath)/dist"; Trunk.toml
    # lives at the root so trunk's default dist would land there instead.
    trunkExtraBuildArgs = "--dist crates/gantz/web/dist";
  }
)
