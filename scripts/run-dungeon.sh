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
# Resident (non-VT) textures: 8-bit R8 height field (cooked from the lossless
# .r16 interchange) + a blue-noise dither tile, both in v6 `.big` containers.
# Height stays out of VT so the POM march pays one direct mip-0 fetch per step.
HEIGHT_BIG="$ROOT/crates/afterglow-web/web/assets/dungeon-height.big"
BLUE_BIG="$ROOT/crates/afterglow-web/web/assets/blue-noise.big"
if [[ ! -s "$HEIGHT_BIG" ]]; then
  nix-shell shell.nix --run "cargo run --release -p afterglow-pipeline -- resident-texture \
    $ROOT/crates/afterglow-web/web/assets/dungeon-height/Rock064_Height.r16 \
    $ROOT/crates/afterglow-web/web/assets/dungeon-height/Ground103_Height.r16 \
    $ROOT/crates/afterglow-web/web/assets/dungeon-height/PavingStones150_Height.r16 \
    '$HEIGHT_BIG' --format r8"
fi
if [[ ! -s "$BLUE_BIG" ]]; then
  nix-shell shell.nix --run "cargo run --release -p afterglow-pipeline -- blue-noise 64 '$BLUE_BIG' --name blue-noise"
fi
nix-shell shell.nix --run "cargo run -p xtask -- wasm --release"
exec nix-shell shell.nix --run "cargo build -p afterglow-cef --example dungeon && DISPLAY=${DISPLAY:-:0} ./target/debug/examples/dungeon --ozone-platform=x11"
