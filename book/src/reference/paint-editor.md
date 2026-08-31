# Paint editor surface

The character-editor paint demo runs the maipointo brush engine — a from-scratch, bit-exact Rust port of the NG libmypaint brush machine — inside a WebAssembly worker. Brush dabs write sparse 64 x 64 RGBA16 tiles.

The vendored C engine exists only in the exactness validation oracles (`prototype/character-editor/paint/maipointo/reference/`). It is not linked into the demo. The wasm module is a plain `rust-lld` cdylib with imported shared memory — no Emscripten runtime, no json-c, no C. The compositor and symmetry parity tests byte-validate the Rust ports against the C reference. A hand-written TypeScript loader (`src/maipo-wasm.ts`) instantiates the module and provides the HEAP views, string helpers, and shared `WebAssembly.Memory` the worker expects. The wasm build uses `panic=unwind` with Wasm exception handling, so a guarded engine panic does not brick the worker.

The tile pool uses a bounded worker count from the page hardware-concurrency value. Each pool worker owns an isolated wasm instance and receives copied dirty-tile data.

The stateful brush and smudge sample run on the paint worker in input order. A fixed `MessageChannel` starts the next drain and permits one pending wake.

The stateful smudge path can exceed the input rate. The fixed queue preserves input order until its capacity limit.

An 8,192-item fixed command ring keeps queued stroke boundaries. A backlog cannot combine separate strokes into one undo record.

The motion queue resolves each missing input run once. It does not scan the remaining queue for each sample.

History capture uses an O(1) generation set for tile slots. One stroke has a 32 MiB before-image limit. An over-size stroke paints without undo.

The display uses exact dirty tile slots. It does not render every tile inside one large dirty rectangle. Render and mip paths reuse fixed tile buffers.

The operation queue has fixed limits of 4,096 dirty tiles and 16,384 operations for each batch. All layers share 4,096 resident RGBA16 tiles. Capacity failures cause visible errors without a heap growth attempt.

The demo supports documents through 16K x 16K, eight paint layers, four groups, and all 22 MyPaint layer modes.

It also supports pressure, tilt, smudge, erasing, zoom, pan, rotation, mirror view, mip display, PNG, and OpenRaster.

A view change commits an open stroke and releases pointer capture before it changes the view. Capture loss and window focus loss also commit the stroke.

The worker reports brush-loop, queue, tile, history, worker, and promise errors. Tile work uses inline processing when the pool is unavailable.

The HUD gives the current motion-sample count, deferred action count, and recent work times.

The full API is in `docs/api/libmypaint-paint-surface.md` and `docs/api/maipointo.md`.
