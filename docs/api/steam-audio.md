# Steam Audio scene backends

Afterglow's evaluated Steam Audio 4.8.1 integration uses one ray-tracing backend
per deployment target:

| Target | Steam scene type | Backend |
|---|---|---|
| Native CEF worker | `IPL_SCENETYPE_EMBREE` | Steam Audio's embedded Embree 4.4 |
| Web Worker | `IPL_SCENETYPE_CUSTOM` | Rust `obvhs` 0.3.2 medium-build CWBVH8 |

This API currently lives in `prototype/steam-audio-wasm`; it is the validated
contract for promotion into an engine audio worker. It uses no baked acoustic
data.

## Web custom-tracer ABI

`obvhs-tracer/include/afterglow_obvhs_tracer.h` exposes:

```c
int32_t afterglow_obvhs_create(
    const AfterglowObvhsTriangle* staticTriangles, uint32_t staticTriangleCount,
    const AfterglowObvhsTriangle* doorTriangles, uint32_t doorTriangleCount,
    const IPLMaterial* materials, uint32_t materialCount, void** tracer);
void afterglow_obvhs_destroy(void* tracer);
void afterglow_obvhs_set_door_y(void* tracer, float doorY);
void afterglow_obvhs_get_stats(const void* tracer, AfterglowObvhsStats* stats);
```

The same header declares all four callbacks required by Steam Audio:

- `afterglow_obvhs_closest_hit`
- `afterglow_obvhs_any_hit`
- `afterglow_obvhs_batched_closest_hit`
- `afterglow_obvhs_batched_any_hit`

The Rust `staticlib` is compiled for `wasm32-unknown-emscripten` and linked into
the same module as Steam Audio. No ray crosses JavaScript or a second WASM
instance.

`afterglow_obvhs_create` returns zero on success and one for invalid pointers,
counts, materials, or material indices. It copies every triangle, material
index, and material before returning. Callers may release the input arrays.
Destroy the tracer only after the simulator and custom scene have been released.

## Ownership and query behavior

A tracer owns:

- one immutable static CWBVH8 and its source triangles/material indices;
- an optional immutable dynamic-instance BLAS;
- copied `IPLMaterial` values;
- one atomic translation value for the evaluated door instance;
- immutable build statistics.

Closest-hit and any-hit traversal use obvhs's fixed shallow CWBVH stack. Batched
callbacks iterate Steam's fixed input arrays and call the same allocation-free
query path. Unit tests run the callbacks under a tracked global allocator and
require zero allocations after construction.

The evaluated dynamic instance is translation-only. Updating it stores one
finite Y value atomically. Queries transform the ray origin into BLAS-local
space; no triangles move and no BVH rebuild occurs. General transforms and
multiple instances are not yet part of this prototype API.

Hits include distance, global triangle index, object index, material index,
geometric normal, and a pointer into the tracer-owned material array. Misses use
infinite distance, `-1` indices, a zero normal, and a null material. Invalid
intervals follow Steam Audio's callback contract: closest-hit returns a miss and
any-hit reports occluded when `maxDistance <= minDistance`.

The tracer uses a local inclusive-edge, two-sided Möller–Trumbore primitive test.
The upstream obvhs 0.3.2 triangle test rejects negative zero and can otherwise
produce an acoustic crack for a ray exactly on a shared triangle edge.

## Build and capacities

The web build uses:

```sh
RUSTFLAGS='-C target-feature=+simd128' cargo build \
  --manifest-path prototype/steam-audio-wasm/obvhs-tracer/Cargo.toml \
  --release --target wasm32-unknown-emscripten \
  -Zbuild-std=core,alloc,std,panic_abort
```

Steam Audio is linked with `-O3 -msimd128`, fixed 256 MiB memory for the dynamic
benchmark, a synchronous outer Web Worker, and `rayBatchSize = 64`. The current
obvhs CWBVH node traversal is scalar on WASM despite the module-level SIMD flag.
Other Steam Audio DSP still uses SIMD128.

The selected laptop tier is:

- 128 audible direct-ray + nearest-HRTF sources;
- 64 priority independent reflection sources;
- 512 global listener rays, two bounces;
- 500 ms parametric, order-0 tail;
- 30 Hz steady updates and bounded 60 Hz motion bursts.

The 10,000-triangle validation scene produced 1,261 CWBVH nodes, reported 661,048
owned bytes, and built in 13.03 ms on average.

## Validation

Five fresh unlocked CEF launches on the Ryzen 7 6800U measured 12.27 ms mean
reflection simulation and 13.365 ms worst p99. No launch exceeded 16.667 ms. All
IRs were valid, all DSP outputs were non-zero, and RT60 changed with motion.

Raw evidence:
`docs/benchmarks/steam-audio-wasm-obvhs-fox-laptop-2026-07-18.json`.

The web backend is accepted. A WASM SIMD CWBVH port is deferred unless structural
proxy or rendered-load tests consume the remaining margin. Native full-scene
acoustics continue to require Embree. AudioWorklet deadline integration,
render-loaded contention, general dynamic instances, and structural-proxy error
measurement remain open shipping gates.
