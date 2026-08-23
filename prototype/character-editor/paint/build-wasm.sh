#!/usr/bin/env bash
# Build the NG libmypaint brush engine to WebAssembly for the paint demo.
# Run from nix-shell with emscripten:
#   nix-shell -p emscripten --run 'bash paint/build-wasm.sh'
set -euo pipefail

# Paths relative to the character-editor prototype root.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PAINT="$ROOT/paint"
VENDOR="$PAINT/vendor"
OUT="$ROOT/public/wasm"
mkdir -p "$OUT"

MP="$VENDOR/libmypaint"
JSONC="$VENDOR/json-c"
JSONC_BUILD="$JSONC/build"

SRCS=(
  "$MP/mypaint.c"
  "$PAINT/mypaint-brush-cooperative.c"
  "$MP/mypaint-mapping.c"
  "$MP/mypaint-brush-settings.c"
  "$MP/helpers.c"
  "$MP/rng-double.c"
  "$MP/brushmodes.c"
  "$PAINT/fixed-operationqueue.c"
  "$MP/mypaint-rectangle.c"
  "$MP/mypaint-matrix.c"
  "$MP/mypaint-symmetry.c"
  "$MP/mypaint-surface.c"
  "$MP/mypaint-tiled-surface.c"
  "$PAINT/web-surface.c"
  "$PAINT/layer-compositor.c"
  "$PAINT/main.c"
)

# 1) Cross-compile json-c to wasm (static).
if [ ! -f "$JSONC_BUILD/libjson-c.a" ]; then
  mkdir -p "$JSONC_BUILD"
  emcmake cmake -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
    -DBUILD_TESTING=OFF -DCMAKE_C_FLAGS="-O3" -S "$JSONC" -B "$JSONC_BUILD" >/dev/null
  emmake make -C "$JSONC_BUILD" -j"$(nproc)" >/dev/null
fi

# 2) Compile libmypaint NG + tiled surface + compositor + wrapper to one module.
# Keep the release optimization level and enable WebAssembly SIMD128 for C code.
emcc \
  -O3 \
  -msimd128 \
  -I"$PAINT" -I"$MP" -I"$JSONC_BUILD" -I"$JSONC" -I"$VENDOR/openmp-wasm" \
  "${SRCS[@]}" \
  "$JSONC_BUILD/libjson-c.a" \
  -o "$OUT/brushlib.js" \
  -s EXPORTED_FUNCTIONS="['_malloc','_free','_init','_paint_destroy','_stroke_to','_reset_brush','_set_brush_base_value','_get_brush_base_value','_set_brush_mapping_n','_set_brush_mapping_point','_new_brush','_load_brush','_begin_stroke','_paint_begin_atomic','_paint_end_atomic','_paint_begin_batch','_paint_end_batch','_paint_is_batch_done','_paint_end_batch_finish','_paint_continue_stroke_to','_paint_has_stroke_continuation','_paint_get_width','_paint_get_height','_paint_get_error_code','_paint_clear_error','_paint_get_tiles_width','_paint_get_tiles_height','_paint_get_used_tile_count','_paint_get_tile_ptr','_paint_render_tile_ptr','_paint_set_eotf','_paint_render_rgba8_tile_ptr','_paint_render_layer_rgba8_tile_ptr','_paint_write_rgba8_tile','_paint_render_rgba8_mip_tile_ptr','_paint_region_has_paint','_paint_get_dirty_count','_paint_get_dirty_rect','_paint_get_dirty_tile_count','_paint_get_dirty_tile_info','_paint_clear_dirty','_paint_set_background_color','_paint_clear_background','_paint_history_begin','_paint_history_commit','_paint_history_undo','_paint_history_redo','_paint_history_can_undo','_paint_history_can_redo','_paint_clear','_paint_pick_color','_paint_set_symmetry','_paint_get_layer_count','_paint_get_active_layer','_paint_set_active_layer','_paint_create_layer','_paint_delete_layer','_paint_set_layer_visible','_paint_set_layer_opacity','_paint_get_layer_opacity','_paint_get_layer_mode','_paint_set_layer_mode','_paint_get_layer_visible','_paint_get_layer_group','_paint_set_layer_group','_paint_move_layer','_paint_get_group_count','_paint_get_group_alive','_paint_get_group_parent','_paint_create_group','_paint_delete_group','_paint_set_group_parent','_paint_get_group_visible','_paint_set_group_visible','_paint_get_group_opacity','_paint_set_group_opacity','_paint_get_group_mode','_paint_set_group_mode','_paint_get_group_pass_through','_paint_set_group_pass_through','_paint_get_group_isolated','_paint_set_group_isolated','_paint_move_group']" \
  -s EXPORTED_RUNTIME_METHODS="['addFunction','ccall','cwrap']" \
  -s ALLOW_TABLE_GROWTH=1 \
  -s ALLOW_MEMORY_GROWTH=1 \
  -s INITIAL_MEMORY=67108864 \
  -s MAXIMUM_MEMORY=4294967296 \
  -s EXPORT_ALL=1 \
  -s NO_EXIT_RUNTIME=1 \
  -s MODULARIZE=1

# Keep the Vite import copy in sync with the public build output.
printf '\nexport default Module;\n' >> "$OUT/brushlib.js"
mkdir -p "$ROOT/src/wasm"
cp "$OUT/brushlib.js" "$ROOT/src/wasm/brushlib.js"
cp "$OUT/brushlib.wasm" "$ROOT/src/wasm/brushlib.wasm"

ls -la "$OUT/"
