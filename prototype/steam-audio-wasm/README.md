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
recorded benchmark builds do not vary with the host's channel.

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

| Mode | Sources | Rays × bounces | Simulation mean | Worst p99 | DSP / 2.67 ms quantum |
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
