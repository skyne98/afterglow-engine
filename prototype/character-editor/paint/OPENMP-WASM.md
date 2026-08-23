# Threading the brush engine on WebAssembly (current state)

Status: **parallel batch processing is implemented and browser-verified.
Serial remains the single-threaded fallback build.** The vendored libmypaint
is pristine upstream.

## Build

`build-wasm-threads.sh` builds the parallel engine with the **emsdk 3.1.74**
Emscripten (`/home/fox/tools/emsdk`), `-pthread`, `-DWEB_USE_THREADS`,
`MODULARIZE=1`, `EXPORT_NAME=createBrushlib`, `EXPORT_ES6=1`,
`PTHREAD_POOL_SIZE=4`. Output lands in `public/wasm/` (served statically, no
bundler transform) and the engine worker loads it with a
`new Function('url','return import(url)')` dynamic import so Vite never
analyzes Emscripten's circular worker import.

The Nix Emscripten 6.0.2 threaded-ESM output is internally broken
(`acorn-optimizer` assertions, no companion worker file), so threaded
builds always use the emsdk.

`build-wasm.sh` is the single-threaded Nix build (no `-pthread`) and keeps
the serial batch path (`main.c` compiles the `#ifndef WEB_USE_THREADS`
branch).

## Why OpenMP was abandoned

libmypaint's upstream parallel path is `#pragma omp parallel for` over the
dirty tiles. On wasm pthreads it deadlocks: the OpenMP barrier busy-waits
and there is no preemption, so the barrier never releases. Nix Emscripten
also ships no OpenMP at all (a `libgomp.a`/`omp.h` for wasm lives in
`paint/vendor/openmp-wasm/` and is only used to satisfy the unconditional
`#include <omp.h>` in `mypaint-tiled-surface.c`; the build passes no
`-fopenmp`, so the pragmas are inert and execution is serial).

## 2026-08-10: clean slate — vendored code restored, queue fixed

The manual pthread parallel loop, the async `paint_end_batch` split, and
all other edits inside `vendor/libmypaint/` were reverted to pristine
upstream (`git stash` in `vendor/libmypaint` holds the old edits as a
reference). Reverting surfaced the real bug:

**Root cause of the "holes / partially updated tiles"**: `fixed-operationqueue.c`
stored ops by *index into a fixed pool* (`op_index = operation -
queue->operations`) and pushed/pop'ed through `next_operation[]`. The
pristine vendored `draw_dab_internal()` `malloc`s each op and
`process_tile()` `free`s it — so `operation - queue->operations` was
pointer arithmetic between unrelated allocations, corrupting memory
("memory access out of bounds", strokes vanishing or only first dabs
rendering).

`fixed-operationqueue.c` now keeps the caller-owned ops out of the queue:
per-tile FIFOs of small `OpNode { op*, next* }` nodes that point at the
malloc'd ops. The queue is bounded (4096 tiles, 16384 queued ops). Adds
happen only on the main thread; each tile's FIFO is popped by one parallel
worker only (see below), so no lock is needed on the queue. `op_count` is
add-only (popping does not decrement it) so concurrent pops cannot race it.
`operation_queue_acquire/release/failed` remain as no-op compat shims
(pristine code never calls them).

## Verified serial behavior (browser)

- Full-width stroke renders complete (probe: `runs [94,1824]` on a 2048
  doc), no holes.
- Dirty rects are correct per flush batch (each batch renders its own
  rect; the canvas accumulates).
- Undo clears, redo restores.
- `bun test` green (28 pass / 0 fail), `bun run build` clean, wasm builds
  with both scripts.

## 2026-08-10 (later): serial-path hardening

- **Interrupted strokes**: an undo/redo/clear that runs while a stroke is
  still open (batch uncommitted) now finalizes or discards the pending
  history record instead of silently merging it into the next stroke. If
  the cursor moved (an undo already restored an older record) the pending
  captures are dropped; otherwise they are committed. The commit also
  refuses to record when the cursor is not at the newest record, so a
  late pointerup-commit after a mid-stroke undo cannot capture a corrupt
  after-state. The undo/redo/clear handlers reset the brush state so the
  pointerup release sample cannot paint taper dabs onto the restored
  canvas.
- **Export**: `exportTiles` renders one 64x64 tile per source tile at full
  document resolution (the old mip-grid path truncated docs larger than
  the display canvas, and the layer path sampled only the top-left tile
  of each mip group).
- **Render allocations**: `drawTile` reuses two `ImageData` scratch
  buffers (level-0 and scaled) instead of allocating two per tile per
  frame.

## 2026-08: parallel batch driver (implemented, browser-verified)

`web-surface-threads.c` is the thread-safe tile backend (atomic hash,
spinlock, `no_create`, 16 null tiles). It now also owns the parallel driver:
`web_surface_batch_begin/precreate/launch/is_done/finish/in_flight`. `main.c`
compiles a threaded batch path (`#ifdef WEB_USE_THREADS`) and `build-wasm-threads.sh`
defines `WEB_USE_THREADS`.

How it runs (one batch = one `drainProcess` in `paint-engine-worker.ts`):

1. `paint_begin_batch` opens the atomic; dabs accumulate in the op queue.
2. `paint_end_batch` (main thread) calls `batch_precreate`: every dirty
   tile is created through the request path, so history before-states are
   captured exactly once, where `calloc` is legal.
