{
  gantz-unwrapped,
  gantz-website,
  lib,
  libGL,
  mkShell,
  rustfmt,
  stdenv,
  trunk,
  binaryen,
  wasm-bindgen-cli,
}:
mkShell {
  name = "gantz-dev";
  inputsFrom = [
    gantz-unwrapped
    gantz-website
  ];
  # The rust toolchain comes via `inputsFrom` (gantz-unwrapped) but does not
  # include rustfmt, which `nix develop -c cargo fmt` (and CI) requires.
  packages = [
    rustfmt
  ];
  # FIXME: Remove this, see #122.
  buildInputs = [
    libGL
    trunk
    binaryen
    wasm-bindgen-cli
  ];
  env = lib.optionalAttrs stdenv.isLinux {
    # FIXME: Switch back when #122 is resolved.
    # inherit (gantz-unwrapped) LD_LIBRARY_PATH;
    LD_LIBRARY_PATH = gantz-unwrapped.LD_LIBRARY_PATH + ":${libGL}/lib";
  };
}
