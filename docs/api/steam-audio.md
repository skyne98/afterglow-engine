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
int32_t afterglow_obvhs_create_indexed(
    const IPLVector3* vertices, uint32_t vertexCount,
    const uint32_t* triangleIndices, const uint8_t* materialIndices,
    uint32_t triangleCount, const IPLMaterial* materials,
    uint32_t materialCount, void** tracer);
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

Both create functions return zero on success and one for invalid pointers,
counts, indices, or materials. `afterglow_obvhs_create` accepts flattened
triangles and an optional door BLAS. `afterglow_obvhs_create_indexed` accepts a
static indexed mesh, validates every vertex/material index, and expands directly
into tracer-owned triangles without a second flattened C++ input array. Both
copy all retained data before returning, so callers may release the input
arrays. Destroy the tracer only after the simulator and custom scene have been
released.

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
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals,+simd128' cargo build \
  --manifest-path prototype/steam-audio-wasm/obvhs-tracer/Cargo.toml \
  --release --target wasm32-unknown-emscripten \
  -Zbuild-std=core,alloc,std,panic_abort
```

Steam Audio is linked with `-O3 -msimd128 -pthread`, `rayBatchSize = 64`, and a
strict two-worker Emscripten pthread pool. The ordinary dynamic module has fixed
256 MiB shared memory. The isolated full-Bistro stress module has fixed 1.5 GiB
shared memory and is never loaded by the normal runtime. Steam
Audio, MySOFA, PFFFT, zlib, and Rust are all rebuilt with atomics/bulk-memory;
mixing non-threaded objects into the module is a link error. Workers are created
during bootstrap, never after sealing.

Afterglow's local CWBVH kernel intersects four children per SIMD128 operation
using `core::arch::wasm32`. WABT disassembly verifies
`f32x4.add/mul/min/max/le`. Native uses obvhs's four-lane SSE2 path.

The selected laptop tier is:

- 128 audible direct-ray + nearest-HRTF sources;
- 64 priority independent reflection sources;
- 512 global listener rays, two bounces;
- 500 ms parametric, order-0 tail;
- 30 Hz steady updates and bounded 60 Hz motion bursts.

The 10,000-triangle validation scene produced 1,261 CWBVH nodes, reported 661,048
owned bytes, and built in 12.82 ms on average.

## Validation

Five fresh unlocked CEF launches on the Ryzen 7 6800U measured 4.47 ms mean
reflection simulation and 6.235 ms worst p99. Every run reported two simulation
threads and four traversal lanes. No launch exceeded 16.667 ms; all IRs were
valid, all DSP outputs were non-zero, and RT60 changed with motion. Against the
scalar one-thread obvhs baseline, mean fell 63.6% and worst p99 fell 53.3%.

Raw evidence:
`docs/benchmarks/steam-audio-wasm-obvhs-simd-pthreads-fox-laptop-2026-07-18.json`.

The browser stress module also loaded and built all three full-resolution Bistro
scenes through `afterglow_obvhs_create_indexed`. Five fresh Worker/WASM builds
per scene on the laptop produced valid, varying IR output throughout. At 512×2,
package-worst p99 was 27.88 ms: all scenes fit 30 Hz, but none establishes a
strict 60 Hz production tier. At 1,024×2, package-worst p99 was 50.28 ms. The
2,832,120-triangle Exterior CWBVH owned 182,158,408 bytes and built in 2.83 s on
average. This proves correctness and gives a full-render stress bound; it also
confirms structural proxies are required for 60 Hz web simulation and bounded
memory.

Raw full-package evidence:
`docs/benchmarks/steam-audio-wasm-bistro-full-package-fox-laptop-2026-07-19.json`.

The same logical obvhs configuration builds natively: medium CWBVH, batch 64,
two simulation threads, 64 reflected sources, and 512×2 quality. Five matching
native laptop launches measured 3.39 ms mean / 4.316 ms worst p99. Native
full-render-mesh acoustics continue to use the separately validated Embree
backend. AudioWorklet deadline integration,
render-loaded contention, general dynamic instances, and structural-proxy error
measurement remain open shipping gates.
