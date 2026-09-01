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
  LRE dab mask, including the spectral (`paint > 0`) Pigment paths.
- `src/mapping.rs` — `mypaint-mapping.c` input mappings.
- `src/rngdouble.rs` — Knuth lagged-Fibonacci RNG (seed 1000).
- `src/settings.rs` + `build.rs` — tables/enums generated from the
  vendored `brushsettings.json` at build time (order equals the C enums).
- `src/surface.rs` — `FixedTiledSurface`: 64x64 RGBA fix15 tiles,
  0xFFFF prefill (the C `memset(buffer, 255)` quirk), fixed-capacity op
  queue (16384 ops, 4096 dirty tiles; overflow is counted, never grown).
- `src/compositor.rs` — the layer compositor (`layer_blend_over`,
  `pigment_blend`, 22 `BlendMode`s). Byte-validated against
  `paint/layer-compositor.c` (21 modes 0 LSB, Pigment <= 1 LSB).
- `src/symmetry.rs` — `mypaint-symmetry.c` + `mypaint-matrix.c` port
  (transforms, snowflake fall-through, rectangle expansion).
- `src/web_surface.rs` — sparse hash-tile surface (>= 8192 hash size),
  used-tile slots, shared 4096-tile resident budget, first-write capture,
  fixed 16384-op queue, `begin_atomic`/`end_atomic` dirty ROI (max 32
  rects), symmetry dab fan-out, `get_color`, display-dirty slots, and fixed
  tile-job publication.
- `src/app.rs` — `PaintApp`: layers/groups tree, history (fixed 40
  records and a 64 MiB entry-byte budget), with a 32 MiB limit for one
  stroke's before-image capture, display EOTF LUT, mip render, render/pick/
  symmetry/layer/group operations, and the cooperative 128-dab, one-sample
  stroke driver mapping onto the brush.
- `src/demo_capi.rs` — the full `_paint_*` + `_init`/`_malloc`/`_free`
  C ABI for the standalone wasm module (feature `demo`), plus the
  shared `.myb` v3 JSON loader (`src/capi_json.rs`). Scratch
  allocations use a fixed 64x32 KiB slot pool with a free bitmask.
  Every app access goes through a guarded `with_app`: Wasm exception
  handling unwinds a panic, drops the RefCell borrow cleanly, and degrades
  to error code 1. A busy RefCell returns the default result without a
  second panic. An engine panic must never abort into a trap that leaks the
  borrow and bricks the instance. The ABI also lists, reads, removes, and
  restores raw RGBA16 tiles for the TypeScript IndexedDB pager. The history
  system carries a 64 MiB entry-byte budget with deterministic oldest-record
  eviction and fallible reservation: at 16K documents a single stroke can
  capture hundreds of tiles. One stroke has a 32 MiB before-image limit. An
  over-size stroke paints without undo and reports error 2; an unbounded
  history OOM-aborted the worker mid-stroke.
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
`WebAssembly.Memory` (initial 256, maximum 4096 pages). The module
imports that memory with its own 256 MiB cap
(`paint/maipointo/.cargo/config.toml`), which overrides the engine
workspace's 64 MiB default for this crate only. All paint layers share a
fixed 4,096-resident-tile budget (128 MiB of RGBA16 tile data), and history
uses a separate 64 MiB byte budget. The worker stores evicted RGBA16 tiles
in IndexedDB and restores the needed brush region before a new stroke. It
evicts only between strokes, protects a bounded 1,024-pixel brush path, and
keeps 2,048 or fewer resident tiles after a page. During a stroke it restores
the next bounded input region, writes older tiles before eviction, and protects
the current brush region. An IndexedDB miss means a zero tile, while a
storage failure rejects the stroke without a wasm trap. A full budget without a page still reports error code `1`
instead of growing until the worker aborts.

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

36 tests total; all green with and without `--features demo`.

## Build

```sh
cd prototype/character-editor
RUSTC_BOOTSTRAP=1 bash paint/build-wasm.sh
```

This builds `public/wasm/brushlib.wasm` (single pure-Rust module) with
release `opt-level = 3` and mirrors it into `src/wasm/`. The demo has no
engine toggle; maipointo is the engine.
