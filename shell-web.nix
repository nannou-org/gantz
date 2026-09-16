# Dev shell for the AudioWorklet web build with `trunk serve`. It uses the nightly toolchain
# and the WASM-threads build flags, so cargo here recompiles `std` with atomics via
# `-Z build-std`. Run native `cargo` commands in the default `gantz-dev` shell instead. The
# build-std flags here make a host build fail.
{
  binaryen,
  lld,
  miniserve,
  llvmPackages,
  mkShell,
  rustToolchainWasmNightly,
  trunk,
  wasm-bindgen-cli,
}:
mkShell (
  {
    name = "gantz-web-dev";
    nativeBuildInputs = [
      rustToolchainWasmNightly
      binaryen
      lld
      trunk
      wasm-bindgen-cli
      miniserve
    ];
  }
  // (import ./pkgs/wasm-threads-env.nix { inherit llvmPackages; })
)
