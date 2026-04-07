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
    ".gantz"
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
  cargoLock = {
    outputHashes = {
      "steel-core-0.8.2" = "sha256-qlGG7BWgg6mQifj80Ycm5P7T2TQUM2OppH91fKFT57A=";
    };
    lockFile = ../Cargo.lock;
  };
  cargoBuildFlags = [
    "--bin"
    "gantz"
  ];
  doCheck = false;
  nativeBuildInputs = [
    makeWrapper
    pkg-config
  ];
  inherit buildInputs env;
}
