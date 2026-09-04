# Paint editor surface

The character-editor paint demo runs the maipointo brush engine — a from-scratch, bit-exact Rust port of the NG libmypaint brush machine — inside a WebAssembly worker. Brush dabs write sparse 64 x 64 RGBA16 tiles.

The vendored C engine exists only in the exactness validation oracles (`prototype/character-editor/paint/maipointo/reference/`). It is not linked into the demo. The wasm module is a plain `rust-lld` cdylib with imported shared memory — no Emscripten runtime, no json-c, no C. The compositor and symmetry parity tests byte-validate the Rust ports against the C reference. A hand-written TypeScript loader (`src/maipo-wasm.ts`) instantiates the module and provides the HEAP views, string helpers, and shared `WebAssembly.Memory` the worker expects. The wasm build uses `panic=unwind` with Wasm exception handling, so a guarded engine panic does not brick the worker.

The tile pool uses a bounded worker count from the page hardware-concurrency value. Each pool worker owns an isolated wasm instance and receives copied dirty-tile data. If the pool is not available, the paint worker processes four tile rows at a time and yields after its 8 ms budget.

The stateful brush and smudge sample run on the paint worker in input order. The cooperative driver limits each batch to 128 dabs and one input sample. A fixed `MessageChannel` starts the next drain with one pending wake. The page sends all coalesced pointer samples without a time filter.

Spectral smudge reads only selected pixels. For each tile, a reused buffer identifies its queued operations. The fixed queue preserves input order until its capacity limit.

An 8,192-item fixed command ring keeps queued stroke boundaries. A backlog cannot combine separate strokes into one undo record.

The motion queue resolves each missing input run once. It does not scan the remaining queue for each sample. The button bit identifies pen contact. Missing pen pressure starts at 0.5 for each stroke.

History capture uses an O(1) generation set for tile slots. The worker writes exact before-images to IndexedDB in 2 MiB batches during each operation. Thus, a large operation does not have the internal 32 MiB limit.

The history queue keeps as many as 256 completed operations. It removes the oldest operation when the count or storage quota blocks a new write. Undo and redo exchange exact tile images. If the current operation cannot fit, the worker restores its before-images instead of completing without undo.

A Chromium gate stored, undid, and redid one 1,770-tile operation (55.31 MiB). A second gate kept the newest 256 of 257 operations.

The wasm dab paths use SIMD128 for normal, eraser, and alpha-lock modes. The scalar and SIMD paths give the same RGBA16 result. A single full-opacity Normal layer also uses an exact full-tile compositor path.

The display uses exact dirty tile slots. It does not render every tile inside one large dirty rectangle. While input remains, it combines display changes for a maximum interval of 16 ms. Render and mip paths reuse fixed tile buffers.

The operation queue has a fixed limit of 16,384 operations for each batch. It drains dirty tiles in 4,096-tile job groups. All layers share one runtime resident-tile limit. It starts at 4,096 RGBA16 tiles. It grows by 2,048 tiles (64 MiB) until it reaches the selected maximum.

The default total paint-memory limit is 25 percent of detected system memory. The maximum is 2 GiB, and the fallback is 1 GiB. The Document control gives a 64 MiB through 2 GiB override. The tile calculation subtracts history, tile-worker, and other paint memory.

The worker writes cold RGBA16 tiles only when the final limit enters its 512-tile allocation headroom. It writes modified tiles in 2 MiB transactions and does not write restored clean tiles again. IndexedDB reads use exact tile-column ranges.

The worker restores the protected brush path before paint. It retains the recent path and loads up to 100 ms and 512 pixels ahead. At the memory maximum, it removes far-behind tiles first. The worker renders pending display tiles before tile removal. An eight-tile inner guard starts restoration before the brush reaches a cold tile.

A missing record means an empty tile. A storage failure drops the stroke without a wasm trap. The store is cleared during worker start, layer clearing, or document changes.

The demo supports documents through 16K x 16K, eight paint layers, four groups, and all 22 MyPaint layer modes.

It also supports pressure, tilt, smudge, erasing, zoom, pan, rotation, mirror view, mip display, PNG, and OpenRaster.

The paint page uses Vue 3, shadcn-vue components, and Lucide icons. Its compact layout has a Photoshop-style menu bar, options bar, tool rail, canvas, panels, and status bar.

The Color panel has a hue wheel and a saturation-value square.
It gives synchronized hexadecimal, RGB, and HSV inputs, plus current and previous color swatches.

The Layers panel shows layers and groups in one nested stack.
The selected item controls set blend mode, opacity, parent, pass-through mode, and isolation.
The row and toolbar controls set visibility, order, creation, and deletion.

The page uses Photoshop shortcuts for each available action:

| Shortcut | Action |
|---|---|
| `B`, `H`, `R`, `Z` | Select Brush, Hand, Rotate View, or Zoom |
| `Space` | Temporarily use the Hand tool |
| `[` / `]` | Change brush size |
| `{` / `}` | Change brush hardness |
| `0` through `9` | Set brush opacity |
| `D` / `X` | Set default colors or switch colors |
| `,` / `.` | Select the previous or next brush |
| `Ctrl/Command+Z` | Undo |
| `Ctrl/Command+Shift+Z` | Redo |
| `Ctrl/Command+N` | Make a new document |
| `Ctrl/Command+Shift+N` | Make a new layer |
| `Ctrl/Command+O` | Open an OpenRaster file |
| `Ctrl/Command+S` | Save an OpenRaster file |
| `Ctrl/Command+Alt+Shift+W` | Export a PNG file |
| `Ctrl/Command++` / `Ctrl/Command+-` | Change zoom |
| `Ctrl/Command+0` / `Ctrl/Command+1` | Fit the canvas or use 100 percent zoom |
| `Delete` | Clear the active layer |
| `Tab` | Hide or show the panels |

A form control keeps its normal keys while it has focus. The app does not replace browser shortcuts that have no paint action.

A view change commits an open stroke and releases pointer capture before it changes the view. Capture loss and window focus loss also commit the stroke.

The worker reports brush-loop, queue, tile, history, worker, and promise errors. Tile work uses inline processing when the pool is unavailable.

The HUD gives the current motion-sample count, deferred action count, resident tile count and limit, and recent work times.

A 2026-09-03 radius-60 blend-and-paint profile measured a 1.165 ms worker-wake p99 and a 15.702 ms maximum. It had no wake above 16.7 ms. The earlier profile measured 35.159 ms p99, 42.171 ms maximum, and 193 wakes above 16.7 ms.

The full API is in `docs/api/libmypaint-paint-surface.md`, `docs/api/maipointo.md`, and `docs/api/paint-tile-storage.md`.
