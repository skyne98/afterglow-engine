# Steam Audio scene backends

Afterglow's evaluated Steam Audio 4.8.1 integration uses one ray-tracing backend
per deployment target:

| Target | Steam scene type | Backend |
|---|---|---|
| Native CEF and other native hosts | `IPL_SCENETYPE_EMBREE` | Steam Audio's embedded Embree 4.4 |
| Public web Worker only | `IPL_SCENETYPE_CUSTOM` | Rust `obvhs` 0.3.2 medium-build CWBVH8 |

This API currently lives in `prototype/steam-audio-wasm`; it is the validated
contract for promotion into an engine audio worker. It uses no baked acoustic
data. The production promotion sequence, asset contract, rings, AudioWorklet,
frame integration, and release gates are specified in
[`../implementation/spatial-audio-integration-plan.md`](../implementation/spatial-audio-integration-plan.md).
That design makes `EngineAudio` required and sole. One fixed Worker/context runs
Steam simulation, direct/HRTF/hybrid DSP and final mixing into a bounded final-
PCM ring on both targets. Native uses 128 total/16 complete world-physical
voices and eight quanta; public web uses 16 total/4 complete world-physical
voices and eight quanta. CEF uses the generated native RPC client, an OS worker,
native Steam Audio/Embree, a native PCM ring and native device callback. Public
web uses the generated WASM RPC Worker and a minimal PCM-consuming AudioWorklet. CEF
must never instantiate the audio service as WASM or a Web Worker. Parametric reflection remains an explicit
low-quality tier. Any fatal audio
fault disables all audio and emits a high-severity diagnostic. Acoustic tiles
stream automatically around the listener from pipeline-built traversal data,
while runtime shape state uses the shared prebuilt Box3D-style primitive,
convex and fixed-topology compound set.

## Integration-gate API (not public gameplay API)

`crates/afterglow-audio-worker` now defines the single
`#[rpc(worker = EngineAudioWorker)]` service used by Gate 0. On native targets,
`AudioWorkerConfig::default()` selects the accepted eight-quantum render-ahead
depth. The generated `EngineAudioServiceClient::spawn_worker` uses
`afterglow-rpc::native` and a real OS thread; a regression test exercises that
transport. On public web, the generated TypeScript client uses the web SAB
transport. Generated lifecycle methods are `configure`, `start`, `stop`, `updateMotion`,
`runSimulation`, `stats`, and `shutdown`. The current integration-gate voice
methods append stable method IDs for `spawn2d`, `spawnAt`, `spawnAttached`,
`spawnSpatialOnly`, `spawnListenerRelative`, `crossfade`, `crossfadeTo`,
`setVoiceVolume`, `pauseVoice`, `resumeVoice`, and `stopVoice`, followed by the
warm-up-only `loadWav` and `unloadSound` resident-asset methods. Large assets use
bounded sequential `beginWavUpload`/`appendWavUpload`/`finishWavUpload` and
`beginAcousticSceneUpload`/`appendAcousticSceneUpload`/
`finishAcousticSceneUpload` calls over the same RPC ring. Sound and voice
IDs use separate packed index+generation handle spaces; zero is invalid. Control
methods return `0` on success; `configure`,
`updateMotion`, and `runSimulation` return `-1` on validation/backend failure;
`start` returns `-1` before configuration and `-2` after a latched fatal fault.
`stats` currently returns a fixed `Float64Array` in this order: sample clock,
rendered quanta, simulation updates, energy, peak, last impulse sample, target
quanta, voice capacity, reflection capacity, four producer counts, running,
fatal, active spatial voices, active reflection voices, active scheduled voices,
active world-physical voices, rejected total capacity, rejected physical
capacity, stale handles, completed fades, loaded resident sounds, and resident
PCM bytes, acoustic vertices, acoustic triangles, and acoustic scene bytes.
Device-rate
`afterglow_audio_pump`, `afterglow_audio_simulate_motion`, and the fixed PCM
pointer exports operate on the exact state owned by that RPC service; they are
bounded host-clock hooks, not a second protocol. The web worker only runs a
simulation tick after restoring the final PCM ring to target depth. The `steam-audio` feature owns the thin
C FFI RAII wrapper. Without it, deterministic Rust PCM exercises service state.
The fixed scheduler reserves leading backend slots for complete world-physical
voices and the remaining slots for explicitly nonphysical placements, so dry
voices cannot crowd out the acoustic capacity. It performs no partial physical
downgrade. Volume/pause/stop ramps advance on the 48 kHz sample clock;
crossfades are equal-power and atomically retain the outgoing voice if the
incoming slot cannot be reserved. Backend gain changes interpolate within each
128-frame quantum. `loadWav` strictly accepts mono or stereo 48 kHz RIFF/WAVE
PCM16, PCM24, PCM32, or finite float32. It rejects malformed chunks, unsupported
sample rates/formats, sound-pool exhaustion, and resident-byte-capacity overflow.
The fixed limits are 64 resident sounds and 32 MiB on web / 256 MiB native.
Loading and unloading require a configured but stopped worker, and unload rejects
stale or in-use handles. Stable boxed PCM remains owned by Rust while the Steam
mixer reads it directly; looping is sample-clock exact, while one-shots release
their voice automatically after their final quantum. Diagnostic IDs 1..=16
remain aliases of the four synthetic producer families. Acoustic scene upload
accepts the checked `AGBIST1` indexed geometry fixture and atomically replaces
the Steam scene before playback; native uses Embree and web uses the custom
obvhs callbacks. The real-assets gate loaded all three full-resolution Bistro
scenes natively. Full Bistro did not fit the bounded normal web integration gate;
web production still requires structural proxies and tile streaming. Cooked `Sound` metadata,
BIG asset reads, and the allocation-free page command facade remain to be
implemented before these methods are the final public game API. Attached entity
IDs are retained by the scheduler, but ECS pose publication is likewise pending.

