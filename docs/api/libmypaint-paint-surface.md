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

The module is a plain `rust-lld` cdylib with imported shared memory (`--import-memory --shared-memory`), `panic=unwind`, `panic-unwind`, and the Wasm `exception-handling` target feature. It has no Emscripten runtime. `paint/build-wasm.sh` copies `maipointo.wasm` to `public/wasm/brushlib.wasm` and `src/wasm/`. The TS loader `src/maipo-wasm.ts` instantiates it with a host-owned shared `WebAssembly.Memory` and wraps the exports for the worker. The memory has a 16 MiB initial size and a 2 GiB maximum.

The default total paint-memory limit is 25 percent of detected system memory.
The maximum is 2 GiB, and the fallback is 1 GiB. The Document control gives
a 64 MiB through 2 GiB override. The tile limit subtracts the 64 MiB history
limit, 128 MiB for other paint data, and 8 MiB for each tile worker.

## Brush processing

`stroke_to()` starts one exact libmypaint input sample. The Rust app uses the cooperative dab driver with a 128-dab limit. A zero result keeps the sample at the queue head, and `paint_continue_stroke_to()` resumes it after the batch drains.

The worker closes the batch after one input sample or continuation unit. This keeps the fixed operation queue below capacity. The normal input drain keeps an 8 ms scheduling budget, and one fixed `MessageChannel` starts the next drain with one pending wake.

The page sends all coalesced pointer samples. It does not remove samples with a small time interval. `MotionQueue` has 8,192 fixed sample slots. It preserves input order until its fixed capacity is full.

An 8,192-item deferred command ring keeps each `beginStroke`, sample, and `commit` boundary. Thus, a backlog cannot combine separate strokes into one history record.

## Parallel tile processing

`paint_begin_batch()` starts an atomic tile batch. `paint_end_batch_parallel()` performs serial tile bookkeeping and publishes one job per dirty tile.

The TypeScript tile pool uses a bounded worker count from the page hardware-concurrency value. Each pool worker owns an isolated wasm instance and receives copied operation and tile bytes.

The paint worker copies completed tile bytes back into its module memory. It uses inline processing when the pool is not ready or fails. Pool boot failures reject pending jobs and stop the pool.

Inline tile work keeps operation and row progress for each job. The worker processes four rows at a time and yields after the 8 ms host budget.

`paint_end_batch_finish()` returns the dirty-ROI rectangle count after all tile jobs complete. Spectral smudge sampling applies queued operations only at selected pixels. It first collects the operation indexes for each tile in a reused fixed-capacity buffer.

The wasm normal, eraser, and alpha-lock dab paths use SIMD128. Their integer operation order and scalar fallback give the same RGBA16 result.

## Fixed operation queue

The operation queue has these limits:

- 4,096 dirty tile keys for each job group
- 16,384 dab operations for each batch
- A power-of-two hash with at least 8,192 entries for O(1) tile lookup

All layers share one runtime resident-tile limit. The first limit is 4,096
RGBA16 tiles (128 MiB). The hash and tile metadata grow with this limit. The
worker increases the limit by 2,048 tiles (64 MiB) until it reaches the
selected maximum. It does not write cold tiles before the final limit enters
its 512-tile allocation headroom.

Before a new stroke, the worker restores a bounded input region. During a
stroke, it protects the recent path and predicts the next 100 ms of movement.
The prediction has a 512-pixel maximum. The worker loads this region before
brush work. It renders pending display tiles before tile removal. An
eight-tile inner guard starts restoration before the brush reaches a cold tile.

At the maximum, the pager frees one 2,048-tile block. It selects tiles behind
the movement direction first. It writes only modified tiles in 64-tile,
2 MiB transactions. A restored tile stays clean until paint, import, undo,
or redo changes it. IndexedDB reads use an exact range for each tile column.
Undo and redo restore history in 64-tile groups. The pager makes room only
for history tiles that are not resident.

The cooperative 128-dab limit keeps normal editor brushes below the 16,384
operation batch limit. A batch with more than 4,096 dirty tiles uses more than
one job group. It drains all groups before the next stroke. A queue capacity
failure sets error code `4` before the engine clears the queue.

## History tile set

The history path uses a fixed generation set with one mark for each surface tile slot. A tile capture is an O(1) operation for all active-stroke sizes.

The external history ABI uses these exports:

- `paint_set_external_history(enabled)`
- `paint_external_history_finish()`
- `paint_external_history_capture_count()`
- `paint_external_history_capture_info(index, out)`
- `paint_external_history_capture_ptr(index)`
- `paint_external_history_clear_captures()`
- `paint_external_history_cancel()`
- `paint_write_layer_rgba16_tile_modified(layer, tx, ty, source)`

