#!/usr/bin/env bash
# Build the NG libmypaint brush engine to WebAssembly with threads (OpenMP)
# using the emsdk Emscripten (not the broken Nix 6.0.2). Run from a shell
# where python3 is on PATH and EMSDK env is sourced:
#   nix-shell -p python3 --run 'source /home/fox/tools/emsdk/emsdk_env.sh && bash paint/build-wasm-threads.sh'
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PAINT="$ROOT/paint"
VENDOR="$PAINT/vendor"
OUT="$ROOT/public/wasm"
mkdir -p "$OUT"

MP="$VENDOR/libmypaint"
JSONC="$VENDOR/json-c"
JSONC_BUILD="$JSONC/build-threads"

SRCS=(
  "$MP/mypaint.c"
  "$MP/mypaint-brush.c"
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
  "$PAINT/web-surface-threads.c"
  "$PAINT/layer-compositor.c"
  "$PAINT/main.c"
)

# json-c cross-compiled for threads (must carry atomics/bulk-memory).
if [ ! -f "$JSONC_BUILD/libjson-c.a" ]; then
  mkdir -p "$JSONC_BUILD"
  emcmake cmake -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
    -DBUILD_TESTING=OFF -DCMAKE_C_FLAGS="-O3 -pthread" -S "$JSONC" -B "$JSONC_BUILD" >/dev/null
  emmake make -C "$JSONC_BUILD" -j"$(nproc)" >/dev/null
fi

EXPORTS="['_malloc','_free','_init','_paint_destroy','_stroke_to','_reset_brush','_set_brush_base_value','_get_brush_base_value','_set_brush_mapping_n','_set_brush_mapping_point','_new_brush','_load_brush','_begin_stroke','_paint_begin_atomic','_paint_end_atomic','_paint_begin_batch','_paint_end_batch','_paint_is_batch_done','_paint_end_batch_finish','_paint_get_width','_paint_get_height','_paint_get_error_code','_paint_clear_error','_paint_get_tiles_width','_paint_get_tiles_height','_paint_get_used_tile_count','_paint_get_tile_ptr','_paint_render_tile_ptr','_paint_set_eotf','_paint_render_rgba8_tile_ptr','_paint_render_layer_rgba8_tile_ptr','_paint_write_rgba8_tile','_paint_render_rgba8_mip_tile_ptr','_paint_region_has_paint','_paint_get_dirty_count','_paint_get_dirty_rect','_paint_clear_dirty','_paint_set_background_color','_paint_clear_background','_paint_history_begin','_paint_history_commit','_paint_history_undo','_paint_history_redo','_paint_history_can_undo','_paint_history_can_redo','_paint_clear','_paint_pick_color','_paint_set_symmetry','_paint_get_layer_count','_paint_get_active_layer','_paint_set_active_layer','_paint_create_layer','_paint_delete_layer','_paint_set_layer_visible','_paint_set_layer_opacity','_paint_get_layer_opacity','_paint_get_layer_mode','_paint_set_layer_mode','_paint_get_layer_visible','_paint_get_layer_group','_paint_set_layer_group','_paint_move_layer','_paint_get_group_count','_paint_get_group_alive','_paint_get_group_parent','_paint_create_group','_paint_delete_group','_paint_set_group_parent','_paint_get_group_visible','_paint_set_group_visible','_paint_get_group_opacity','_paint_set_group_opacity','_paint_get_group_mode','_paint_set_group_mode','_paint_get_group_pass_through','_paint_set_group_pass_through','_paint_get_group_isolated','_paint_set_group_isolated','_paint_move_group']"

emcc \
  -O2 \
  -msimd128 \
  -pthread \
  -DWEB_USE_THREADS \
  -s PTHREAD_POOL_SIZE=4 \
  -s PTHREAD_POOL_SIZE_STRICT=0 \
  -I"$PAINT" -I"$MP" -I"$JSONC_BUILD" -I"$JSONC" -I"$VENDOR/openmp-wasm" \
  "${SRCS[@]}" \
  "$JSONC_BUILD/libjson-c.a" \
  -o "$OUT/brushlib.js" \
  -s EXPORTED_FUNCTIONS="$EXPORTS" \
  -s MODULARIZE=1 \
  -s EXPORT_NAME=createBrushlib \
  -s EXPORT_ES6=1 \
  -s INITIAL_MEMORY=268435456 \
  -s MAXIMUM_MEMORY=1073741824 \
  -s ALLOW_MEMORY_GROWTH=1 \
  -s NO_EXIT_RUNTIME=1 \
  -s EXPORTED_RUNTIME_METHODS="['stringToUTF8','lengthBytesUTF8','ccall','cwrap']"

mkdir -p "$ROOT/src/wasm"
cp "$OUT/brushlib.js" "$ROOT/src/wasm/brushlib.js"
cp "$OUT/brushlib.wasm" "$ROOT/src/wasm/brushlib.wasm"

ls -la "$OUT/"
