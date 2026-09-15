# Paint editor surface

The character-editor paint demo uses the maipointo Rust brush engine.
Brush dabs write sparse 64 x 64 RGBA16 tiles.
The public-web target uses a WebAssembly worker.
The native shell uses one OS worker and the generated native RPC client.

## Native target

The native worker owns the document, brush, layers, groups, pinned tile cache, and SQLite history.
It never creates a service WebAssembly instance or a browser worker.
The shell displays RGBA tiles through Canvas2D and Blitz.
The native application connects to the profiling CLI through WebSocket and sends separate shell and paint records.
Native paint records contain stroke admission and raster publication counts, not image data.
The browser adapter sends stroke and worker-message records through a dedicated RPC diagnostics worker.
The browser URL can use `?profilingServer=ws://127.0.0.1:8086/` to select another local server.
The shared capture probe accepts `?target=web` for the browser. The default probe requires native workers.
Native and browser WebSocket captures passed with no lost records.
Controlled native timing comparisons and sustained memory checks remain open.
The native pacing change lets Wayland withhold redraws without stopping runtime maintenance.
Hidden-window rAF callbacks and game updates can wait until the next compositor redraw.
Timers, native workers, and input use separate bounded runtime turns.
The repair still needs real-driver acceptance, including hide/restore transitions.
See [Generic Telemetry](telemetry.md) for the capture boundary and current checks.
The current Blitz update uses upstream `main` commit `a50cb897`.
Native compilation, 53 native regression tests, and the RTX 3090 control probe passed.
The screenshot confirms visible select labels and numeric values.
The native adapter converts wheel direction and keyboard key names to DOM conventions.
Single-select controls have visible labels and a pointer/keyboard popup. Escape cancels selection, and Tab closes the popup.
Popup scrolling moves the options without a change to the container rectangle.
The Document panel uses native number inputs with sufficient line height for the Width and Height values.
Native text controls use Blitz/Parley for caret movement, drag selection, text replacement, and deletion.
Canceled keyboard or `beforeinput` events stop edits. Readonly fields retain selection without text changes.
A native raster regression checks visible selection and edited glyphs, including Unicode deletion and drag selection beyond the field border.
File/Edit/View menus now pass native opening, placement, activation, Escape, keyboard, and outside-click checks.
Shared DOM identity, style mutation, transformed geometry, and initial popup bounds repairs supply this behavior without menu-specific changes.
The current native input change adds text copy/cut/paste, script clipboard access, text undo/redo, and IME composition.
Text history keeps at most 512 edits per control and 32 MiB per document. Old history disappears at capacity, but current text stays unchanged.
Clipboard transfers have a 32 MiB UTF-8 limit. Clipboard errors do not cut text, and delayed paste ignores a changed focus or selection.
IME commit forms one text-history edit. Focus loss cancels transient composition.
A fixed mouse buffer retains 64 samples before dispatch, with host-arrival timestamps.
Native regression, pen, physical latency, and cross-platform checks for this change remain open.
The native shell does not yet have evidence of Chromium-equivalent artist input quality.
Button transitions use a monotonic clock instead of a fixed zero timestamp.
OpenRaster and PNG files use native file dialogs.

