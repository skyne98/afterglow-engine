# Paint editor surface

The character-editor paint demo uses NG libmypaint in a WebAssembly worker. Brush dabs write sparse 64 x 64 RGBA16 tiles.

The threaded build uses a maximum of four pthreads for separate dirty tiles. The main WASM worker polls joinable threads without a blocking join.

A large input sample can request thousands of dabs. The engine processes 128 dabs at a time and resumes the exact libmypaint state.

This fixed work unit prevents a wide 16K stroke from blocking all paint-worker messages. Brush and color changes wait for the current sample to complete.

The display uses exact dirty tile slots. It does not render every tile inside one large dirty rectangle.

The operation queue has fixed limits of 4,096 dirty tiles and 16,384 operations for each batch. Capacity failures cause visible errors.

The demo supports documents through 16K x 16K, eight paint layers, four groups, and all 22 MyPaint layer modes.

It also supports pressure, tilt, smudge, erasing, zoom, pan, rotation, mirror view, mip display, PNG, and OpenRaster.

The worker reports brush-loop, queue, tile, history, pthread, worker, and promise errors. It has no watchdog or automatic serial mode.

The full API is in `docs/api/libmypaint-paint-surface.md`.
