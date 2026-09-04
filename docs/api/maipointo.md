# maipointo (マイペイント) — the paint brush engine

Status: prototype implementation, byte-exactness gate met, zero-C wasm module

`prototype/character-editor/paint/maipointo/` is a from-scratch Rust
reimplementation of the NG libmypaint brush machine
(`mypaint-brush.c` + the dab blend/mask math + tiled surface +
layer compositor). It is the paint demo's only brush engine and the
entire wasm module — no C is linked into the product. The vendored C
engine exists solely as the reference for the exactness tests.

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
Start the development server with `cd prototype/character-editor && bun run dev`, then open `/paint/paint-demo.html` on port 5173.
Plain HTTP LAN origins do not supply `crypto.randomUUID()`.
The page uses a `crypto.getRandomValues()` UUIDv4 fallback for these origins.

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