Native tile storage grows in 64 MiB pixel blocks after the initial 4,096-tile limit, within the selected maximum.
All layers share this capacity, and unused capacity does not allocate tile pixels.
The native RPC owner sends independent tile jobs to a fixed pool of at most 16 OS threads.
The owner waits for all jobs before it uses the job table again.
The native memory control now uses 50% of installed RAM as its default and maximum, in 64 MiB blocks.
An explicit native override can decrease the limit without changing the web setting.
Admission reserves metadata, display buffers, history/staging, renderer/codec memory, and thread scratch before resident tiles.
The native RPC bootstrap uses disk paging. All 25 native-worker integration tests passed.
Shared byte admission and process-memory checks passed in the final run. The counter does not intercept all foreign allocations.
The shared paint surface can use a pinned cache, and a new SQLite backing stores tile versions and completed-document roots.
The cache and surface checks passed nine native-worker tests and 35 shared-engine tests, including exact smudge results with two resident tiles.
The backing stages at most 32 tiles per transaction and retains at most 40 undo changes on disk.
Nine SQLite history checks passed, including process exit and history beyond the old byte limit.
New document metadata methods save and restore layer identities, group order and properties, background, and display EOTF without tile pixels.
Metadata and pixel recovery checks passed with 66 Rust tests across the native, shared-engine, and storage crates.
The native owner combines the app, pinned cache, and durable history without internal pixel capture.
The owner checks passed in a 70-test Rust lane.
Atomic replacement for an explicit New or Discard decision passed in a 71-test Rust lane.
A failed replacement retains the previous drawing and recovery root.
Native commands and Restore/Discard controls now use this owner.
The editor suite passed 90 tests. TypeScript, Vite, generated-web drift, shell compilation, and book checks passed.
The RTX 3090 release check passed Restore/Discard clicks through XWayland across three process launches.
After forced process termination, Restore retained dimensions, layer/group order, raster hash, undo, and redo.
Discard removed the previous drawing and history.
Run `nix-shell shell.nix --run 'bun scripts/test-native-paint-recovery.ts'` after the release and editor builds.
This runner uses private temporary storage, not user recovery data.
The 512 × 384 recovery check does not establish 16K performance or a memory bound.
An earlier RTX 3090 check measured the full-copy 4K display raster with one 16 KiB change per frame.
Its full-raster native commit took 14.826 ms mean and 17.336 ms p99 across 120 measured frames.
These are CPU timings, not GPU timings. The later region path replaces this full-copy behavior.
The `bench_canvas_upload` experiment measured 0.182 ms mean setup/submit for a persistent 4K texture on the RTX 3090.
Six exact same-image-identity comparisons passed, with no GPU validation errors.
A separate comparison with different image identities recorded a one-byte difference in one channel under rotation.
These experiment results are not native frame timings.
The new native path publishes changed display regions through a retained CPU raster and persistent GPU texture.
It preserves CPU capture through cold snapshots and has no artist configuration.
The region path passed the hardware pixel/lifetime check and 91 editor tests.
The release shell measured 0.0125 ms mean and 0.026 ms p99 CPU commit time for a 16 KiB change on a 4K raster.
Its rAF interval was 16.696 ms mean and 16.890 ms p99 across 120 frames.
These are not GPU timestamps or input-to-pixel timings.
The physical Restore/Discard rerun passed after process termination, with matching saved raster hashes and composition.
The final pass completed 108 Rust tests, 94 TypeScript tests, all specified builds, and both ten-minute 16K drawing runs.
Both targets retained 5,120 tiles, and all 131,072 sampled bytes matched exactly across eight display rows.
Shared admission and the final three-minute process-memory floor checks passed.
These checks do not prove an unlimited-session memory bound or native/web performance parity.
Native initial stroke completion reached 7.274 seconds maximum and needs further investigation.
See `docs/benchmarks/native-paint-shell/final-validation.md` in the repository for results and limits.
At startup, select Restore to open the previous completed drawing or Discard to create an empty document.
Restore uses the saved dimensions. New explicitly replaces the document and its history.
The shell selects a trusted `paint` directory under its native storage root.
Module startup completes at the recovery prompt so that native button input can operate.
The document stays unavailable until the decision and pixel publication complete.

Native history retains at most 40 undo changes on disk, without the old internal capture limit.
If a document operation fails, the engine tries to restore the last completed document.
After confirmed restoration, the editor reports the canceled stroke and accepts subsequent commands without a restart.
An error without confirmed restoration still stops native painting.
Native tiles use SQLite paging, not IndexedDB.
The command queue has 4,096 slots and an 8 MiB byte limit.
An input queue failure disables native paint and gives an error.
A single RGBA export cannot exceed 256 MiB.

This editor is a prototype, not a `GameplaySealed` runtime system.
JSON commands, document changes, canvas publication, and file operations can allocate memory.
The public-web timing measurements below do not describe native performance.

Build and run the native editor from the repository root:

```sh
(cd prototype/character-editor && bun run build)
nix-shell shell.nix --run 'CARGO_BUILD_JOBS=$(nproc) cargo run --release -p afterglow-shell -- --document prototype/character-editor/dist/paint/paint-demo.html'
```

