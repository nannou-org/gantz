{
  gantz-unwrapped,
  gantz-wasm,
  lib,
  mkShell,
  stdenv,
}:
mkShell {
  name = "gantz-dev";
  inputsFrom = [
    gantz-unwrapped
    gantz-wasm
  ];
  env = lib.optionalAttrs stdenv.isLinux {
    inherit (gantz-unwrapped) LD_LIBRARY_PATH;
  };
}
