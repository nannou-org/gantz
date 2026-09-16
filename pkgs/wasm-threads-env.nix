# Build environment for cpal's AudioWorklet backend on wasm32. The worklet runs on a real audio
# thread, which needs WASM threads. `-Z build-std` recompiles `std` with atomics, and the
# linear memory is shared and imported so the worklet thread gets the same memory. Both are
# nightly-only. Cargo excludes host build scripts when `--target` is set, so RUSTFLAGS applies
# only to the wasm target. The `__tls_*` exports let wasm-bindgen's threading transform set up
# per-thread state. The flags follow cpal's audioworklet-beep example. `pkgs/gantz-website.nix`
# and the `gantz-web` dev shell share this file so the flags cannot drift.
{ llvmPackages }:
{
  RUSTFLAGS = builtins.concatStringsSep " " [
    # SIMD128 vectorizes the per-sample DSP loops. Browsers that support threads support it too.
    "-C target-feature=+atomics,+simd128"
    "-C link-arg=--shared-memory"
    "-C link-arg=--max-memory=1073741824"
    "-C link-arg=--import-memory"
    "-C link-arg=--export=__heap_base"
    "-C link-arg=--export=__wasm_init_tls"
    "-C link-arg=--export=__tls_size"
    "-C link-arg=--export=__tls_align"
    "-C link-arg=--export=__tls_base"
  ];
  CARGO_UNSTABLE_BUILD_STD = "std,panic_abort";

  # `ring`, the crypto behind iroh collab sessions, compiles its C sources to
  # wasm. That needs the unwrapped clang, since the nix cc wrapper pins the
  # host target. Without it the build leaves the `ring_core_*` symbols as
  # dangling `env` imports and the browser fails to instantiate the module.
  CC_wasm32_unknown_unknown = "${llvmPackages.clang-unwrapped}/bin/clang";
  AR_wasm32_unknown_unknown = "${llvmPackages.llvm}/bin/llvm-ar";
}