The development shell supplies the GTK and desktop GSettings schema paths for native file dialogs.
Missing schemas can abort the process even after the editor becomes ready.
Check these paths with `nix-shell shell.nix --run 'bash scripts/test-shell-settings.sh'`.

Native paint combines adjacent stroke samples in ordered batches of at most 32.
It reads at most eight raster tiles concurrently instead of waiting for one host poll per tile.
Each ready stroke sample group publishes before the next group.
Native sliders have a track, thumb, pointer drag, and keyboard controls.
The brush list scrolls under wheel input, and labels appear below their images.

The shell executes all module scripts in the built HTML, including the editor entry after the Vite helpers.
A separate normal-page screenshot check is necessary because the probe has a different module entry.

Use `paint-native-probe.html` for automatic native stroke, pointer, history, brush selection, canvas retention, slider, label, scrolling, and PNG checks.
It also checks select popups, wheel zoom direction, numeric values, and button hover transitions.
The RTX 3090 run passed these checks and produced a native screenshot.
The host also presents pending HUD changes when its animation and RPC queues become empty.
The current pointer probe uses native hit-testing, but does not measure OS input-to-pixel latency.
That full probe keeps startup open and does not accept OS input during its checks.
Its latest run stopped at composition undo. This separate failure remains open.
The profiling capture and control pages finish startup before measurement, so native menus remain available.
After the release and Vite builds, run `nix-shell shell.nix --run 'bun scripts/test-native-paint-menus.ts'`.
This separate check sends XTest mouse clicks to File, Edit, and View during the capture workload without a collector.
The cadence probe records 120 rAF intervals for unchanged frames, hover changes, and slider movement.
Use the release build for UI timing checks.
The shell uses incremental Blitz layout and retains unchanged layout results.
Empty mutation callbacks no longer force duplicate DOM synchronization.
Unused mouse-event offsets no longer force layout during slider movement.
Brush state replies also retain unchanged Layers panel rows and canvas attributes instead of rebuilding the UI.
The initial debug build measured only 11.57 FPS during hover changes and 9.78 FPS during slider movement on a 144 Hz display.
The matching release baseline measured 126.05 FPS and 56.36 FPS, so compiler optimization alone did not meet the display frame budget.
Before the upstream Blitz update, three corrected release launches measured 143.99–144.25 FPS during slider movement and 130.04–138.31 FPS during hover changes.
Hover changes still miss frame deadlines, and these short checks do not prove continuous 144 Hz presentation.
See `docs/benchmarks/native-paint-shell/` for the log, screenshot, and test limits.
Native dialog interaction and full browser-control visual parity still need manual checks.
The shell supplies OS-backed `crypto.randomUUID()` for document identifiers.
Its `crypto.getRandomValues()` accepts integer typed arrays up to 65,536 bytes.
The shell does not supply `crypto.subtle`.

## Public-web target

Run `cd prototype/character-editor && bun run dev` to serve the editor on the LAN with a generated self-signed HTTPS certificate.
Open `https://<computer-address>:5173/paint/paint-demo.html` from another device, then accept the certificate warning.

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

The default public-web paint-memory limit is 25 percent of detected system memory. The maximum is 2 GiB, and the fallback is 1 GiB. The Document control gives a 64 MiB through 2 GiB override. The tile calculation subtracts history, tile-worker, and other paint memory.

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
The Eyedropper tool samples the visible composite and updates all Color panel fields.

The options bar has one stabilizer profile for all brushes.
Select Pulled String for accurate corners, Moving Average for rounded curves, Exponential for long curves, or Inertia for flowing strokes.
The Amount control sets the filter strength.
Moving Average and Exponential also have Catch Up for pauses and pen lift.
The editor saves this profile in the browser.

The page uses Photoshop shortcuts for each available action:

| Shortcut | Action |
|---|---|
| `B`, `I`, `H`, `R`, `Z` | Select Brush, Eyedropper, Hand, Rotate View, or Zoom |
| `Alt` with the Brush tool | Temporarily sample the visible composite color |
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
