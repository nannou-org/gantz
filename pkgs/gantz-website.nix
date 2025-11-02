# Combines `gantz-wasm-bindgen` with an index.html and some js.
{
  lib,
  stdenv,
  gantz-wasm-bindgen,
}:
stdenv.mkDerivation {
  pname = "gantz-website";
  version = gantz-wasm-bindgen.version;
  src = lib.sourceFilesBySuffices ./.. [
    ".html"
    ".css"
    ".js"
    ".json"
    ".png"
    ".svg"
    ".ico"
  ];
  buildPhase = ''
    mkdir -p $out
    cp -r crates/gantz/web/* $out/
    cp -r ${gantz-wasm-bindgen}/lib/* $out/
  '';
  dontInstall = true;
}
