{
  binaryen,
  lib,
  lld,
  rustPlatform,
  trunk,
  wasm-bindgen-cli,
  writableTmpDirAsHomeHook,
}:
let
  src = lib.sourceFilesBySuffices ../. [
    ".gantz"
    ".lock"
    ".rs"
    ".toml"
    ".html"
    ".css"
    ".js"
    ".json"
    ".png"
    ".svg"
    ".ico"
  ];
in
rustPlatform.buildRustPackage {
  pname = "gantz-website";
  version = "0.1.0";
  inherit src;
  cargoLock = {
    outputHashes = {
      "steel-core-0.8.2" = "sha256-qlGG7BWgg6mQifj80Ycm5P7T2TQUM2OppH91fKFT57A=";
    };
    lockFile = ../Cargo.lock;
  };
  doCheck = false;
  dontFixup = true;

  nativeBuildInputs = [
    binaryen
    lld
    trunk
    wasm-bindgen-cli
    writableTmpDirAsHomeHook
  ];

  # Tell trunk to use Nix-provided tools, not download its own.
  TRUNK_SKIP_VERSION_CHECK = "true";

  # buildRustPackage's configurePhase sets up cargo vendoring.
  # Override buildPhase to call trunk instead of cargo directly.
  buildPhase = ''
    trunk build --release --dist $out
  '';

  installPhase = "true";
}
