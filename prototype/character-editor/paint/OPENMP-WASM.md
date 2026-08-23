# Brush-engine pthread design

Status: The threaded WebAssembly build uses a fixed pthread tile batch. The serial build is a separate test and development build.

The vendored libmypaint source is unchanged. `mypaint-brush-cooperative.c` includes it and adds the Afterglow continuation API.

## Build

Use `paint/build-wasm-threads.sh` with Emscripten 3.1.74 from `/home/fox/tools/emsdk`.

The build uses these settings:

- `-pthread`
- `WEB_USE_THREADS`
- four initial pthread workers
- a 128 KiB pthread stack
- 256 MiB initial memory
- 1 GiB maximum memory

The script writes `brushlib.js` and `brushlib.wasm` to `public/wasm/` and `src/wasm/`.

`paint/build-wasm.sh` makes the separate serial module. Nix Emscripten 6.0.2 does not make correct threaded ES modules for this project.

## OpenMP decision

The upstream OpenMP loop can deadlock in WebAssembly. Its barrier can wait without browser-worker preemption.

Thus, the build does not use `-fopenmp`. The OpenMP pragmas are inactive.

## Operation queue

`fixed-operationqueue.c` keeps libmypaint operation ownership unchanged. Libmypaint allocates each operation, and `process_tile()` frees it.

The queue contains fixed per-tile FIFOs. A fixed 8,192-entry hash table gives O(1) tile lookup.

The limits are 4,096 dirty tiles and 16,384 operations for each batch. The engine reports a capacity error before it clears the queue.

## Parallel tile batch

One TypeScript drain is one tile batch:

1. `paint_begin_batch` starts the libmypaint atomic operation.
2. `paint_end_batch` creates all necessary tiles on the main WASM worker.
3. `batch_launch` starts a maximum of four pthreads.
4. Each pthread claims one dirty tile at a time.
5. `paint_is_batch_done` polls the completion counters.
6. It also uses `pthread_tryjoin_np()` until each joinable thread fully exits.
7. `paint_end_batch_finish` merges the dirty areas and clears the operation queue.

The main WASM worker never uses a blocking join. It does not use `Atomics.wait`.

Workers do not create tiles. Thus, a pthread cannot cause a proxied memory-growth deadlock during tile processing.

If fewer than two tiles are dirty, the main WASM worker processes the batch. A partial spawn still processes all unclaimed tiles.

## Exact cooperative dab loop

A 16K input sample can request many thousands of dabs. Some smudge brushes also read and update tiles for each dab.

The original `mypaint_brush_stroke_to()` processes all these dabs in one synchronous call. This blocks the paint worker and makes input appear deadlocked.

`mypaint-brush-cooperative.c` keeps the same internal algorithm and state. It stops after 128 dabs and saves the exact continuation state.

The next tile batch resumes at the same loop position. It does not add an input sample or a final state update between continuations.

`mypaint-brush-cooperative.test.c` compares the original call with 495 continuations. All output bytes and all brush states are equal.

The continuation also checks for nonfinite dab counts and no-progress states. These conditions cause a visible engine error.

## Dirty display tiles

A dirty rectangle can cover most of a 16K document although only a small number of tiles changed.

Each `WebPaintSurface` now keeps a fixed list of changed tile slots. The display worker renders these tiles instead of all tiles in the bounding rectangle.

The rectangle list stays available for diagnostics. Full document changes still use the full render path.

## TypeScript ordering

`paint-engine-worker.ts` keeps the current motion sample in `MotionQueue` while a dab continuation is active.

The worker yields to its event loop between continuation batches. A later color or brush command waits until the current sample completes.

Commit, clear, undo, redo, layer, export, and probe commands also wait for all current paint work. This prevents concurrent surface access.

## Error information

The worker reports these errors in the status and log output:

- WebAssembly worker errors
- unhandled promise rejections
- libmypaint no-progress states
- operation-queue capacity failures
- tile-allocation failures
- history-capacity failures
- pthread exit failures
- batch-finish exceptions

The engine does not use a watchdog, batch abort, cooldown, or automatic serial mode.

## Tests

`bun run test` runs the TypeScript tests, layer parity tests, operation-queue tests, and the cooperative brush test.

The layer tests keep 21 modes bit-exact. Pigment stays within one least-significant bit because of float operation order.

The 16K Tail Feathers test uses radius 60 and a color change. The queue completes, the probe returns, and the worker log has no error.
