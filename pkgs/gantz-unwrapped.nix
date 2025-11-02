{
  lib,
  libxkbcommon,
  makeWrapper,
  pkg-config,
  rustPlatform,
  stdenv,
  vulkan-loader,
  vulkan-validation-layers,
  wayland,
}:
let
  src = lib.sourceFilesBySuffices ../. [
    ".lock"
    ".rs"
    ".toml"
  ];
  buildAndTestSubdir = "crates/gantz";
  manifestPath = "${src}/${buildAndTestSubdir}/Cargo.toml";
  manifest = builtins.fromTOML (builtins.readFile manifestPath);

  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    libxkbcommon
    vulkan-loader
    vulkan-validation-layers
    wayland
  ];

  env = lib.optionalAttrs stdenv.hostPlatform.isLinux {
    LD_LIBRARY_PATH = lib.makeLibraryPath buildInputs;
  };

in
rustPlatform.buildRustPackage {
  inherit src buildAndTestSubdir;
  pname = manifest.package.name;
  version = manifest.package.version;
  cargoLock.lockFile = ../Cargo.lock;
  nativeBuildInputs = [
    makeWrapper
    pkg-config
  ];
  inherit buildInputs env;
}
