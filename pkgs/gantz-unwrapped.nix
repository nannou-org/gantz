{
  alsa-lib,
  craneLib,
  lib,
  libxkbcommon,
  pkg-config,
  stdenv,
  vulkan-loader,
  vulkan-validation-layers,
  wayland,
}:
let
  inherit (import ./workspace-src.nix { inherit craneLib lib; }) src;

  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    alsa-lib
    libxkbcommon
    vulkan-loader
    vulkan-validation-layers
    wayland
  ];

  env = lib.optionalAttrs stdenv.hostPlatform.isLinux {
    LD_LIBRARY_PATH = lib.makeLibraryPath buildInputs;
  };

  commonArgs = {
    inherit src buildInputs env;
    inherit (craneLib.crateNameFromCargoToml { cargoToml = ../crates/gantz/Cargo.toml; })
      pname
      version
      ;
    strictDeps = true;
    nativeBuildInputs = [ pkg-config ];
    cargoExtraArgs = "--locked -p gantz --bin gantz";
    doCheck = false;
  };
in
craneLib.buildPackage (
  commonArgs
  // {
    cargoArtifacts = craneLib.buildDepsOnly commonArgs;
  }
)
