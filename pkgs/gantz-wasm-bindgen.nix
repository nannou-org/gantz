{
  binaryen,
  gantz-wasm,
  stdenv,
  wasm-bindgen-cli,
}:
stdenv.mkDerivation {
  pname = "gantz-wasm-bindgen";
  version = gantz-wasm.version;
  dontUnpack = true;
  dontInstall = true;
  dontFixup = true;

  nativeBuildInputs = [
    binaryen
    wasm-bindgen-cli
  ];

  buildPhase = ''
    # Run wasm-bindgen directly on the store path
    echo "Generating bindings for ${gantz-wasm}/lib/gantz.wasm"
    wasm-bindgen \
      --target web \
      --out-dir $out/lib \
      --out-name gantz \
      ${gantz-wasm}/lib/gantz.wasm

    # Additional optimization with wasm-opt
    echo "Optimising $out/lib/gantz_bg.wasm"
    wasm-opt \
      -Oz \
      --enable-bulk-memory \
      --enable-reference-types \
      --enable-multivalue \
      --enable-tail-call \
      --enable-nontrapping-float-to-int \
      $out/lib/gantz_bg.wasm \
      -o $out/lib/gantz_bg_opt.wasm

    # Replace original with optimized.
    rm $out/lib/gantz_bg.wasm
    mv $out/lib/gantz_bg_opt.wasm $out/lib/gantz_bg.wasm
  '';
}
