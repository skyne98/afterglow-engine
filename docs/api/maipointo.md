# maipointo (マイペイント) — the paint brush engine

Status: prototype implementation, byte-exactness gate met, zero-C wasm module

`prototype/character-editor/paint/maipointo/` is a from-scratch Rust
reimplementation of the NG libmypaint brush machine
(`mypaint-brush.c` + the dab blend/mask math + tiled surface +
layer compositor). It is the paint demo's only brush engine and the
entire wasm module — no C is linked into the product. The vendored C
engine exists solely as the reference for the exactness tests.

## Native paint service

`crates/afterglow-paint-worker` links maipointo directly.
One real OS worker owns the `PaintApp`, pinned tile cache, and SQLite document store through thread-local storage.
The application does not move its `Rc`-owned tile budget between threads.
Generated `PaintClient` calls use the existing native RPC rings.
The service name is `paint`. The editor contains no worker IDs.

`PaintSession` selects the target before it creates a browser worker.
Its native implementation is `engine/paint/native-paint.ts`.
The public-web implementation keeps the existing paint worker and OffscreenCanvas.
Native publication reads bounded RGBA tiles and commits the software canvas to Blitz.
The Eyedropper and diagnostic probe read this visible canvas.
The native adapter also registers `afterglow-engine.paint` with shell profiling.
Its fixed 1,024-record buffer contains stroke admission and raster publication
counts, not pixels or coordinates. Capture starts only after viewer connection.
See [telemetry](telemetry.md) for transfer capacities and allocation limits.

Native commands have a 64 KiB limit.
The page queue has 4,096 slots and an 8 MiB byte limit.
Each tile response contains 16,384 bytes (64 by 64 RGBA8 pixels).
Raster publication uses at most eight concurrent tile reads (128 KiB of response data).
The worker accepts `strokeBatch` with 1–32 samples, in input order, within the same 64 KiB command limit.
The page combines only adjacent stroke samples and keeps other commands as ordering barriers.
One page batch processes at most 32 commands or samples before raster publication.
A stroke sample group ends the batch immediately, so continuous input cannot postpone publication until 32 serial RPC calls complete.
Native history uses SQLite tile versions and at most 40 undo changes, without internal pixel capture.
A failed document operation restores the last completed document and cancels the remaining stroke work.
After a successful rollback, the response has `documentRolledBack: true`, the error text, state, and a full-document dirty rectangle.
The adapter publishes the restored pixels and reports the error without disabling commands.
The worker ignores further samples from that stroke until `commit` or a new `beginStroke`.
Transport errors and errors without confirmed rollback still disable the adapter.
`PaintApp::history_failure_rolled_back()` confirms restoration only for internal history, not for the external web history store.
Native resident tiles start at a shared limit of 4,096, or the configured maximum if smaller.
Tile admission increases this limit by up to 2,048 tiles when necessary, without exceeding that maximum.
`PaintApp::set_resident_tile_growth(step)` selects this behavior. A zero step retains manual admission for the web pager.
The call allocates no tile pixels. All layers use the same limit.
Native tile processing uses one fixed Rayon pool per RPC owner, with at most 16 OS threads.
`PaintApp::set_tile_job_dispatcher` selects synchronous job dispatch. The dispatcher must complete all jobs before return.
Each concurrent worker has a distinct scratch index. Table publication and capture bookkeeping remain on the RPC owner.
The public-web default dispatcher remains unchanged.
Native admission now uses 50% of installed RAM, rounded down to 64 MiB blocks, as its default and maximum.
An explicit native override can decrease this limit. The native override uses a separate storage key from the web control.
The calculation reserves metadata, three display buffers, history/staging, renderer/codec memory, and thread scratch before tile admission.
The platform query uses Linux/Android `sysconf`, macOS `sysctlbyname`, or Windows `GlobalMemoryStatusEx`.
These admission checks are not proof of a measured hard limit on all foreign-library allocations.
Native commands now use the pinned tile cache and persistent history. All 25 native-worker tests passed after integration.
The `tile_cache` module has shared LRU storage with read/write pins and fallible `Backing::load/save/remove` operations.
Pins prevent eviction and mutable aliases. Dirty writeback failure retains the original pixels.
`flush()` and `reset()` are bounded cold operations.
`PaintApp::set_tile_cache` can attach one cache to empty layers. Future layers use the same cache.
Paged surface access returns `TileRead` or `TileWrite` guards. Tile jobs keep write pins until completion.
Nine native-worker tests and 35 shared-engine tests passed after the guard changes.
The surface regression compares exact pixels and smudge results with only two resident tiles.
The native RPC bootstrap selects this cache for new and restored documents.
The new native `paged_store.rs` connects the cache backing interface to SQLite tile versions and completed-document roots.
It stages at most 32 tiles per transaction and keeps at most 40 undo changes on disk.
Nine SQLite paint-store checks passed, including process exit, full-database rollback, corrupt records, and history beyond the old byte limit.
The native command path and Restore/Discard controls now use this store.
The editor suite passed 90 tests. TypeScript, Vite, generated-web drift, and shell compilation checks also passed.
The accepted disk path has a 64 GiB scratch limit with a 16 GiB free-space reserve.
The database calculation leaves 2 GiB inside that ceiling for journal growth.
A connection-owned VFS additionally rejects writes that exceed the combined database/journal file extent limit.
The final quota and recovery tests passed, including journal exhaustion, retained data, continued writes, and rejected bypass operations.
The native `EngineMemory` resource supplies shared reservations for paint, store capacity, canvases, snapshots, and source GPU headroom.
Tile growth reuses unpinned disk-backed storage when a new reservation cannot fit.
Native state includes `sharedMemory.limitBytes`, `sharedMemory.reservedBytes`, `sharedMemory.peakBytes`, and `scratchPeakFileBytes`.
The memory values are admission charges, not RSS. The disk value is a logical file-extent high-water mark.
See `docs/api/native-memory.md` for ownership and foreign-allocation limits.
See `docs/implementation/native-large-canvas-paint-plan.md` and `docs/api/transactional-byte-store.md`.
File export and import use the generic shell file-dialog operations.
A single RGBA export cannot exceed 256 MiB.

