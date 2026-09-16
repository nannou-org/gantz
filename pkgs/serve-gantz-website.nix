# Serves the `gantz-website` locally with the COOP/COEP headers its SharedArrayBuffer needs.
# With real headers the bundled coi-serviceworker is a no-op. It only matters on hosts that
# cannot send headers, for example GitHub Pages.
{
  writeShellScriptBin,
  gantz-website,
  miniserve,
}:
writeShellScriptBin "serve-gantz-website" ''
  ${miniserve}/bin/miniserve \
    --index index.html \
    --disable-indexing \
    --hide-version-footer \
    --hide-theme-selector \
    --header "Cross-Origin-Opener-Policy:same-origin" \
    --header "Cross-Origin-Embedder-Policy:require-corp" \
    --header "Cache-Control:no-store, no-cache, must-revalidate" \
    --header "Pragma:no-cache" \
    --header "Expires:0" \
    -i 0.0.0.0 \
    --port 8088 \
    ${gantz-website}
''
