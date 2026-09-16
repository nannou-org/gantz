{
  cargo-semver-checks,
  gantz-unwrapped,
  gantz-website,
  lib,
  libGL,
  mkShell,
  release-plz,
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
  # The rust toolchain comes via `inputsFrom` from gantz-unwrapped. It does not
  # include rustfmt, which `nix develop -c cargo fmt` and CI require.
  # `release-plz` drives the release process in .github/workflows/release-plz.yml.
  # Running it from this shell reuses the native build deps that `cargo publish`'s
  # verify build needs. It also lets maintainers preview a release with
  # `nix develop -c release-plz update`. release-plz shells out to
  # `cargo-semver-checks` for the `semver_check` step in release-plz.toml.
  packages = [
    cargo-semver-checks
    release-plz
    rustfmt
  ];
  # FIXME: Remove this when #122 is resolved.
  buildInputs = [
    libGL
    trunk
    binaryen
    wasm-bindgen-cli
  ];
  env = lib.optionalAttrs stdenv.isLinux {
    # FIXME: Use the gantz-unwrapped LD_LIBRARY_PATH when #122 is resolved.
    # inherit (gantz-unwrapped) LD_LIBRARY_PATH;
    LD_LIBRARY_PATH = gantz-unwrapped.LD_LIBRARY_PATH + ":${libGL}/lib";
  };
}