This prototype does not claim allocation-free `GameplaySealed` operation.
JSON commands, raster publication, and file operations allocate memory.

The RTX 3090 native probe passed stroke, pointer-handler, undo/redo, and PNG checks.
See `docs/benchmarks/native-paint-shell/` for the log and screenshot.
Native dialog interaction and full browser-control visual parity remain manual checks.

## Document metadata

`PaintApp::document_metadata()` returns version-1 JSON metadata without tile pixels or undo records.
It contains dimensions, active layer, stable layer storage IDs, layer/group properties and order, background RGBA16, and display EOTF.
Brush settings and transient stroke state are not document metadata.

`PaintApp::from_document_metadata(metadata, initial, maximum)` constructs a separate empty app.
Invalid dimensions, storage IDs, modes, properties, duplicate nodes, missing nodes, and cyclic group links return errors.
The original app stays unchanged. The owner must attach the cache and register saved tile keys before publication.
Future layers use storage IDs above all restored IDs.

Metadata and SQLite pixel recovery checks passed with 19 native-worker, 39 shared-engine, and eight storage-worker tests.
The same change corrects group scratch sizing and layer-link movement after deletion.
Group-parent setters reject parent values below `-1`, which is the document root.

## Native document ownership

The private native `document.rs` owner combines `PaintApp`, the pinned tile cache, and the SQLite store.
It prepares a separate app before undo/redo changes the durable history head.
Rollback prepares the last completed document before it removes working state.
A layer deletion now removes its working storage keys before it removes the surface.
Unchanged commits do not add history roots.

`PaintApp::set_history_capture_enabled(false)` disables pixel capture for an owner with versioned storage.
The default remains enabled. The public-web external-history path is unchanged.
Change this setting between strokes. This setting does not supply history by itself.

