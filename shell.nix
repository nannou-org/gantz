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
  # The rust toolchain comes via `inputsFrom` (gantz-unwrapped) but does not
  # include rustfmt, which `nix develop -c cargo fmt` (and CI) requires.
  # `release-plz` drives the release process (see .github/workflows/release-plz.yml);
  # running it from this shell reuses the native build deps that `cargo publish`'s
  # verify build needs, and lets maintainers preview a release with
  # `nix develop -c release-plz update`. `cargo-semver-checks` is the binary
  # release-plz shells out to for `semver_check` (release-plz.toml).
  packages = [
    cargo-semver-checks
    release-plz
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