The set prevents a scan of all prior stroke tiles for each dab operation. Separate queued strokes also keep separate history records.

The browser worker uses an IndexedDB queue for exact history. It keeps as many as 256 completed operations and removes the oldest operation when the count or IndexedDB quota blocks a new one.

Each operation stores one before-image for each changed tile. The worker writes these images in 64-tile, 2 MiB batches during a stroke. Thus, one operation does not have the internal 32 MiB limit.

Undo first writes all current images to the opposite history side. It then restores all stored images. Redo uses the same exchange. The cursor changes only after all tiles are restored. If restoration fails, the opposite side restores the prior document state.

If a current operation cannot fit after all older operations are removed, the worker restores its before-images. Thus, it does not complete paint that has no undo data. The 40-record, 64 MiB Rust history stays available as the internal fallback for hosts that do not select external history.

## Dirty display data

The C surface keeps both dirty rectangles and exact dirty tile slots.

Use these exports:

- `paint_get_dirty_count()`
- `paint_get_dirty_rect(index, out)`
- `paint_get_dirty_tile_count()`
- `paint_get_dirty_tile_info(index, out)`
- `paint_clear_dirty()`

The display worker renders exact dirty tiles for brush changes. Thus, one wide rectangle does not cause a full 16K document scan. While input remains, the worker combines display changes for a maximum interval of 16 ms.

A document with one visible full-opacity Normal layer uses an exact full-tile compositor path. Full document changes use the full render path. Render and mip paths reuse fixed tile buffers, so repeated tile renders do not grow the wasm heap.

## Main C exports

The module includes these brush and batch exports:

- `init(width, height)`
- `paint_init_with_tile_limits(width, height, initial, maximum)`
- `paint_get_resident_tile_count()`
- `paint_get_resident_tile_limit()`
- `paint_get_maximum_resident_tile_limit()`
- `paint_set_resident_tile_limit(limit)`
- `paint_destroy()`
- `load_brush(json)`
- `begin_stroke(...)`
- `stroke_to(..., linear)`
- `paint_continue_stroke_to()`
- `paint_has_stroke_continuation()`
- `paint_cancel_stroke()`
- `paint_begin_batch()`
- `paint_end_batch()`
- `paint_end_batch_finish()`
- `paint_process_tile_job_work(job, worker, row_budget)`
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
- `paint_get_layer_used_tile_is_storage_dirty(layer, index)`
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

The queue resolves each missing pressure, tilt, or view run once. It does not scan the remaining queue for each sample. The button bit identifies pen contact. Missing pen pressure starts at 0.5 for each stroke and then uses the last valid value from that stroke.

The deferred command ring has 8,192 slots. It reports and rejects a new command when full. It does not remove or reorder stored boundaries.

The demo sends brush data only after brush selection or engine initialization. It does not reload the same brush for each stroke.

The HUD gives the sample count, deferred action count, resident count, current
resident limit, and recent work times. The worker state also gives the selected maximum.

## Error codes

`paint_get_error_code()` returns these current values:

- `1`: A guarded engine call panicked or the resident tile budget failed.
- `2`: An internal history limit or an external capture reservation failed.
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

- TypeScript paint-memory limit, reserve, worker-count, and growth-block tests
- TypeScript exact IndexedDB column-range and 2 MiB write-batch tests
- TypeScript motion-queue order, interpolation, capacity, and long-run cost tests
- TypeScript fixed-ring order, wrap, and capacity tests
- TypeScript fixed task-wake coalescing and long-chain tests
- TypeScript paint-pointer view-change, stale-stroke, pan, and capture-loss tests
- Fixed operation-queue hash and capacity tests
- Fixed history-tile generation and capacity tests
- A 256-operation FIFO history queue test
- Exact cooperative-brush state and pixel tests
- Layer compositor tests
- MyPaint layer parity tests
- TypeScript compilation

The layer parity result is 21 bit-exact modes. Pigment has a maximum difference of one least-significant bit.

The browser pointer regression omits one pointer release, applies wheel zoom, and draws a second stroke. Both strokes complete without an engine error, and two undo commands remove them separately.

A 16K browser worker test used the 512 MiB override and the radius-60
`blend+paint` brush. A 5,101-sample serpentine stroke increased the tile limit
from 4,096 to 8,448 and then paged to 7,885 resident tiles. A 2,551-sample
reverse-path stroke restored and changed old regions. It ended with 7,911
resident tiles and no tile-allocation or IndexedDB error. Both strokes exceeded
the undo capture limit, as specified.

The wide-stroke wake measurements are in `docs/benchmarks/paint-wide-stroke-fox-workstation-2026-08-24.md`. The radius-60 blend-and-paint profile is in `docs/benchmarks/paint-blend-paint-fox-workstation-2026-09-03.md`.
