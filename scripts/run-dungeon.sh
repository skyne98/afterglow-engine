#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${TMPDIR:-/tmp}/afterglow-dungeon-materials-v3"
SOURCES="$CACHE/sources"
BIG="$CACHE/dungeon.big"
PUBLISHED="$ROOT/crates/afterglow-web/web/assets/dungeon.big"
cd "$ROOT"
if [[ ! -s "$PUBLISHED" ]]; then
  rm -rf "$CACHE"; mkdir -p "$SOURCES"
  for material in Rock064 Ground103 PavingStones150; do
    archive="$HOME/Downloads/${material}_8K-PNG.zip"
    [[ -s "$archive" ]] || { echo "missing $archive" >&2; exit 1; }
    for channel in Color NormalGL Roughness AmbientOcclusion; do
      unzip -p "$archive" "${material}_8K-PNG_${channel}.png" >"$SOURCES/${material}_${channel}.png"
    done
    nix-shell shell.nix --run "cargo run --release -p afterglow-pipeline -- pack-masks '$SOURCES/${material}_Roughness.png' '$SOURCES/${material}_AmbientOcclusion.png' '$SOURCES/${material}_Masks.png'"
    rm "$SOURCES/${material}_Roughness.png" "$SOURCES/${material}_AmbientOcclusion.png"
  done
  nix-shell shell.nix --run "cargo run --release -p afterglow-pipeline -- process '$SOURCES' '$BIG'"
  # CEF confinement deliberately cannot escape its asset root. Publish the
  # cached immutable container; source data remains in /tmp.
  cp -f "$BIG" "$PUBLISHED"
fi
nix-shell shell.nix --run "cargo run -p xtask -- wasm --release"
exec nix-shell shell.nix --run "cargo build -p afterglow-cef --example dungeon && DISPLAY=${DISPLAY:-:0} ./target/debug/examples/dungeon --ozone-platform=x11"
