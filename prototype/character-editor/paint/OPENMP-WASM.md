# Threading the brush engine on WebAssembly (current state)

Status: **serial tile processing is the verified, working path.** The
vendored libmypaint is pristine upstream. Multi-core parallel tile
processing is an open item.

## Build

`build-wasm-threads.sh` builds with the **emsdk 3.1.74** Emscripten
(`/home/fox/tools/emsdk`), `-pthread`, `MODULARIZE=1`, `EXPORT_NAME=
createBrushlib`, `EXPORT_ES6=1`, `PTHREAD_POOL_SIZE=4`. Output lands in
`public/wasm/` (served statically, no bundler transform) and the engine
worker loads it with a `new Function('url','return import(url)')` dynamic
import so Vite never analyzes Emscripten's circular worker import.

The Nix Emscripten 6.0.2 threaded-ESM output is internally broken
(`acorn-optimizer` assertions, no companion worker file), so threaded
builds always use the emsdk.

`build-wasm.sh` is the single-threaded Nix build (no `-pthread`).

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
malloc'd ops. The queue is bounded (4096 tiles, 16384 queued ops) and
single-threaded. `operation_queue_acquire/release/failed` remain as
no-op compat shims (pristine code never calls them).

## Verified serial behavior (browser)

- Full-width stroke renders complete (probe: `runs [94,1824]` on a 2048
  doc), no holes.
- Dirty rects are correct per flush batch (each batch renders its own
  rect; the canvas accumulates).
- Undo clears, redo restores.
- `bun test` green (28 pass / 0 fail), `bun run build` clean, wasm builds
  with both scripts.

## Open item: multi-core brush

Serial processing is the committed baseline. The earlier async attempt
proved `pthread_create` + atomic work-stealing works from a browser
worker *if the event loop stays live* (no `Atomics.wait`, no
`pthread_join`, no `pthread_mutex_lock`), but the work must be driven
entirely from our non-vendored code (`web-surface-threads.c` /
`main.c`), never by editing vendored libmypaint again. The operation
queue, tile hash, and dirty-rect/bbox accounting all need explicit
synchronization (the bbox union is main-thread only; the queue is
currently single-threaded by contract).