The native gate uses `NativeAudioRuntime::spawn`: the generated RPC service runs
on an OS thread, an idle hook fills a fixed `afterglow-rpc` native PCM ring, and
a no-allocation device callback drains arbitrary even stereo callback lengths.
It has reached the physical device with native Steam Audio/Embree. Final
`AppBuilder::on_ready` CEF composition is not yet implemented and must not be
claimed as integrated. `NativeAudioRuntime` exposes the typed RPC `client`,
`events`, one `NativePcmReader`, and shared `NativeAudioTelemetry`.
`NativePcmReader::read_interleaved` preserves partial-quantum state when a device
requests less or more than 128 frames and emits silence on empty/malformed input.
Telemetry reports rendered frames, full-ring polls, malformed/sequence faults,
device callbacks/underruns, and pump total/max nanoseconds. Call
`NativeAudioTelemetry::arm` immediately before starting the physical stream so
pre-start callbacks neither consume PCM nor count false underruns.

The public-web final-PCM SAB uses the exact `afterglow-rpc::RingBuffer` 12-byte
header (`capacity_bytes`, atomic `write_bytes`, atomic `read_bytes`) and two-to-
eight fixed framed slots; the selected web candidate uses eight. Each frame is
`[payload_len:u32][sequence:u32][128 interleaved stereo f32 frames]`; fixed
atomic telemetry follows the ring storage. Publication/consumption use monotonic
byte counters; the AudioWorklet
checks sequence, writes silence on empty/fatal state, and notifies the worker
through a consumption epoch. It allocates and posts no messages in `process()`.
Telemetry includes callbacks, rendered quanta, underruns, sequence errors,
atomic wake hits/misses, pump mean/max/deadline misses and a fatal latch.

Native initializes 128 scheduled voices and 16 complete world-physical effects.
Its historical four-quantum synthetic-scene physical-device gate completed 600
seconds with zero underruns, but full-resolution Bistro invalidated that depth:
four quanta dropped 22–27 callbacks per 10-second scene when reflection
simulation occupied the sole worker. Eight quanta completed all three scenes
with zero underruns and is now the native default.
Public web initializes 16 scheduled voices and 4 complete world-physical
effects. The earlier four-quantum/16-wet profile failed because a 15.97 ms Worker
interval exceeded 10.67 ms. The reduced 16/4 profile with eight quanta now passed
a short same-page hardware-WebGPU run: 38,900 callbacks and 104 Worker-owned
simulation updates with zero underruns, sequence/fatal errors or pump deadline
misses; pump mean/max was 0.095/2.265 ms. Physical capture had no internal zero
span above 0.063 ms. A second public-web run loaded the five official Steam
Audio SDK speech/noise/impulse WAVs (7,518,980 resident PCM bytes): 11,244
callbacks, zero underruns/errors/deadline misses, and a 0.083 ms physical-capture
internal zero span. This satisfies the bounded 60-second public-web gate.

`real_asset_audio_gate` validates coupled real content natively. It chunk-loads
all five sounds and each official full-resolution Bistro v5.2 environment,
places four sounds through the complete physical chain plus one dry sound, and
runs 512×2 reflections every second. At four quanta, Exterior/Interior/Wine
produced 27/22/26 underruns over 3,750 callbacks each. At eight quanta all three
produced zero underruns, sequence errors, or silent frames; pump maxima were
2.04/1.62/1.83 ms. The checked scene payloads are 71,421,336 / 23,366,297 /
29,930,363 bytes and contain 2,832,120 / 1,046,609 / 1,320,323 triangles.
`docs/benchmarks/steam-audio-real-assets-fox-workstation-2026-07-18.json` is the
raw record. Reproduce after cooking/downsampling the licensed source assets:

```sh
nix-shell -p assimp pkg-config --run \
  './prototype/steam-audio-wasm/build-native-bistro.sh'
# Resample the SDK WAV fixtures to mono 48 kHz PCM16 under target/real-audio-48k.
nix-shell shell.nix --run \
  'cargo run --release -p afterglow-audio-worker --features steam-audio \
   --example real_asset_audio_gate -- \
   target/bistro-source target/real-audio-48k /tmp/real-audio.json'
```

The normal 256 MiB web module did not complete full Bistro Interior
inside the bounded integration test; this confirms that public web needs cooked
structural proxies and streamed acoustic tiles, not full render geometry.

The Emscripten Wasm-AudioWorklet-DSP path remains research evidence only. It is
not the production ownership model and will be deleted after the unified Worker
path clears its long gate.

## Web custom-tracer ABI

`crates/afterglow-obvhs-tracer/include/afterglow_obvhs_tracer.h` exposes:

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
  --manifest-path crates/afterglow-obvhs-tracer/Cargo.toml \
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
