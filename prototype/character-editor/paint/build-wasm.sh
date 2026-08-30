#!/usr/bin/env bash
# Build the paint demo wasm module: the pure-Rust maipointo brush engine is
# the only engine. No C is linked into the module; C exists only in the
# exactness-test oracles (maipointo/reference/, paint/layer-compositor.*
# parity harness, paint/vendor/libmypaint).
# Run from nix-shell with the Rust wasm target:
#   RUSTC_BOOTSTRAP=1 bash paint/build-wasm.sh
set -euo pipefail

# Paths relative to the character-editor prototype root.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PAINT="$ROOT/paint"
OUT="$ROOT/public/wasm"
mkdir -p "$OUT"

(cd "$PAINT/maipointo" && RUSTC_BOOTSTRAP=1 cargo build --release \
   --features demo --target wasm32-unknown-unknown -Zbuild-std=std)
cp "$PAINT/maipointo/target/wasm32-unknown-unknown/release/maipointo.wasm" \
   "$OUT/brushlib.wasm"
cp "$OUT/brushlib.wasm" "$ROOT/src/wasm/brushlib.wasm"

# Content-stamped module URL: the loader reads brushlibWasm from the stub,
# so a rebuilt module busts the browser's wasm cache automatically.
SIZE=$(stat -c%s "$OUT/brushlib.wasm")
cat > "$OUT/brushlib.js" <<EOF
export const brushlibWasm = "/wasm/brushlib.wasm?v=$SIZE";
export default {};
EOF

echo "brushlib.wasm: $SIZE bytes"
