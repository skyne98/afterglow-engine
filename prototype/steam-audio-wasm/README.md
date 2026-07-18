# Steam Audio WASM dynamic-acoustics prototype

This prototype links Valve Steam Audio 4.8.1's experimental Emscripten library
with the missing WASM MySOFA, PFFFT, and zlib dependencies built from Valve's
pinned sources. It executes Steam Audio's built-in ray tracer in a Web Worker.
Requests and responses use the same SharedArrayBuffer SPSC ring layout as
Afterglow and payload-free `postMessage` wake-ups.

It measures direct raycast occlusion/transmission and fully runtime-generated
parametric or convolution reflections. The dynamic benchmark moves an instanced
door, sources, and listener before every simulation; it uses no baked acoustic
data. It also applies Steam Audio's reflection DSP to 128-frame buffers, but does
not yet host that DSP in an AudioWorklet or run alongside a loaded game renderer.

## Build

The first build downloads roughly 200 MB and builds the missing dependencies.
Build products and upstream sources remain under ignored `target/` and `dist/`
directories.

```sh
nix-shell prototype/steam-audio-wasm/toolchain.nix --run \
  './prototype/steam-audio-wasm/build.sh'
```

`toolchain.nix` pins nixpkgs revision `aa290c9891fa` and Emscripten 4.0.23 so
recorded benchmark builds do not vary with the host's channel. The matching
native OS-thread benchmark uses Valve's released Linux x64 library:

```sh
nix-shell prototype/steam-audio-wasm/toolchain.nix --run \
  './prototype/steam-audio-wasm/build-native.sh'
./prototype/steam-audio-wasm/dist-native/native-dynamic-benchmark 2
```

The argument is Steam Audio's reflection-simulation thread count. The benchmark
itself always runs inside one outer `std::thread`, matching a native engine
worker. To download, cook, and build all three scenes in the real Amazon
Lumberyard Bistro package:

```sh
nix-shell prototype/steam-audio-wasm/toolchain.nix --run \
  './prototype/steam-audio-wasm/build-native-bistro.sh'
./prototype/steam-audio-wasm/dist-native/native-bistro-geometry-benchmark \
  target/bistro-source/BistroInterior.acoustic.bin 2 embree
```

The official archive is large (about 894 MiB compressed); generated assets stay
under ignored `target/` and are never committed.

To run through CEF's cross-origin-isolated custom scheme during development:

```sh
rm -rf crates/afterglow-web/www/steam-audio-prototype
cp -r prototype/steam-audio-wasm/dist \
  crates/afterglow-web/www/steam-audio-prototype
# launch the minimal CEF example, then navigate DevTools/CDP to:
# afterglow://local/steam-audio-prototype/index.html
```

Remove the temporary `www/steam-audio-prototype` directory before running the
canonical web artifact check.

## fox-workstation result — 2026-07-18

Environment: Ryzen 9 9950X3D, CEF/Chromium 149.0.7827.201, Steam Audio 4.8.1,
Emscripten 5.0.7, `-O3`, WASM SIMD128, fixed 64 MiB memory. Five launches, 100
warm-up calls and 2,000 measured worker round trips per scenario per launch.
Batched compute means use 10,000 Steam Audio simulations per source state.

| Triangles | Sources | Occluded compute | Visible compute | Ring+wake mean | Worst p99 | Worst observed |
|---:|---:|---:|---:|---:|---:|---:|
| 24 | 1 | 0.094 µs | 0.048 µs | 11.37 µs | 25 µs | 0.520 ms |
| 10,000 | 1 | 0.074 µs | 0.049 µs | 8.97 µs | 20 µs | 0.175 ms |
| 100,000 | 1 | 0.078 µs | 0.048 µs | 9.10 µs | 20 µs | 0.690 ms |
| 10,000 | 32 | 2.12 µs | 1.36 µs | 12.37 µs | 25 µs | 0.255 ms |
| 10,000 | 128 | 8.53 µs | 5.45 µs | 17.47 µs | 30 µs | 0.560 ms |

