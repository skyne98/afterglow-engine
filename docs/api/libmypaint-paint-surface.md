# WASM paint surface

Status: prototype implementation

The character-editor paint demo uses the maipointo brush engine (see `maipointo.md`) driving the NG libmypaint tiled surface. The vendored source is at commit `d5a88fbe6649d5ec776bc42ec8c1f4bb29d7fd7f`.

`paint-engine-worker.ts` owns the WebAssembly module, motion queue, brush state, document pixels, and `OffscreenCanvas`. The page sends input and control messages.

## Build

Build the pure-Rust module (the engine is maipointo; see `maipointo.md`):

```sh
cd prototype/character-editor
RUSTC_BOOTSTRAP=1 bash paint/build-wasm.sh
```

The module is a plain `rust-lld` cdylib with imported shared memory (`--import-memory --shared-memory`), panic=abort, and no Emscripten runtime. `paint/build-wasm.sh` copies `maipointo.wasm` to `public/wasm/brushlib.wasm` and `src/wasm/`. The TS loader `src/maipo-wasm.ts` instantiates it with a host-owned shared `WebAssembly.Memory` and wraps the exports for the worker.

## Brush processing

`stroke_to()` starts one exact libmypaint input sample. It processes a maximum of 128 dabs and returns one of these values:

- `0`: More dabs remain.
- `1` or `2`: The input sample is complete.
- A negative value: The brush state has an error.

`paint_continue_stroke_to()` processes the next 128 dabs. `paint_has_stroke_continuation()` reports the continuation state.

The continuation does not add input points. It resumes inside the original libmypaint dab loop.

The original and cooperative paths have equal brush states and output bytes in `mypaint-brush-cooperative.test.c`.

The TypeScript worker keeps the current `MotionQueue` sample until its continuation completes. It yields between batches, so later worker messages can enter fixed queues.

One fixed `MessageChannel` starts continuation tasks. It permits only one pending wake and does not use a nested zero-delay timer.

The worker can process more 128-dab continuation units in one tile batch for a maximum of 2 ms. The normal input drain keeps its 8 ms budget.

Chromium clamps nested zero-delay timers to approximately 4 ms. This delay multiplied a 613-batch Tail Feathers stroke to approximately 2.52 seconds.

An 8,192-item deferred command ring keeps each `beginStroke`, sample, and `commit` boundary. Thus, a backlog cannot combine separate strokes into one history record.

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

## History tile set

The history path uses a fixed generation set with one mark for each surface tile slot. A tile capture is an O(1) operation for all active-stroke sizes.

The set prevents a scan of all prior stroke tiles for each dab operation. Separate queued strokes also keep separate history records.

The history pixel capacity does not change. Error code `2` reports that capacity limit.

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

`PaintPointerState` owns the stroke, pan, and pointer-capture state. Before each zoom, rotation, mirror, view reset, or pan, the page commits an open stroke and releases its pointer capture.

A document change discards this pointer state before it initializes the new document. A lost pointer capture, hidden document, or window focus loss commits the open stroke. Thus, a missed pointer release cannot block the next stroke.

`MotionQueue` has 8,192 sample slots. If it becomes full, it removes the oldest motion sample and increments `overflowCount`.

The queue resolves each missing pressure, tilt, or view run once. It does not scan the remaining queue for each sample.

The deferred command ring has 8,192 slots. It reports and rejects a new command when full. It does not remove or reorder stored boundaries.

The demo sends brush data only after brush selection or engine initialization. It does not reload the same brush for each stroke.

The HUD gives both the current sample count and the deferred action count.

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

- TypeScript motion-queue order, interpolation, capacity, and long-run cost tests
- TypeScript fixed-ring order, wrap, and capacity tests
- TypeScript fixed task-wake coalescing and long-chain tests
- TypeScript paint-pointer view-change, stale-stroke, pan, and capture-loss tests
- Fixed operation-queue hash and capacity tests
- Fixed history-tile generation and capacity tests
- Exact cooperative-brush state and pixel tests
- Layer compositor tests
- MyPaint layer parity tests
- TypeScript compilation

The layer parity result is 21 bit-exact modes. Pigment has a maximum difference of one least-significant bit.

The browser pointer regression omits one pointer release, applies wheel zoom, and draws a second stroke. Both strokes complete without an engine error, and two undo commands remove them separately.

The wide-stroke wake measurements are in `docs/benchmarks/paint-wide-stroke-fox-workstation-2026-08-24.md`.