The native owner checks passed with 23 native-worker, 39 shared-engine, and eight storage-worker tests.
The RPC bootstrap now uses this owner. Its 25 native-worker tests passed, including a generated-client restart and Restore/Discard decisions.

The new `Document::replace` operation prepares an empty app before one transaction replaces the document and clears its history.
Only an explicit New or Discard decision may call this operation.
A failed transaction retains the old app and recovery root.
Replacement checks passed with 24 native-worker, 39 shared-engine, and eight storage-worker tests.
They verify failure, reopen, empty replacement, history removal, and unchanged commits.

## Native recovery startup

Call `PaintWorker::set_storage_root(path)` before the worker starts.
The shell selects its native storage root plus `paint`. The worker uses `paint.sqlite` in that directory.
RPC commands cannot select a filesystem path.

An `init` command without a recovery decision returns `recoveryRequired: true` when a completed document exists.
It supplies no state or pixels. Document commands and tile writes stay blocked.
Send `init` with `recovery: "restore"` or `recovery: "discard"` after an explicit user decision.
Restore uses the saved dimensions and recalculates memory admission.
Discard uses the requested dimensions and atomically replaces the document.
The New control explicitly selects Discard. The public-web target retains its existing initialization behavior.

The editor shows Restore and Discard controls before it emits ready or accepts paint input.
Module startup can complete at the recovery prompt because the native host must accept button input.
The separate document-ready promise completes only after successful initialization and pixel publication.
The native adapter rejects exports before the document is ready.
The release recovery check passed three native process launches on the RTX 3090.
XWayland button clicks restored a killed process's completed drawing and then discarded it after another restart.
The check retained dimensions, layer/group order, raster hash, undo, and redo.
Run `nix-shell shell.nix --run 'bun scripts/test-native-paint-recovery.ts'` after the release and editor builds.
The runner uses separate temporary storage and never opens user recovery data.
See `docs/benchmarks/native-paint-shell/recovery-check/` for results and limits.
Large-canvas, memory, and sustained performance checks remain necessary.
A separate RTX 3090 baseline changed 16 KiB per frame on a 4K display raster.
The full 64 MiB native commit took 14.826 ms mean and 17.336 ms p99.
The rAF interval was 21.963 ms mean and 28.388 ms p99 (120 measured frames).
These values are CPU commit and frame-production measurements, not GPU timestamps.
See `docs/benchmarks/native-paint-shell/raster-baseline/`.
The `bench_canvas_upload` experiment passed six exact same-image-identity comparisons with no GPU validation errors.
At 4K, persistent setup/submit took 0.182 ms mean and 0.367 ms p99 on the RTX 3090.
Blocking completion took 1.118 ms mean and 1.933 ms p99. These are not GPU timestamps or native frame timings.
A different-identity comparison recorded one channel with a one-byte difference under rotation.
The evidence file retains that difference.
The native adapter now gives `context.commit` the changed display-tile rectangle, clipped at document edges.
The shell retains CPU pixels and uploads the combined pending rectangle to a persistent GPU texture.
Initial publication and resize still copy the full raster. CPU capture uses an immutable snapshot.
The region path passed 27 browser tests, an exact hardware GPU test, and 91 editor tests (3,464 assertions).
The release shell measured a 4K region commit at 0.0125 ms mean and 0.026 ms p99 across 120 frames.
Its rAF interval was 16.696 ms mean and 16.890 ms p99. These are not GPU timestamps or presentation timings.
See `docs/benchmarks/native-paint-shell/raster-regions/` for raw results and limits.
The physical recovery rerun passed after process termination, with Restore/Discard clicks on the owned window and matching saved raster hashes and composition.
See `docs/benchmarks/native-paint-shell/recovery-regions-ready/` for results and limits.
No artist configuration or fixed canvas-count limit was added.
The final pass completed 108 Rust tests, 94 TypeScript tests, all specified builds, and both ten-minute 16K drawing runs.
Both targets retained 5,120 tiles. All 131,072 compared bytes across eight display rows matched exactly.
The shared-admission and final three-minute process-memory floor checks passed.
The counter does not bound every foreign allocation, and these samples do not prove an unlimited-session memory bound.
Native initial stroke completion reached 7,274.183 ms maximum, versus 169.575 ms on web. Performance parity is not established.
See `docs/benchmarks/native-paint-shell/final-validation.md` for measurements and reproduction.

