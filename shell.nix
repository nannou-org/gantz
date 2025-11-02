{
  gantz-unwrapped,
  lib,
  mkShell,
  stdenv,
}:
mkShell {
  name = "gantz-dev";
  inputsFrom = [
    gantz-unwrapped
  ];
  env = lib.optionalAttrs stdenv.isLinux {
    inherit (gantz-unwrapped) LD_LIBRARY_PATH;
  };
}