The occluded result was exactly `0`, the visible result exactly `1`, and the
wall transmission bands were `0.05 / 0.03 / 0.01`, proving that the measured
path traversed Steam Audio geometry and acoustic material handling.

## Interpretation

A fresh direct result does **not** require a 10–30 ms simulation delay on this
machine. In an idle CEF process, the complete worker-ring-wake round trip stayed
at or below 30 µs p99 and below 0.7 ms across 50,000 measured calls. One 128-frame
48 kHz audio quantum is 2.67 ms, leaving substantial direct-ray margin.

This does not establish a shipping bound. Rendering load, browser background
scheduling, volumetric occlusion, complex overlapping BVHs, dynamic scene
commits, reflections, and AudioWorklet/device buffering still need separate
measurements. The correct next gate is the same test during Dungeon rendering,
followed by bounded reflection tiers and end-to-speaker scheduling.

Raw measurements: `docs/benchmarks/steam-audio-wasm-direct-2026-07-18.json`.

## Fully dynamic fox-laptop result — 2026-07-18

The dynamic benchmark ran in CEF/Chromium 149 on fox-laptop's Ryzen 7 6800U
with a 10K-triangle runtime scene. Every sample moved and committed an instanced
door, moved the source(s) and listener, generated fresh non-baked reflection
results, and passed them through Steam Audio's real reflection effect. Five
launches were recorded. The laptop direct-path pass also remained cheap: 100K
triangles / one source had 55 µs worst p99 ring round trips, while 10K triangles
/ 128 sources had 105 µs worst p99. A rare scheduler outlier reached 5.30 ms.

`numRays` is a shared simulation input: these are total listener rays per
reflection update, **not rays per source**. Additional sources increase source
connection/accumulation work without multiplying the primary listener-ray count.

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

Runtime door/source/listener updates and scene commits had a worst p99 of 0.025
ms. All reflection IR handles were valid, all DSP outputs had non-zero energy,
and the parametric low-band RT60 changed over motion (for example 0.10–0.21 s),
showing that the results were refreshed rather than static or baked. Each tier
uses 32 diffuse samples and ray batch size 1.

The result establishes that fully dynamic acoustics are feasible, but require a
bounded quality scheduler. In this idle Worker test, low and medium
single-source reflections fit 60 Hz; eight medium sources fit roughly 20–30 Hz;
32 medium sources fit 8 Hz. High ray counts should update near 10 Hz. Direct
rays remain suitable for all relevant sources at much higher cadence.

Valve's released WASM archive cannot run reflections because it constructs
`std::thread` from a non-pthread build. The reproducible build patches the
upstream ThreadPool to process synchronously inside the dedicated Web Worker and
rebuilds `libphonon.a`; this avoids nested workers while preserving all dynamic
simulation work.

The final reflection runs occurred after GNOME auto-locked the session. They use
synchronous Worker CPU timing rather than rAF/presentation timing and were stable
across launches, but a render-loaded unlocked Dungeon run and a real
AudioWorklet deadline test remain required shipping gates.

Raw measurements:
`docs/benchmarks/steam-audio-wasm-dynamic-fox-laptop-2026-07-18.json`.

## Many-source optimum on fox-laptop

A second unlocked, idle-inhibited sweep measured 28 configurations over five
launches. It included the real per-source Steam Audio reflection effect and
nearest-interpolated binaural HRTF processing for every source. Ray counts remain
global listener rays per update.

The quality/performance knee for 64 independently reflected sources was:

| 64-source parametric tier | Simulation worst p99 | Combined reflection + HRTF / quantum | Observed low RT60 range |
|---|---:|---:|---:|
| 256 rays × 2 | 7.47 ms | 1.212 ms | 0.10–0.32 s |
| 384 rays × 2 | 12.26 ms | 1.218 ms | 0.10–0.25 s |
| **512 rays × 2** | **15.25 ms** | **1.215 ms** | **0.10–0.21 s** |
| 1,024 rays × 2 | 29.24 ms | 1.217 ms | 0.10–0.19 s |