## Crate layout

- `src/brush.rs` — the stroke state machine (`stroke_to`,
  `count_dabs_to`, `update_states_and_setting_values`, smudge buckets,
  directional offsets). Every C implicit float-to-double promotion is
  reproduced explicitly: `expf`/`hypotf`/`atan2f` (float libm) versus
  `exp`/`log`/`atan2` (double libm) call sites are distinguished.
- `src/brush/cooperative.rs` — verbatim port of the demo's
  `mypaint-brush-cooperative.c` dab-budget wrapper (`cooperative_stroke_start`,
  `cooperative_stroke_continue`, `StrokeState`). Returns 2 to split a
  stroke, 1 when queued/finished, 0 while budgeted work remains.
- `src/brushmodes.rs`, `src/mask.rs` — the fix15 dab blend modes and the
  LRE dab mask, including the spectral (`paint > 0`) Pigment paths. The wasm
  normal, eraser, and alpha-lock paths use SIMD128. Scalar paths use the same
  integer operation order. The mask API can calculate one pixel or a specified
  row range without a change to its result.
- `src/mapping.rs` — `mypaint-mapping.c` input mappings.
- `src/rngdouble.rs` — Knuth lagged-Fibonacci RNG (seed 1000).
- `src/settings.rs` + `build.rs` — tables/enums generated from the
  vendored `brushsettings.json` at build time (order equals the C enums).
- `src/surface.rs` — `FixedTiledSurface`: 64x64 RGBA fix15 tiles,
  0xFFFF prefill (the C `memset(buffer, 255)` quirk), fixed-capacity op
  queue (16,384 ops) and 4,096-tile job groups. One batch drains all groups
  before the next stroke. Queue overflow is counted, never grown.
- `src/compositor.rs` — the layer compositor (`layer_blend_over`,
  `pigment_blend`, 22 `BlendMode`s). Byte-validated against
  `paint/layer-compositor.c` (21 modes 0 LSB, Pigment <= 1 LSB).
- `src/symmetry.rs` — `mypaint-symmetry.c` + `mypaint-matrix.c` port
  (transforms, snowflake fall-through, rectangle expansion).
- `src/web_surface.rs` — sparse hash-tile surface (>= 8192 hash size),
  used-tile slots, shared runtime tile budget, first-write capture,
  fixed 16384-op queue, `begin_atomic`/`end_atomic` dirty ROI (max 32
  rects), symmetry dab fan-out, `get_color`, display-dirty slots, and fixed
  tile-job publication. Spectral `get_color` applies queued dabs only to the
  selected pixels. A reused operation-index buffer limits each pixel to the
  operations for its tile. Row progress permits bounded inline tile work.
  Ops whose tiles fall outside the document grid are
  dropped at queue time (the C keeps them and skips them at tile-fetch
  time); a fully off-canvas dab reports `false`. Tile metadata and the hash
  grow when the host increases the runtime tile limit. Each tile has a storage-dirty bit.
- `src/app.rs` — `PaintApp`: layers/groups tree, an internal history fallback
  (fixed 40 records and a 64 MiB entry-byte budget), external history capture,
  display EOTF LUT, mip render, render/pick/
  symmetry/layer/group operations, and the cooperative 128-dab, one-sample
  stroke driver mapping onto the brush. A visible full-opacity Normal layer
  uses an exact full-tile compositor path when it is the only root item.
