#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${TMPDIR:-/tmp}/afterglow-vt-dungeon-materials-v3"
SOURCES="$CACHE/sources"
BIG="$CACHE/vt-dungeon.big"
cd "$ROOT"
if [[ ! -s "$BIG" ]]; then
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
fi
# CEF confinement deliberately cannot escape its asset root. Publish the cached
# immutable container; generation and all source data remain in /tmp.
cp -f "$BIG" crates/afterglow-web/www/vt-dungeon.big
nix-shell shell.nix --run "cargo run -p xtask -- wasm --release"
exec nix-shell shell.nix --run "cargo build -p afterglow-cef --example vt-dungeon && DISPLAY=${DISPLAY:-:0} ./target/debug/examples/vt-dungeon --ozone-platform=x11"
