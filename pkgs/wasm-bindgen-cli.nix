# wasm-bindgen-cli version must match the exact version of wasm-bindgen used
# within the crate dependencies. nixpkgs' version doesn't always match the
# latest version picked up in our Cargo.lock, so here we pin to a particular
# wasm-bindgen-cli so we can override the nixpkgs version.
{
  buildWasmBindgenCli,
  fetchCrate,
  rustPlatform,
}:
buildWasmBindgenCli rec {
  src = fetchCrate {
    pname = "wasm-bindgen-cli";
    version = "0.2.105";
    hash = "sha256-zLPFFgnqAWq5R2KkaTGAYqVQswfBEYm9x3OPjx8DJRY=";
  };
  cargoDeps = rustPlatform.fetchCargoVendor {
    inherit src;
    inherit (src) pname version;
    hash = "sha256-a2X9bzwnMWNt0fTf30qAiJ4noal/ET1jEtf5fBFj5OU=";
  };
}
