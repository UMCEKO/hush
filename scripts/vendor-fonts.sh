#!/usr/bin/env bash
# Vendor the UI fonts (latin woff2) from Fontsource's CDN into the app crate so
# the GUI has zero runtime font dependency. Re-run to refresh; the committed
# binaries are what actually ship.
set -euo pipefail
DEST="$(dirname "$0")/../crates/hush-app/assets/fonts"
mkdir -p "$DEST"

base="https://cdn.jsdelivr.net/fontsource/fonts"
get() { # family weight
  local url="$base/$1@latest/latin-$2-normal.woff2"
  local out="$DEST/${1}-${2}.woff2"
  echo "  $1 $2"
  curl -fsSL "$url" -o "$out"
}

for w in 400 500 600 700; do get chakra-petch "$w"; done
for w in 400 500 700; do get jetbrains-mono "$w"; done
echo "vendored $(ls "$DEST" | wc -l) fonts into $DEST"
