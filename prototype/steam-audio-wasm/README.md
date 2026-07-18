# Steam Audio WASM direct-ray latency prototype

This prototype links Valve Steam Audio 4.8.1's experimental Emscripten library
with the missing WASM MySOFA, PFFFT, and zlib dependencies built from Valve's
pinned sources. It executes Steam Audio's built-in ray tracer in a Web Worker.
Requests and responses use the same SharedArrayBuffer SPSC ring layout as
Afterglow and payload-free `postMessage` wake-ups.

It measures **direct raycast occlusion plus one material-transmission ray**. It
does not measure reflection simulation, convolution, AudioWorklet scheduling, or
a loaded game/render workload.

## Build

The first build downloads roughly 200 MB and builds the missing dependencies.
Build products and upstream sources remain under ignored `target/` and `dist/`
directories.

```sh
nix-shell -p emscripten python3 cmake git curl unzip --run \
  './prototype/steam-audio-wasm/build.sh'
```

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

## fox-laptop result — 2026-07-18

Environment: Ryzen 7 6800U, CEF/Chromium 149.0.7827.201, Steam Audio 4.8.1,
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
