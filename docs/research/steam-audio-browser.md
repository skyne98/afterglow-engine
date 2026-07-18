# Steam Audio in the browser

**Investigated:** 2026-07-18  
**Upstream version:** Steam Audio 4.8.1  
**Verdict:** technically viable through Valve's experimental Emscripten target,
but not a supported turnkey browser integration. Prototype direct sound, HRTF,
and occlusion first; do not commit the engine to dynamic reflections until they
are measured in an AudioWorklet/Worker implementation.

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
the measured idle case. This is not yet a shipping bound: rendering load,
volumetric occlusion, adversarial BVHs, scene commits, reflections, AudioWorklet
scheduling, and device buffering remain unmeasured. Source and methodology are
in `prototype/steam-audio-wasm/`; raw data is in
`docs/benchmarks/steam-audio-wasm-direct-2026-07-18.json`.

## Recommendation

For the web target, prototype these tiers in order:

1. binaural direct sound;
2. distance attenuation and directivity;
3. bounded geometry occlusion at a lower Worker update rate;
4. baked reflection/reverb data;
5. only then, bounded dynamic reflections.

For native CEF, separately evaluate native `libphonon` in an Afterglow native
worker. It avoids the experimental WASM build and missing-package limitations,
but processed PCM or acoustic results still need a bounded low-latency path into
CEF's browser audio output.

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
