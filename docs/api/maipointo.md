# maipointo (マイペイント) — the paint brush engine

Status: prototype implementation, byte-exactness gate met

`prototype/character-editor/paint/maipointo/` is a from-scratch Rust
reimplementation of the NG libmypaint brush machine
(`mypaint-brush.c` + the dab blend/mask math). It is the paint demo's
only brush engine. The vendored C engine exists solely as the reference
for the exactness tests.

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
- `src/random.rs` — `RandomSource` trait: `PortableRand` (production)
  and `GlibcRand` (TYPE_3 glibc clone) so parity tests can replay the
  C `rand()` stream exactly.
- `src/capi.rs` — the wasm C ABI (`maipo_*`) for the demo's Emscripten
  module, including the `.myb` v3 JSON loader. Rust allocations route
  through the module's emscripten malloc (`#[global_allocator]`).

## C in the demo module

The wasm module keeps only non-engine C: `mypaint-tiled-surface.c`
(mask + `process_op`), `brushmodes.c`/`helpers.c` (blend math),
`mypaint-surface.c` (public entry points the Rust engine calls back
through), `web-surface.c`, `layer-compositor.c`, and the fixed queues.
`main.c` talks to the engine only through `brush_engine.h`
(`brush_engine_rust.c` bridges to the `maipo_*` ABI).

The C brush engine (`mypaint-brush.c`, `mypaint-mapping.c`,
`mypaint-brush-settings.c`, `rng-double.c`, the cooperative wrapper) is
not linked into the product. It is compiled only by the exactness
oracles under `maipointo/reference/` and the vendored source the tests
compile against.

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
- full stroke replay: opcode streams through both engines with
  per-event state, evaluated-settings, speed-mapping, 17-float dab
  argument, and tile-byte comparison,
- event-prefix bisection to the first divergent dab.

The oracle rebuild is cached by source mtime; deleting
`target/tmp/*oracle*` forces a rebuild.

## Build

```sh
cd prototype/character-editor
nix-shell -p emscripten --run 'bash paint/build-wasm.sh'
```

This builds `public/wasm/brushlib.js` + `.wasm` (single module, no
json-c): the maipointo staticlib plus the C surface/compositor
infrastructure. The demo has no engine toggle; maipointo is the
engine.
