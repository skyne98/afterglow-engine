# Steam Audio in the browser

**Investigated:** 2026-07-18  
**Upstream version:** Steam Audio 4.8.1  
**Verdict:** fully dynamic direct sound and real-time reflections are technically
viable through Valve's experimental Emscripten target, including on the Ryzen 7
6800U. It is not a supported turnkey browser integration, and AudioWorklet
hosting plus render-loaded scheduling remain unvalidated shipping boundaries.

## Evidence

Valve added “experimental support for building Steam Audio for WebAssembly using
Emscripten” in 4.6.0. Current upstream source:

- recognizes `CMAKE_SYSTEM_NAME=Emscripten` and `IPL_OS_WASM`;
- builds a static library with WASM SIMD128;
- packages `lib/wasm/libphonon.a`;
- keeps Embree native-only and Radeon Rays/TrueAudio Next Windows/OpenCL-only.

The official 4.8.1 SDK archive contains a 7 MB `libphonon.a` under `lib/wasm`.
However, browsers are still absent from Valve's documented supported-platform
list and there is no official JavaScript, Web Audio, AudioWorklet, npm, Unity
WebGL, or browser sample integration.

A public user reported successfully compiling Steam Audio 4.6.1 into a Unity/FMOD
WebGL application after manually removing multithreading. That report only
validated binauralization, not reflections or reverb. Open upstream issue #496
reports that the released WASM archive does not include the MySOFA, PFFFT, and
zlib archives needed to link it; the reporter succeeded after building those
WASM dependencies locally. The issue remains open, and the 4.8.1 archive still
contains only `libphonon.a` in its WASM directory.

## Browser architecture

Steam Audio processes buffers; browser output still belongs to Web Audio.
A credible integration would:

1. build Steam Audio and its dependencies from source with Emscripten;
2. expose a narrow C ABI rather than the whole SDK;
3. run scene updates and bounded acoustic simulation in a Web Worker;
4. run deadline-safe HRTF/convolution DSP in an AudioWorklet;
5. exchange fixed-size controls and prepared acoustic results through bounded
   shared rings;
6. preallocate and seal memory before gameplay rather than relying on upstream's
   default `ALLOW_MEMORY_GROWTH`;
7. use the built-in CPU ray tracer, because no Steam Audio WebGPU/Embree/OpenCL
   backend is available in browsers.

Afterglow already supplies cross-origin isolation, SharedArrayBuffer, worker
ownership, and fixed rings. Those solve transport prerequisites, not real-time
audio deadlines. AudioWorklet code must not block, allocate, ray trace a large
scene, or compile WASM while rendering.

## Unified Afterglow worker target

The engine target is one source-level `SpatialAudio` service and protocol using
`afterglow-rpc::RingBuffer` on both backends—not one identical binary:

- native builds link the normal Steam Audio C/C++ library and run the service on
  an Afterglow native worker thread;
- web builds compile the same service boundary with Steam Audio's Emscripten
  library and run it behind the existing shared-ring Web Worker transport.

Upstream's Emscripten archive cannot be linked directly into Afterglow's current
`wasm32-unknown-unknown` service module. The clean prototype is either a
`wasm32-unknown-emscripten` service target with an adapted loader, or a thin
Emscripten module adapter behind the same generated method IDs and ring framing.
Do not leak this toolchain difference into game APIs.

One acoustics worker should own the scene initially. Additional simulation
workers duplicate scene/context memory and are justified only by measurements.
The browser still requires a separate AudioWorklet execution context for
hard-deadline sample processing; ray tracing and scene mutation must never run
in its callback. The worklet is a renderer endpoint fed by bounded rings, not a
second game-facing service. Native builds use the corresponding real-time audio
callback endpoint.

## Measured direct-path prototype

A real Steam Audio 4.8.1 Emscripten prototype now runs the built-in ray tracer
in a CEF Web Worker. It uses WASM SIMD128, fixed 64 MiB memory, the Afterglow
SPSC ring layout, and payload-free wake messages. Occlusion and material
transmission outputs were validated against a wall before timing.

On fox-workstation (Ryzen 9 9950X3D, Chromium 149), five launches × 2,000 measured calls
per scenario produced:

| Geometry / active sources | Direct compute, occluded | Ring+wake mean | Worst p99 | Worst observed |
|---|---:|---:|---:|---:|
| 24 triangles / 1 | 0.094 µs | 11.37 µs | 25 µs | 0.520 ms |
| 10K triangles / 1 | 0.074 µs | 8.97 µs | 20 µs | 0.175 ms |
| 100K triangles / 1 | 0.078 µs | 9.10 µs | 20 µs | 0.690 ms |
| 10K triangles / 32 | 2.12 µs | 12.37 µs | 25 µs | 0.255 ms |
| 10K triangles / 128 | 8.53 µs | 17.47 µs | 30 µs | 0.560 ms |