- `src/demo_capi.rs` — the full `_paint_*` + `_init`/`_malloc`/`_free`
  C ABI for the standalone wasm module (feature `demo`), plus the
  shared `.myb` v3 JSON loader (`src/capi_json.rs`). Scratch
  allocations use a fixed 64x32 KiB slot pool with a free bitmask.
  Every app access goes through a guarded `with_app`: Wasm exception
  handling unwinds a panic, drops the RefCell borrow cleanly, and degrades
  to error code 1. A busy RefCell returns the default result without a
  second panic. An engine panic must never abort into a trap that leaks the
  borrow and bricks the instance. The ABI also controls the shared runtime tile
  limit and lists, reads, removes, and restores raw RGBA16 tiles for the TypeScript pager. In external
  history mode, the ABI supplies each first-write RGBA16 capture to TypeScript.
  TypeScript writes captures to IndexedDB in 64-tile, 2 MiB batches. Thus, the
  internal 32 MiB capture limit does not apply to the browser paint worker.
- `src/random.rs` — `RandomSource` trait: `PortableRand` (production)
  and `GlibcRand` (TYPE_3 glibc clone) so parity tests can replay the
  C `rand()` stream exactly.

## Zero C in the product

The wasm module is a plain `rust-lld` cdylib: `--import-memory
--shared-memory`, `panic=unwind`, `panic-unwind`, and the Wasm
`exception-handling` target feature. The host owns the imported shared memory.
No Emscripten runtime, no json-c, no C glue. C lives only in
validation: `maipointo/reference/*.c` oracles, the compositor parity
harness (`paint/layer-compositor.parity.test.c` +
`paint/layer-compositor.c`), and the vendored libmypaint the tests
compile against.

The demo's TS loader (`src/maipo-wasm.ts`) instantiates the module,
wraps the exports (`_name` keys), provides `HEAPU8`/`HEAP32` views,
the string helpers the worker uses (`lengthBytesUTF8`,
`stringToUTF8`, `UTF8ToString`), and the shared
`WebAssembly.Memory` (initial 256, maximum 32768 pages). The module
imports that memory with a 2 GiB maximum
(`paint/maipointo/.cargo/config.toml`). This crate does not use the engine
workspace's 64 MiB maximum.

The Vite development and preview servers bind to `0.0.0.0` for LAN access.
They use a generated self-signed HTTPS certificate.
Start the development server with `cd prototype/character-editor && bun run dev`.
Then open `https://<computer-address>:5173/paint/paint-demo.html` and accept the certificate warning.
The page uses a `crypto.getRandomValues()` UUIDv4 fallback when `crypto.randomUUID()` is not available.

The page sets a total paint-memory limit. The default is 25 percent of
`navigator.deviceMemory`, with a 2 GiB maximum and a 1 GiB fallback. The
new-document control has a 64 MiB through 2 GiB override. The tile calculation
subtracts 64 MiB for history, 128 MiB for other paint data, and 8 MiB for
each tile worker.

All paint layers first share 4,096 resident tiles. The worker increases the
limit by 2,048 tiles (64 MiB of RGBA16 pixels) near each limit. It writes
cold tiles only after the limit reaches the selected maximum and enters its
512-tile allocation headroom. It then frees one 64 MiB block. The worker
protects the current and recent path. It also loads a path
up to 100 ms and 512 pixels in the movement direction. An eight-tile inner
guard starts a restore before the brush reaches a cold tile.

The pager writes only storage-dirty tiles. It uses 64-tile, 2 MiB write
transactions. A restored clean tile can leave memory without a second write.
IndexedDB reads use one exact compound-key range for each tile column. The
history queue keeps as many as 256 completed operations. It removes the oldest
operation at this count or when IndexedDB quota blocks a new write. Undo and
redo exchange one exact tile snapshot in two phases. A failed current-operation
write restores its before-images instead of completing without undo. An
IndexedDB miss means an empty tile. A paging failure ends the operation and
keeps its undo data. A history write failure rolls the operation back. A full
budget without a page reports error code `1`.

## Paint editor UI

The paint page uses Vue 3, shadcn-vue components, and Lucide icons.
`src/PaintApp.vue` supplies the Photoshop-style menu bar, options bar, tool rail, canvas, panels, and status bar.
`src/paint-demo.ts` connects these controls to the paint worker.

