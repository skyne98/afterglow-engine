# WASM paint surface

Status: prototype implementation

The character-editor paint demo uses the vendored NG libmypaint tiled surface. The brush engine runs entirely in a Web Worker (`paint-brush-worker.ts`) that also owns the display `OffscreenCanvas` and the motion queue. The main thread only forwards input and configuration, so the brush engine never blocks the render thread.
The browser does not receive `draw_dab` callbacks. The demo uses `getCoalescedEvents()` so fast strokes do not lose pointer samples.
The worker owns the `MotionQueue` and processes every received sample on its own thread, so a burst of pointer moves never blocks input and no samples are lost.
The worker fills the display canvas with the uniform background color once, then draws only the mip tiles whose source regions contain paint. Regions with paint use an exact box filter that keeps the background.

## Current API

The WASM module is built by:

```sh
cd prototype/character-editor
nix-shell -p emscripten --run 'bash paint/build-wasm.sh'
```

The build uses `-O3` and `-msimd128`. `wasm-opt -all --print-features public/wasm/brushlib.wasm` reports WebAssembly SIMD.

The public C exports include:

- `init(width, height)`
- `paint_destroy()`
- `load_brush(json)`
- `begin_stroke(...)`
- `stroke_to(..., linear)`
- `paint_get_tile_ptr(tx, ty)`
- `paint_render_tile_ptr(tx, ty)`
- `paint_render_rgba8_tile_ptr(tx, ty)`
- `paint_render_rgba8_mip_tile_ptr(tx, ty, level)` for levels `1..2`
- `paint_render_layer_rgba8_tile_ptr(layer, tx, ty)`
- `paint_write_rgba8_tile(tx, ty, rgba8)`
- `paint_set_eotf(value)`
- `paint_get_error_code()`
- `paint_clear_error()`
- `paint_get_dirty_count()`
- `paint_get_dirty_rect(index, out)`
- `paint_clear_dirty()`
- `paint_get_used_tile_count()`
- `paint_pick_color(...)`
- `paint_create_layer()`
- `paint_delete_layer(layer)`
- `paint_set_active_layer(layer)`
- `paint_set_layer_visible(layer, visible)`
- `paint_set_layer_opacity(layer, opacity)`
- `paint_get_layer_opacity(layer)`
- `paint_set_layer_mode(layer, mode)` uses the MyPaint mode IDs `0..21`, with Pigment at `21`.
- `paint_set_layer_group(layer, group)`
- `paint_move_layer(layer, direction)`
- `paint_create_group()` and `paint_delete_group(group)`
- `paint_set_group_parent(group, parent)`
- `paint_set_group_visible(group, visible)`
- `paint_set_group_opacity(group, opacity)`
- `paint_set_group_mode(group, mode)`
- `paint_set_group_pass_through(group, value)`
- `paint_set_group_isolated(group, value)`
- `paint_move_group(group, direction)`
- `paint_set_background_color(r, g, b)`
- `paint_clear_background()`
- `paint_history_begin()`
- `paint_history_commit()`
- `paint_history_undo()`
- `paint_history_redo()`

The internal paint pixels use 64 x 64 RGBA16 premultiplied tiles.
The full channel value is `32768`.
The output path un-premultiplies, applies EOTF `2.2`, applies fixed dither, and writes RGBA8 display tiles.

## Current behavior

The surface supports the libmypaint dab mask, real surface color sampling, brush eraser and smudge paths, brush lock-alpha, colorize, posterize, spectral brush paint, symmetry data, and 22 layer mode selectors.
The layer compositor supports bounded paint layers, visibility, opacity, standard separable modes, W3C nonseparable modes, Porter-Duff modes, and pigment mode.
The default layer mode is Pigment, and the default background is the original MyPaint fallback color `#A8A498`.
The demo supports up to 40 undo and redo records for active-layer tile changes.
The history callback stores only the tiles a stroke writes, and the snapshot pool grows on demand, so a large stroke never fails to create an undo record except on true out-of-memory.
The page view supports zoom, pan, quarter-turn rotation, horizontal mirror, pixelized high zoom, and box-filtered low-zoom mip tiles without changing document pixels.
Pointer input removes the CSS translation component before inverse mapping, so camera pan does not offset strokes.
The layer compositor is a clean-room implementation of the flat MyPaint modes.
It uses 15-bit integer compositing, the ten-channel WGM path, MyPaint mode IDs, and the published nonseparable color operations.
The fixed tree supports eight paint layers and four groups, nested group parents, layer reparenting, ordering, visibility, opacity, blend mode, pass-through, and isolated rendering.
The brush and dab path uses the vendored ISC libmypaint source at commit `d5a88fbe6649d5ec776bc42ec8c1f4bb29d7fd7f`, with a replacement fixed operation queue.

The browser document size is configurable from `64` through `16384` pixels per axis.
The current surface uses a sparse signed-coordinate tile map with a default `2048 x 2048` display document.
Documents above `4096` pixels use a reduced display backing store and level-2 display tiles. The display code places each mip tile at its reduced backing-store size, so a 16K document can show paint at the correct location.
The frame checkbox records the view choice only.
Each surface holds as many tiles as its full document needs; a painted tile is allocated on demand the first time it is touched, so memory scales with use and no fixed `4096` ceiling remains.
Signed sparse unframed storage and bounded base-to-mip display generation are active.
The history pool is growable tile-snapshot storage, not the final generational copy-on-write root system.
The demo shows a HUD with the input queue state: queued samples, brush and render times, and the input sample rate.
The demo exports internal RGBA8 tiles to PNG without view transforms.
The demo exports and imports bounded OpenRaster ZIP files with merged image data, layer PNGs, stack XML, and Afterglow layer/group metadata.

## Ownership and limits

The WASM engine owns brush state and all document pixels and runs in the worker.
TypeScript in the worker owns pressure and tilt interpolation and display tile drawing. The main thread owns input capture, DOM controls, brush metadata, and brush color restore.
The current layer limit is eight paint layers.
Each created layer allocates its tiles on demand, so memory grows only as a layer is painted.

The browser operation queue uses fixed storage for 8192 operations and 4096 dirty tile keys.
Low-zoom display work reuses sixteen source tiles per mip tile and skips view-only redraws when the selected mip level does not change.
The WASM display path uses a fixed EOTF and dither lookup table, so tile output does not call `powf` for each color channel.
Tile memory is bounded only by the WASM growth ceiling (set to the 4 GiB wasm32 maximum), so painting does not abort when it fills the document.
The flat compositor checks run with `paint/layer-compositor.test.c` and native `cc` before the WASM build.
