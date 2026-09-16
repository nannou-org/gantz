#!/usr/bin/env bash
# Trunk post-build hook, wired up in Trunk.toml. It prepends the TextEncoder/TextDecoder
# polyfill to the wasm-bindgen JS glue so the glue can load on the AudioWorklet thread. The
# glue is identified by its `initSync` export. A marker keeps the hook idempotent.
set -euo pipefail

dir="${TRUNK_STAGING_DIR:-${TRUNK_DIST_DIR:?TRUNK_STAGING_DIR/TRUNK_DIST_DIR not set}}"
poly="$(dirname "$0")/textcodec-polyfill.js"

for js in "$dir"/*.js; do
  [ -f "$js" ] || continue
  grep -q "initSync" "$js" || continue
  grep -q "GANTZ_TEXTCODEC_POLYFILL" "$js" && continue
  cat "$poly" "$js" >"$js.tmp"
  mv "$js.tmp" "$js"
  echo "prepended TextEncoder/TextDecoder polyfill to $(basename "$js")"
done
