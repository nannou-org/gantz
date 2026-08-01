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
  root = ../.;
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (craneLib.fileset.commonCargoSources root)
      (lib.fileset.fileFilter (file: file.hasExt "gantz") root)
    ];
  };

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
