# WASM paint surface

Status: prototype implementation

The character-editor paint demo uses the NG libmypaint tiled surface. The vendored source is at commit `d5a88fbe6649d5ec776bc42ec8c1f4bb29d7fd7f`.

`paint-engine-worker.ts` owns the WebAssembly module, motion queue, brush state, document pixels, and `OffscreenCanvas`. The page sends input and control messages.

## Build

Build the threaded module with Emscripten 3.1.74:

```sh
cd prototype/character-editor
nix-shell -p python3 --run \
  'source /home/fox/tools/emsdk/emsdk_env.sh && bash paint/build-wasm-threads.sh'
```

The build uses four pthread workers, SIMD128, a 128 KiB pthread stack, and a 1 GiB memory limit.

`paint/build-wasm.sh` makes a separate serial module. The threaded build is the shipped demo module.

## Brush processing

`stroke_to()` starts one exact libmypaint input sample. It processes a maximum of 128 dabs and returns one of these values:

- `0`: More dabs remain.
- `1` or `2`: The input sample is complete.
- A negative value: The brush state has an error.

`paint_continue_stroke_to()` processes the next 128 dabs. `paint_has_stroke_continuation()` reports the continuation state.

The continuation does not add input points. It resumes inside the original libmypaint dab loop.

The original and cooperative paths have equal brush states and output bytes in `mypaint-brush-cooperative.test.c`.

The TypeScript worker keeps the current `MotionQueue` sample until its continuation completes. It yields between batches, so later worker messages can enter fixed queues.

## Parallel tile processing

`paint_begin_batch()` starts an atomic tile batch. `paint_end_batch()` creates missing tiles before it starts any pthread.

A maximum of four pthreads claim separate dirty tiles. Each pthread runs the unchanged libmypaint `process_tile()` function.

`paint_is_batch_done()` polls counters and `pthread_tryjoin_np()`. The main WASM worker does not use a blocking join.

`paint_end_batch_finish()` merges dirty areas and clears the operation queue. It runs only after all joinable pthreads exit.

Workers cannot create tile memory. This rule prevents a proxied memory-growth deadlock in a pthread.

## Fixed operation queue

The operation queue has these limits:

- 4,096 dirty tile keys for each batch
- 16,384 dab operations for each batch
- 8,192 fixed hash entries for O(1) tile lookup

A capacity failure sets error code `4` before the engine clears the queue. The engine does not silently use a serial mode.

## Dirty display data

The C surface keeps both dirty rectangles and exact dirty tile slots.

Use these exports:

- `paint_get_dirty_count()`
- `paint_get_dirty_rect(index, out)`
- `paint_get_dirty_tile_count()`
- `paint_get_dirty_tile_info(index, out)`
- `paint_clear_dirty()`

The display worker renders exact dirty tiles for brush changes. Thus, one wide rectangle does not cause a full 16K document scan.

Full document changes use the full render path.

## Main C exports

The module includes these brush and batch exports:

- `init(width, height)`
- `paint_destroy()`
- `load_brush(json)`
- `begin_stroke(...)`
- `stroke_to(..., linear)`
- `paint_continue_stroke_to()`
- `paint_has_stroke_continuation()`
- `paint_begin_batch()`
- `paint_end_batch()`
- `paint_is_batch_done()`
- `paint_end_batch_finish()`
- `reset_brush()`

It includes these tile and display exports:

- `paint_get_tile_ptr(tx, ty)`
- `paint_render_tile_ptr(tx, ty)`
- `paint_render_rgba8_tile_ptr(tx, ty)`
- `paint_render_rgba8_mip_tile_ptr(tx, ty, level)`
- `paint_render_layer_rgba8_tile_ptr(layer, tx, ty)`
- `paint_write_rgba8_tile(tx, ty, rgba8)`
- `paint_get_used_tile_count()`
- `paint_region_has_paint(tx, ty, level)`
- `paint_set_eotf(value)`

It also includes layer, group, history, background, symmetry, color-pick, and brush-setting exports.

## Pixels and display

Document pixels use sparse 64 x 64 RGBA16 premultiplied tiles. The full channel value is `32768`.

The display path removes premultiplication, applies EOTF `2.2`, applies fixed dither, and writes RGBA8 tiles.

Documents from 64 through 16,384 pixels per axis are permitted. Documents above 4,096 pixels use reduced display storage and mip tiles.

The surface can contain eight paint layers and four groups. It supports all 22 MyPaint layer modes.

The default layer mode is Pigment. The default background color is `#A8A498`.

## Ownership and command order

The worker completes current paint data before it applies these commands:

- Brush or color changes
- Commit
- Clear
- Undo or redo
- Layer or group changes
- Export
- Probe

This order prevents brush-state changes during a continuation and surface access during a pthread batch.

`MotionQueue` has 8,192 sample slots. If it becomes full, it removes the oldest motion sample and increments `overflowCount`.

## Error codes

`paint_get_error_code()` returns these current values:

- `1`: Tile allocation failed.
- `2`: The history capacity was reached.
- `3`: The libmypaint dab loop made no progress.
- `4`: The operation queue reached capacity.
- `5`: A pthread did not exit correctly.

The worker sends each fatal engine error to the page log and status output. It also reports worker errors, promise rejections, and batch-finish exceptions.

The implementation does not contain a watchdog, abort path, cooldown, or automatic serial mode.

## Tests

Run all prototype tests:

```sh
cd prototype/character-editor
timeout 240 bun run test
```

The test set includes:

- TypeScript motion-queue tests
- Fixed operation-queue hash and capacity tests
- Exact cooperative-brush state and pixel tests
- Layer compositor tests
- MyPaint layer parity tests
- TypeScript compilation

The layer parity result is 21 bit-exact modes. Pigment has a maximum difference of one least-significant bit.
