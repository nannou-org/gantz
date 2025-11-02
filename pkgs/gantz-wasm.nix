{
  lib,
  lld,
  rustPlatform,
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
  pname = manifest.package.name;
  buildType = "wasm_release";
  wasm-target = "wasm32-unknown-unknown";
in
rustPlatform.buildRustPackage {
  inherit src buildAndTestSubdir pname;
  version = manifest.package.version;
  cargoLock.lockFile = ../Cargo.lock;
  doCheck = false;
  depsBuildBuild = [
    lld
  ];
  buildPhase = ''
    cargo build -p "${pname}" --profile "${buildType}" --target "${wasm-target}"
  '';
  installPhase = ''
    mkdir -p $out/lib
    cp target/${wasm-target}/${buildType}/gantz.wasm $out/lib/
  '';
}
