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

The module is a plain `rust-lld` cdylib with imported shared memory (`--import-memory --shared-memory`), `panic=unwind`, `panic-unwind`, and the Wasm `exception-handling` target feature. It has no Emscripten runtime. `paint/build-wasm.sh` copies `maipointo.wasm` to `public/wasm/brushlib.wasm` and `src/wasm/`. The TS loader `src/maipo-wasm.ts` instantiates it with a host-owned shared `WebAssembly.Memory` and wraps the exports for the worker.

## Brush processing

`stroke_to()` starts one exact libmypaint input sample. The Rust app uses the cooperative dab driver with a 128-dab limit. A zero result keeps the sample at the queue head, and `paint_continue_stroke_to()` resumes it after the batch drains.

The worker closes the batch after one input sample or continuation unit. This keeps the fixed operation queue below capacity. The normal input drain keeps an 8 ms scheduling budget, and one fixed `MessageChannel` starts the next drain with one pending wake.

The `MotionQueue` has 8,192 fixed sample slots. It preserves input order until its fixed capacity is full.

An 8,192-item deferred command ring keeps each `beginStroke`, sample, and `commit` boundary. Thus, a backlog cannot combine separate strokes into one history record.

## Parallel tile processing

`paint_begin_batch()` starts an atomic tile batch. `paint_end_batch_parallel()` performs serial tile bookkeeping and publishes one job per dirty tile.

The TypeScript tile pool uses a bounded worker count from the page hardware-concurrency value. Each pool worker owns an isolated wasm instance and receives copied operation and tile bytes.

The paint worker copies completed tile bytes back into its module memory. It uses inline processing when the pool is not ready or fails. Pool boot failures reject pending jobs and stop the pool.

`paint_end_batch_finish()` merges dirty areas and clears the operation queue after all tile jobs complete. The stateful brush and smudge sampling remain serial.

## Fixed operation queue

The operation queue has these limits:

- 4,096 dirty tile keys for each batch
- 16,384 dab operations for each batch
- 8,192 fixed hash entries for O(1) tile lookup

All layers share 4,096 resident RGBA16 tiles (128 MiB). The hash and tile
slot arrays use this fixed resident limit instead of the full document tile
count. Before a new stroke, the worker writes cold tiles to IndexedDB and
restores a bounded input region. During a stroke, it restores each next input region before brush work. It
writes older tiles before eviction and protects the current brush region.

The cooperative 128-dab limit keeps normal editor brushes below the 16,384
operation batch limit. A queue capacity failure sets error code `4` before the
engine clears the queue.

## History tile set

The history path uses a fixed generation set with one mark for each surface tile slot. A tile capture is an O(1) operation for all active-stroke sizes.

The set prevents a scan of all prior stroke tiles for each dab operation. Separate queued strokes also keep separate history records.

History uses a 64 MiB tile-entry byte budget and keeps at most 40 records. One active stroke limits before-image capture to 32 MiB. An over-size stroke still paints, but it gets no undo record and reports error code `2`. The system evicts the oldest records before it allocates new after-images.

## Dirty display data

The C surface keeps both dirty rectangles and exact dirty tile slots.

Use these exports:

- `paint_get_dirty_count()`
- `paint_get_dirty_rect(index, out)`
- `paint_get_dirty_tile_count()`
- `paint_get_dirty_tile_info(index, out)`
- `paint_clear_dirty()`

The display worker renders exact dirty tiles for brush changes. Thus, one wide rectangle does not cause a full 16K document scan.

Full document changes use the full render path. Render and mip paths reuse fixed tile buffers, so repeated tile renders do not grow the wasm heap.

## Main C exports

The module includes these brush and batch exports:

- `init(width, height)`
- `paint_destroy()`
- `load_brush(json)`
- `begin_stroke(...)`
- `stroke_to(..., linear)`
- `paint_continue_stroke_to()`
- `paint_has_stroke_continuation()`
- `paint_cancel_stroke()`
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
- `paint_get_layer_used_tile_count(layer)`
- `paint_get_layer_used_tile_info(layer, index, out)`
- `paint_get_layer_tile_ptr(layer, tx, ty)`
- `paint_remove_layer_tile(layer, tx, ty)`
- `paint_write_layer_rgba16_tile(layer, tx, ty, rgba16)`
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

The worker completes current paint data before it applies these commands. It
also waits for IndexedDB page work before it starts a new stroke:

- Brush or color changes
- Commit
- Clear
- Undo or redo
- Layer or group changes
- Export
- Probe

A page operation never runs during a Rust tile batch. It copies a tile, waits
for the IndexedDB write, and then removes the resident tile. A cold-tile read
that finds no record keeps the tile absent and therefore transparent.

This order prevents brush-state changes and surface access during a tile batch.

`PaintPointerState` owns the stroke, pan, and pointer-capture state. Before each zoom, rotation, mirror, view reset, or pan, the page commits an open stroke and releases its pointer capture.

A document change discards this pointer state before it initializes the new document. A lost pointer capture, hidden document, or window focus loss commits the open stroke. Thus, a missed pointer release cannot block the next stroke.

`MotionQueue` has 8,192 sample slots. If it becomes full, it removes the oldest motion sample and increments `overflowCount`.

The queue resolves each missing pressure, tilt, or view run once. It does not scan the remaining queue for each sample.

The deferred command ring has 8,192 slots. It reports and rejects a new command when full. It does not remove or reorder stored boundaries.

The demo sends brush data only after brush selection or engine initialization. It does not reload the same brush for each stroke.

The HUD gives the current sample count, deferred action count, and recent work times.

## Error codes

`paint_get_error_code()` returns these current values:

- `1`: A guarded engine call panicked or the resident tile budget failed.
- `2`: The history byte budget rejected a reservation.
- `3`: The libmypaint dab loop made no progress.
- `4`: The operation queue reached capacity.

The worker sends each fatal engine error to the page log and status output. It also reports worker errors, promise rejections, and batch-finish exceptions.

The implementation does not contain a watchdog or cooldown. Tile work uses inline processing when the pool is unavailable.

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