Therefore a fresh single-ray direct result does not itself require a 10–30 ms
lookahead on this hardware. One 128-frame 48 kHz quantum (2.67 ms) is ample in
the measured idle case. Source and methodology are in
`prototype/steam-audio-wasm/`; raw data is in
`docs/benchmarks/steam-audio-wasm-direct-2026-07-18.json`.

## Measured fully dynamic reflections

The expanded WASM prototype was run on fox-laptop (Ryzen 7 6800U, Chromium 149)
with 10K runtime triangles and **no baked acoustic data**. A direct-path pass
first measured 55 µs worst p99 for 100K triangles / one source and 105 µs worst
p99 for 10K triangles / 128 sources, with one rare 5.30 ms scheduler outlier.
Before every reflection sample it
moved and committed an instanced door, moved the source(s) and listener, ran
Steam Audio's real-time reflection simulator, retrieved fresh parametric or
convolution output, and applied Steam Audio's reflection DSP to 128-frame audio
buffers. Five launches produced:

The ray count is shared per simulation update, not per source: Steam Audio
traces these primary rays from the listener and evaluates active source
contributions along the paths.

| Mode | Sources | Global listener rays × bounces | Simulation mean | Worst p99 | DSP / 2.67 ms quantum |
|---|---:|---:|---:|---:|---:|
| Parametric low | 1 | 1,024 × 2 | 1.02 ms | 1.70 ms | 0.020 ms |
| Parametric medium | 1 | 4,096 × 4 | 7.82 ms | 10.95 ms | 0.017 ms |
| Parametric high | 1 | 16,384 × 8 | 62.01 ms | 66.09 ms | 0.016 ms |
| Parametric medium | 8 | 4,096 × 4 | 30.72 ms | 33.43 ms | 0.123 ms |
| Parametric medium | 32 | 4,096 × 4 | 108.74 ms | 114.83 ms | 0.513 ms |
| Convolution low | 1 | 1,024 × 2 | 1.16 ms | 1.79 ms | 0.021 ms |
| Convolution medium | 1 | 4,096 × 4 | 9.02 ms | 12.29 ms | 0.098 ms |
| Convolution medium | 8 | 4,096 × 4 | 41.83 ms | 44.51 ms | 0.856 ms |

Door/source/listener update plus scene commit stayed below 0.025 ms worst p99.
All IR handles were valid, DSP output energy was non-zero, and parametric RT60
changed over motion. With 32 diffuse samples and ray batch size 1, the idle
Worker supports low and medium one-source simulation at 60 Hz, eight medium
sources at approximately 20–30 Hz, 32 at 8 Hz, and high rays near 10 Hz.

Valve's released WASM archive aborts when reflections construct `std::thread`,
because the archive was not compiled for Emscripten pthreads. Re-linking it with
`-pthread` also fails because its objects lack atomics/bulk-memory target
features. The prototype therefore rebuilds pinned 4.8.1 source with a small
WASM-only patch that executes ThreadPool jobs synchronously inside the dedicated
simulation Web Worker. No simulation work moves onto the main or audio thread.

The session auto-locked during final runs. The reported values are synchronous
Worker CPU timings rather than rAF/presentation timings and were stable, but
render-loaded unlocked testing remains necessary. Reflection DSP was measured in
the Worker, not yet hosted under an actual AudioWorklet callback deadline.
Raw data:
`docs/benchmarks/steam-audio-wasm-dynamic-fox-laptop-2026-07-18.json`.

## Optimal many-source configuration

An unlocked five-launch sweep added nearest-interpolated Steam Audio HRTF cost
for every source and searched 28 reflection configurations. For 64 sources,
one-bounce parametric tiers stayed at the 0.10 s RT60 estimator floor. Two
bounces with 256 or 384 global rays showed much larger RT60 excursions than 512;
1,024 rays doubled simulation cost for little additional stability. The measured
knee was therefore **512 global listener rays × 2 bounces, 500 ms parametric
reverb, order 0**:

| Independently reflected + HRTF sources | Simulation worst p99 | Reflection + HRTF per 2.67 ms quantum |
|---:|---:|---:|
| 32 | 7.43 ms | 0.592 ms (22%) |
| **64** | **15.25 ms** | **1.215 ms (46%)** |
| 96 | 20.78 ms | 1.850 ms (69%) |
| 128 | 29.50 ms | 2.517 ms (94%) |

