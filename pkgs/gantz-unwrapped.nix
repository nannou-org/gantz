# Built with crane rather than `buildRustPackage` so that the dependency
# closure compiles once into a deps-only artifact derivation (`buildDepsOnly`,
# built against stubbed sources) that survives workspace source edits - only
# the workspace crates recompile on change.
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
  # Cargo-relevant files (Cargo.toml/lock, *.rs, *.toml - including
  # .cargo/config.toml) plus the .gantz assets that gantz_base and
  # gantz_plyphon include at compile time.
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (craneLib.fileset.commonCargoSources root)
      (lib.fileset.fileFilter (file: file.hasExt "gantz") root)
    ];
  };

  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    # `cpal` (via `bevy_gantz_plyphon`) links ALSA on Linux for audio output.
    alsa-lib
    libxkbcommon
    vulkan-loader
    vulkan-validation-layers
    wayland
  ];

  env = lib.optionalAttrs stdenv.hostPlatform.isLinux {
    LD_LIBRARY_PATH = lib.makeLibraryPath buildInputs;
  };

  # Shared verbatim between the deps and package derivations: any divergence in
  # cargo flags, RUSTFLAGS or profile between the two invalidates cargo's
  # fingerprints and silently recompiles the whole dep closure in the final
  # derivation.
  commonArgs = {
    inherit src buildInputs env;
    inherit (craneLib.crateNameFromCargoToml { cargoToml = ../crates/gantz/Cargo.toml; })
      pname
      version
      ;
    strictDeps = true;
    nativeBuildInputs = [ pkg-config ];
    # crane's default is just "--locked"; keep it when overriding.
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
