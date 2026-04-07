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
    version = "0.2.114";
    hash = "sha256-xrCym+rFY6EUQFWyWl6OPA+LtftpUAE5pIaElAIVqW0=";
  };
  cargoDeps = rustPlatform.fetchCargoVendor {
    inherit src;
    inherit (src) pname version;
    hash = "sha256-Z8+dUXPQq7S+Q7DWNr2Y9d8GMuEdSnq00quUR0wDNPM=";
  };
}