3. `batch_launch` sets `no_create=1`, spawns up to 4 detached pthreads;
   each claims tiles via one atomic counter and runs the vendored
   `process_tile()` (never edited). Workers never allocate tile memory;
   they only `free()` already-allocated op copies (Emscripten proxies
   `sbrk` to the live main thread). Returns 0 = async.
4. `paint_is_batch_done` reads one atomic; the worker TS polls every 2 ms.
5. `paint_end_batch_finish` (main thread) mirrors the vendored roi/bbox
   merge, clears the op queue, resets `no_create`, re-arms for the next
   batch.

Rules kept: no `Atomics.wait`, no `pthread_join`, no `pthread_mutex_lock`;
the queue is quiescent during a batch; each tile is claimed by one worker.
If there are < 2 dirty tiles or a spawn fails, `paint_end_batch` falls back
to the serial `end_atomic_internal()`. The engine keeps ONE batch at a time.

TS worker (`paint-engine-worker.ts`) changes for correctness:
- **Deferred commit**: `doCommit` no longer commits while a parallel batch
  is in flight — it sets `commitP` and the poll-completion path commits
  after `paint_end_batch_finish`.
- **In-flight stroke buffering**: a `beginStroke` that arrives while a
  batch is in flight is stored in `pendingBegin`; its samples go to
  `pendingSamples`; `afterBatch` drains any old-stroke residue, then
  applies the deferred begin. Surface-touching commands (clear, undo,
  redo, layers, export, probe, ...) arriving mid-flight are deferred to
  `pendingCmds` and replayed in order after the batch finishes, so a
  `clear` can never free tiles while workers write them.

Serial build: `main.c`'s `#ifndef WEB_USE_THREADS` branch keeps the
synchronous `paint_end_batch` (returns 1 from `paint_is_batch_done`); the
demo and tests behave unchanged.

## Verified parallel behavior (browser, 680M / threaded build)

- Stroke renders a full-width run (probe 311–1262) with 32 used tiles and
  no holes; worker log confirms "ASYNC batch in flight" each batch.
- Undo clears the stroke, redo restores it, clear empties the canvas
  (952 painted → 0 → 952 → 0 across the four probe steps) — history
  round-trips across parallel batches.
- No worker errors; `bun test` green (28 pass / 0 fail).
- Open item: per-batch latency/throughput tuning (BUDGET of 8 samples per
  batch, thread count 4) needs a measured A/B against serial on heavy
  strokes; bit-exact serial-vs-parallel tile equality is the acceptance
  gate to add.

## Layer-mode parity with MyPaint (2026-08)

`layer-compositor.c` implements the same 22 layer modes as MyPaint's layer
stack (lib/pixops.cpp -> blending.hpp + compositing.hpp + fix15.hpp), with the
same mode ordering and the same Pigment default (`mypaint:spectral-wgm`). All
arithmetic is truncating fix15 exactly like MyPaint. The one divergence that
existed — the soft-light sqrt — is now a bit-for-bit transcription of MyPaint's
table+Babylonian `fix15_sqrt`.

`layer-compositor.parity.test.c` transcribes MyPaint's fix15/blending/
compositing headers (GPL, test-only) and exhaustively compares every mode over
a source/backdrop colour, alpha and opacity grid. Result: **21 modes bit-exact
(0 LSB); Pigment within 1 LSB** (its float `fastpow` accumulation order).
Both C tests run from `bun run test` (`test:cc`).

`rgb_to_spectral`/`spectral_to_rgb`/`fastpow` are the vendored libmypaint
copies, byte-identical to MyPaint's.

## Error surfacing & stall recovery (2026-08)

A parallel batch that never completes would leave every input command
deferred silently (no console error — the symptom is "drawing stops"). The
pipeline now surfaces every failure path and self-heals:

- **Batch watchdog** (`paint-engine-worker.ts` `pollBatch`): after 250 ms of
  polling without completion, it calls `paint_batch_abort` (C: drops the
  pending op queue, clears `in_flight`, disables threading for the session so
  the next batches run serially), posts a `WATCHDOG ...` log AND a status
  line, then resumes the normal queue. Painting continues on the serial path.
- **Worker crash/exception reporter**: `self.onerror` and
  `self.onunhandledrejection` post `WORKER ERROR`/`WORKER UNHANDLED
  REJECTION` logs; every message is routed through a `handleInput` guard whose
  rejection triggers `batchRecovery` (reset async state, abort any batch,
  re-render, push state) instead of freezing.
- **Batch finish guard**: `_paint_end_batch_finish` throw also routes to
  `batchRecovery`.
- **History correctness**: a `commit` arriving mid-flight (and `doCommit`
  itself) now defers to `commitP` instead of capturing the history "after"
  state before the parallel workers finish.
- **Spawn failure**: partial `pthread_create` failure no longer falls back to
  serial while a spawned worker still runs (double-process race). Live
  workers drain the whole claim counter; `web_surface_batch_finish` processes
  any unclaimed tiles serially, and zero-workers degrades cleanly.
- **Surface gate**: `brush load` failures and per-setting `config` exceptions
  are logged instead of silently ignored.