The Color panel has a hue wheel, a saturation-value square, two swatches, and synchronized hexadecimal, RGB, and HSV inputs.
The Layers panel shows one nested layer and group stack.
Worker state replies retain unchanged layer rows, canvas dimensions, and tile metrics.
Brush configuration changes do not rebuild the panel.
Layer or group field changes, order changes, and active-layer changes still update the panel.
It puts the selected item controls above the stack and the stack actions below it.
These controls set visibility, blend mode, opacity, parent, group composition, order, creation, and deletion.
The Eyedropper tool samples the visible composite and updates all Color panel fields.

The options bar has one global stroke stabilizer profile.
`src/paint-stabilizer.ts` supplies four fixed-capacity position filters:

- Pulled String holds the brush until the screen-space string is tight.
- Moving Average uses a fixed 64-sample maximum window for rounded curves.
- Exponential gives more weight to new samples for long curves.
- Inertia uses a damped spring for fast, flowing strokes.

The Amount range is 1 through 100.
Moving Average and Exponential support Catch Up during a pause and at pen lift.
Catch Up sends one point per animation frame and one final point at pen lift.
The profile applies to all brush presets and uses the `afterglow.paintStabilizer` local-storage key.
The sample path does not allocate filter storage during a stroke.

The page uses these Photoshop shortcuts for available actions:

| Shortcut | Action |
|---|---|
| `B`, `I`, `H`, `R`, `Z` | Select Brush, Eyedropper, Hand, Rotate View, or Zoom |
| `Alt` with the Brush tool | Temporarily sample the visible composite color |
| `Space` | Temporarily use the Hand tool |
| `[` / `]` | Decrease or increase the brush size |
| `{` / `}` | Decrease or increase brush hardness |
| `0` through `9` | Set brush opacity |
| `D` / `X` | Set default colors or switch foreground and background colors |
| `,` / `.` | Select the previous or next brush |
| `Ctrl/Command+Z` | Undo |
| `Ctrl/Command+Shift+Z` | Redo |
| `Ctrl/Command+N` | Make a new document |
| `Ctrl/Command+Shift+N` | Make a new layer |
| `Ctrl/Command+O` | Open an OpenRaster file |
| `Ctrl/Command+S` | Save an OpenRaster file |
| `Ctrl/Command+Alt+Shift+W` | Export a PNG file |
| `Ctrl/Command++` / `Ctrl/Command+-` | Increase or decrease zoom |
| `Ctrl/Command+0` / `Ctrl/Command+1` | Fit the canvas or use 100 percent zoom |
| `Delete` | Clear the active layer |
| `Tab` | Hide or show the panels |

A form control keeps its normal keys while it has focus. The app does not replace browser shortcuts that have no paint action.

## Exactness validation

`cargo test` in the crate builds C oracle binaries from
`reference/*.c` with
`cc -O2 -ffp-contract=off -fno-tree-vectorize -fno-tree-slp-vectorize`
(the vectorization flags pin the reference to source-order floating
point; GCC's SLP vectorizer reassociates reductions and would make the
gate compiler-dependent). The oracle replay covers:

- fixed and fuzzed single-dab tile bytes (all blend modes, spectral
  and legacy),
- mapping/RNG/interp/spectral-primitive bit comparisons,
- compositor parity (21 modes + Pigment spectral, byte-exact),
- symmetry parity (75 states, bit-for-bit vs `mypaint-symmetry.c`),
- full stroke replay: opcode streams through both engines with
  per-event state, evaluated-settings, speed-mapping, 17-float dab
  argument, and tile-byte comparison,
- event-prefix bisection to the first divergent dab.

The full test set is green with and without `--features demo`.

## Build

```sh
cd prototype/character-editor
RUSTC_BOOTSTRAP=1 bash paint/build-wasm.sh
```

This builds `public/wasm/brushlib.wasm` (single pure-Rust module) with
release `opt-level = 3` and mirrors it into `src/wasm/`. The demo has no
engine toggle; maipointo is the engine.
