# Short-hand for running `miniserve` to serve `gantz-website`.
{
  writeShellScriptBin,
  gantz-website,
  miniserve,
}:
writeShellScriptBin "serve-gantz-website" ''
  ${miniserve}/bin/miniserve \
    --index ${gantz-website}/index.html \
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
