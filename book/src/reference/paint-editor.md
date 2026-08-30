# Paint editor surface

The character-editor paint demo runs the maipointo brush engine — a from-scratch, bit-exact Rust port of the NG libmypaint brush machine — inside a WebAssembly worker. Brush dabs write sparse 64 x 64 RGBA16 tiles.

The vendored C engine exists only in the exactness validation oracles (`prototype/character-editor/paint/maipointo/reference/`). It is not linked into the demo. The wasm module is a plain `rust-lld` cdylib with imported shared memory — no Emscripten runtime, no json-c, no C. The compositor and symmetry parity tests byte-validate the Rust ports against the C reference. A hand-written TypeScript loader (`src/maipo-wasm.ts`) instantiates the module and provides the HEAP views, string helpers, and shared `WebAssembly.Memory` the worker expects.

The threaded build uses a maximum of four pthreads for separate dirty tiles. The main WASM worker polls joinable threads without a blocking join.

A large input sample can request thousands of dabs. The engine processes 128 dabs at a time and resumes the exact libmypaint state.

This fixed work unit prevents a wide 16K stroke from blocking all paint-worker messages. Brush and color changes wait for the current sample to complete.

A fixed `MessageChannel` starts the next continuation task. It avoids the browser nested-timer clamp and permits only one pending wake.

One tile batch can include 2 ms of continuation units. This makes wide and local input use the same bounded batch path.

An 8,192-item fixed command ring keeps queued stroke boundaries. A backlog cannot combine separate strokes into one undo record.

The motion queue resolves each missing input run once. It does not scan the remaining queue for each sample.

History capture uses an O(1) generation set for tile slots. Its cost does not increase with the active stroke tile count.

The display uses exact dirty tile slots. It does not render every tile inside one large dirty rectangle.

The operation queue has fixed limits of 4,096 dirty tiles and 16,384 operations for each batch. Capacity failures cause visible errors.

The demo supports documents through 16K x 16K, eight paint layers, four groups, and all 22 MyPaint layer modes.

It also supports pressure, tilt, smudge, erasing, zoom, pan, rotation, mirror view, mip display, PNG, and OpenRaster.

A view change commits an open stroke and releases pointer capture before it changes the view. Capture loss and window focus loss also commit the stroke.

The worker reports brush-loop, queue, tile, history, pthread, worker, and promise errors. It has no watchdog or automatic serial mode.

The HUD gives the current motion-sample count and the deferred action count.

The full API is in `docs/api/libmypaint-paint-surface.md`.