The lower ray tiers produced large RT60 excursions; one-bounce tiers remained
at Steam Audio's 0.10 s estimator floor and did not produce useful dynamic late
reverb. The 512×2 tier was the first stable knee while still fitting a 16.67 ms
simulation burst deadline.

Scaling that configuration by source count:

| Reflected + HRTF sources | Simulation worst p99 | Combined DSP / 2.67 ms quantum | Verdict |
|---:|---:|---:|---|
| 32 | 7.43 ms | 0.592 ms (22%) | Large margin |
| **64** | **15.25 ms** | **1.215 ms (46%)** | Recommended maximum |
| 96 | 20.78 ms | 1.850 ms (69%) | Aggressive 30 Hz tier |
| 128 | 29.50 ms | 2.517 ms (94%) | Rejected: no callback margin |

Recommended 6800U policy: allow **128 audible direct-ray + HRTF sources**, but
cap independent reflections at **64 priority sources**. Use 512 global reflection
rays, two bounces, 500 ms parametric tail, order 0, 30 Hz steady updates, and 60
Hz bursts after important motion. Combining the measured 64-source reflection
cost with 128-source HRTF costs about **1.364 ms (51%)** of the quantum, leaving
1.30 ms for direct effects, mixing, callback, and browser overhead. Directional
convolution should be limited to a much smaller priority set: 32
order-1 convolution sources already consumed 2.115 ms per quantum, and 64 order-1
sources deterministically exceeded the fixed 256 MiB WASM memory during fresh
initialization.

The combined DSP measurement excludes actual AudioWorklet callback overhead,
final source mixing, and the browser/device graph. Therefore 96 is an opt-in
stress tier, not a default. Raw evidence:
`docs/benchmarks/steam-audio-wasm-many-sources-fox-laptop-2026-07-18.json`.

## Native-worker comparison on fox-laptop

The identical runtime geometry, motion path, source counts, ray tiers, and DSP
ran against Valve's released Linux x64 `libphonon.so` with AVX2. Five launches
were recorded at one, two, and four Steam Audio simulation threads. The outer
benchmark worker was always one real `std::thread`; the main thread only joined
it.

Matching the web-selected 64-source, 512-ray × 2-bounce tier:

| Backend / Steam simulation threads | Simulation mean | Worst p99 | 64-source reflection + HRTF DSP |
|---|---:|---:|---:|
| WASM Web Worker / synchronous | 13.17 ms | 15.25 ms | 1.215 ms |
| Native worker / 1 | 16.48 ms | 19.59 ms | 1.324 ms |
| **Native worker / 2** | **9.27 ms** | **10.74 ms** | **1.330 ms** |
| Native worker / 4 | 5.53 ms | 6.44 ms | 1.357 ms |

The balanced native policy is therefore **two Steam Audio simulation threads**,
128 direct-ray + HRTF sources, 64 priority reflection slots, 512 global rays,
two bounces, 500 ms parametric/order-0 tails, 30 Hz steady updates, and 60 Hz
bursts. Measured 64-source reflection DSP plus 128-source HRTF projects to
**1.433 ms (54%)** per quantum. Four simulation threads are a higher-CPU tier;
they can raise reflections to 1,024 rays × 2 while retaining 12.52 ms worst p99
and 1.452 ms projected DSP.

Most wall-clock CPU still appears on one core by design. Steam Audio's thread
count parallelizes `iplSimulatorRunReflections`; it does not parallelize the
per-source `iplReflectionEffectApply` or `iplBinauralEffectApply` loops. The
benchmark performs 1,000 serial parametric DSP quanta after each short simulation
sample set, so process monitors mostly show the outer worker. The simulation
curve nevertheless demonstrates real parallel work: 16.48 → 9.27 → 5.53 ms
mean for one → two → four threads.

Reflecting all 96 sources consumed about 2.12 ms per quantum and is too close to
the device deadline; all 128 exceeded it at about 3.09 ms. Native 64-source
order-1 convolution no longer OOMed, but reached 3.70 ms DSP, 50.01 ms one-thread
simulation p99, and roughly 283 MiB process peak RSS, so it remains rejected.