The recommended 6800U configuration is **128 direct-ray + HRTF sources with 64
priority reflection slots** at 30 Hz steady cadence and 60 Hz bursts after
important motion. Measured 64-source reflection DSP plus 128-source nearest-HRTF
processing projects to 1.364 ms (51%) of the quantum, leaving 1.30 ms for direct
effects, mixing, callback, and browser overhead. Reflecting all 96 sources is an
aggressive option; reflecting all 128 is rejected because only 0.15 ms remains.

Parametric order 0 was decisively preferable for density. Thirty-two order-1
convolution sources consumed 2.115 ms per quantum, while 64 order-1 convolution
sources deterministically exhausted the fixed 256 MiB WASM memory in fresh
initialization. Directional convolution should be restricted to a smaller
priority set rather than applied to every source.

Raw evidence:
`docs/benchmarks/steam-audio-wasm-many-sources-fox-laptop-2026-07-18.json`.

## Native-worker comparison

The matching 28 scenarios plus the native-only 64-source order-1 convolution
case ran in a real `std::thread` on fox-laptop against Valve's released Linux
x64 `libphonon.so` with AVX2. Five launches each tested one, two,
and four Steam Audio reflection-simulation threads:

| Backend / simulation threads | 64 sources, 512×2 mean | Worst p99 | 64-source reflection + HRTF DSP |
|---|---:|---:|---:|
| WASM / synchronous Worker | 13.17 ms | 15.25 ms | 1.215 ms |
| Native / 1 | 16.48 ms | 19.59 ms | 1.324 ms |
| **Native / 2** | **9.27 ms** | **10.74 ms** | **1.330 ms** |
| Native / 4 | 5.53 ms | 6.44 ms | 1.357 ms |

Native does not automatically win when constrained to one Steam simulation
thread: the released library's worker handoff made that configuration slower
than the synchronously patched WASM Worker. Two threads are the balanced native
tier. They preserve the 128 direct-ray + HRTF / 64 priority-reflection policy and
project to 1.433 ms per audio quantum. Four threads are the quality tier: 64
sources at 1,024 global rays × 2 bounces measured 12.52 ms worst p99 and 1.452 ms
projected DSP.

System monitors mostly showed one busy core because Steam Audio only uses its
thread pool during `iplSimulatorRunReflections`. Per-source reflection and HRTF
effects remain serial on the outer native worker and dominate the benchmark's
wall time. The reflection timings prove actual parallelism: 16.48 → 9.27 → 5.53
ms mean for one → two → four simulation threads.

Native removed the fixed-WASM-heap failure for 64-source order-1 convolution,
but did not make it viable: it consumed 3.70 ms DSP, reached 50.01 ms one-thread
simulation p99, and pushed process peak RSS to roughly 283 MiB. Reflecting all
128 parametric sources likewise exceeded the audio deadline at about 3.09 ms.

Raw evidence:
`docs/benchmarks/steam-audio-native-many-sources-fox-laptop-2026-07-18.json`.

## Recommendation

Use a zero-baked-acoustics baseline:

1. binaural direct sound, attenuation, directivity, and bounded direct rays for
   every relevant source;
2. fully dynamic low reflections continuously;
3. prioritize medium convolution or parametric reflections for the most
   important sources and update lower-priority sources less often;
4. degrade ray count, bounce count, source count, duration, order, and cadence
   under pressure—not to baked probes or static impulse responses;
5. retain and smoothly crossfade the latest dynamic result when a source misses
   its update budget.

For native CEF, use the measured two-simulation-thread `libphonon` tier in an
Afterglow native worker; reserve four simulation threads for a higher-quality
option. Processed PCM or acoustic results still need a bounded low-latency path
into CEF's browser audio output, and serial DSP must remain capped at 64
independently reflected sources.

## Primary sources

- [Steam Audio 4.6.0 release](https://github.com/ValveSoftware/steam-audio/releases/tag/v4.6.0)
- [Steam Audio source and supported-platform README](https://github.com/ValveSoftware/steam-audio)
- [Core CMake Emscripten target](https://github.com/ValveSoftware/steam-audio/blob/master/core/CMakeLists.txt)
- [WASM platform detection](https://github.com/ValveSoftware/steam-audio/blob/master/core/src/core/platform.h)
- [Issue #234: WebAssembly/WebGL compilation](https://github.com/ValveSoftware/steam-audio/issues/234)
- [Issue #496: missing dependencies in released WASM library](https://github.com/ValveSoftware/steam-audio/issues/496)
- [Chrome AudioWorklet + WebAssembly design pattern](https://developer.chrome.com/blog/audio-worklet-design-pattern/)
- [Emscripten Wasm Audio Worklets API](https://emscripten.org/docs/api_reference/wasm_audio_worklets.html)

A fuller source snapshot is stored privately in
`skyne98/kb-steam-audio-webassembly`.
