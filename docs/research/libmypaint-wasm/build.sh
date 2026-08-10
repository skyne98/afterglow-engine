#!/usr/bin/env bash
# Build libmypaint NG (v2.0 master) brush engine to WebAssembly.
# Mirrors eliot-akira/brushlib-wasm's approach (emcc + proxy surface) but for
# the next-generation (v2/NG) libmypaint core, plus json-c compiled to wasm.
set -euo pipefail

MP=/tmp/libmypaint
WS=/tmp/libmypaint-wasm
JSONC_BUILD=$WS/json-c/build
OUT=$WS/out
mkdir -p "$OUT"

# Minimal NG brush-engine sources (abstract-surface + brush + mapping + settings
# + helpers + rng + brushmodes + rectangle + surface + mypaint init).
SRCS=(
  "$MP/mypaint.c"
  "$MP/mypaint-brush.c"
  "$MP/mypaint-mapping.c"
  "$MP/mypaint-brush-settings.c"
  "$MP/helpers.c"
  "$MP/rng-double.c"
  "$MP/brushmodes.c"
  "$MP/mypaint-rectangle.c"
  "$MP/mypaint-surface.c"
  "$WS/build/proxy-surface.c"
  "$WS/build/main.c"
)

# json-c wasm static lib
JSONC_LIB="$JSONC_BUILD/libjson-c.a"
JSONC_INC="$JSONC_BUILD"

emcc \
  -O3 \
  -I"$WS/build" -I"$MP" -I"$JSONC_INC" -I"$WS/json-c" \
  "${SRCS[@]}" \
  "$JSONC_LIB" \
  -o "$OUT/brushlib.js" \
  -s EXPORTED_FUNCTIONS="['_init','_stroke_to','_reset_brush','_set_brush_base_value','_set_brush_mapping_n','_set_brush_mapping_point','_new_brush']" \
  -s EXPORTED_RUNTIME_METHODS="['addFunction','ccall','cwrap']" \
  -s ALLOW_TABLE_GROWTH=1 \
  -s EXPORT_ALL=1 \
  -s NO_EXIT_RUNTIME=1 \
  -s MODULARIZE=1 \
  2>&1 | tail -30

echo "exit: $?"
ls -la "$OUT/"