Raw evidence:
`docs/benchmarks/steam-audio-native-many-sources-fox-laptop-2026-07-18.json`.

## Full Amazon Lumberyard Bistro package

The synthetic 10K scene was not representative because most filler triangles
were distant. The native benchmark now cooks and tests every distinct FBX in the
official CC-BY 4.0 Bistro v5.2 package. Interior and Interior-with-wine are tested
separately because they are overlapping variants, not geometry to merge:

| Scene | Cooked vertices | Cooked triangles |
|---|---:|---:|
| Exterior | 2,883,704 | 2,832,120 |
| Interior | 813,360 | 1,046,609 |
| Interior with wine | 1,067,322 | 1,320,323 |

Assimp triangulates and flattens node transforms, all resulting render triangles
are retained, and render material names map to six acoustic categories. Each
scene uses its authored camera, 64 sources, two bounces, a 500 ms parametric tail,
and no baked data. The original built-in Steam Audio tracer missed 60 Hz on every
full render scene, including with eight simulation threads. Steam Audio 4.8.1's
embedded **Embree 4.4.0** changes that result decisively. Five launches measured
each scene/thread cell:

| Scene | Embree 2 threads, 512×2 p99 | Embree 2 threads, 1,024×2 p99 | Embree 4 threads, 512×2 p99 | Embree 4 threads, 1,024×2 p99 |
|---|---:|---:|---:|---:|
| Exterior | 3.90 ms | 5.52 ms | 3.58 ms | 3.55 ms |
| Interior | 3.19 ms | 6.56 ms | 2.04 ms | 2.75 ms |
| Interior with wine | 3.64 ms | 6.93 ms | 2.90 ms | 3.36 ms |

All full render scenes now sustain strict 60 Hz simulation p99 at both quality
tiers, even with two simulation threads. Against matching four-thread built-in
runs, Embree improved mean simulation time by **18.8–22.9×**. This does not alter
the separately measured serial DSP cap: 128 direct+HRTF sources with 64 reflected
sources remains the endpoint.

Embree also moved acceleration-structure construction from `iplStaticMeshCreate`
to `iplSceneCommit` and reduced total build time substantially:

| Scene | Built-in build | Embree build | Embree scene RSS | Embree peak RSS |
|---|---:|---:|---:|---:|
| Exterior | 7.81 s | 0.52 s | 486 MiB | 487 MiB |
| Interior | 2.53 s | 0.19 s | 183 MiB | 184 MiB |
| Interior with wine | 3.25 s | 0.25 s | 228 MiB | 228 MiB |

Embree is the required native backend. Full render geometry is no longer a
ray-traversal deadline blocker, but the Exterior still consumes nearly 486 MiB
for its acoustic scene and includes acoustically irrelevant detail. Structural
proxies remain required for bounded engine memory, faster loading, controllable
materials, and correct geometry policy—not to rescue reflection simulation time.

Attribution: *Amazon Lumberyard Bistro, Open Research Content Archive (ORCA)*,
Amazon Lumberyard, July 2017,
[CC-BY 4.0](https://creativecommons.org/licenses/by/4.0/), from
[NVIDIA ORCA](https://developer.nvidia.com/orca/amazon-lumberyard-bistro).
Raw evidence:
`docs/benchmarks/steam-audio-native-bistro-full-package-fox-laptop-2026-07-18.json`
(built-in baseline) and
`docs/benchmarks/steam-audio-native-bistro-embree-fox-laptop-2026-07-18.json`
(Embree).

Build and run one cell with:

```sh
nix-shell prototype/steam-audio-wasm/toolchain.nix --run \
  'prototype/steam-audio-wasm/build-native-bistro.sh'
nix-shell prototype/steam-audio-wasm/toolchain.nix --run \
  'prototype/steam-audio-wasm/dist-native/native-bistro-geometry-benchmark \
   target/bistro-source/BistroExterior.acoustic.bin 2 embree'
```
