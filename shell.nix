{
  gantz-unwrapped,
  gantz-wasm,
  lib,
  libGL,
  mkShell,
  stdenv,
}:
mkShell {
  name = "gantz-dev";
  inputsFrom = [
    gantz-unwrapped
    gantz-wasm
  ];
  # FIXME: Remove this, see #122.
  buildInputs = [
    libGL
  ];
  env = lib.optionalAttrs stdenv.isLinux {
    # FIXME: Switch back when #122 is resolved.
    # inherit (gantz-unwrapped) LD_LIBRARY_PATH;
    LD_LIBRARY_PATH = gantz-unwrapped.LD_LIBRARY_PATH + ":${libGL}/lib";
  };
}
