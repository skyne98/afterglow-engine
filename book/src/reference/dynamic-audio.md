# Dynamic Audio

Afterglow's Steam Audio evaluation uses fully runtime acoustic simulation—no
baked probes or impulse responses.

## Scene backends

- **Native CEF and other native hosts:** Steam Audio 4.8.1's embedded Embree
  4.4 backend on a native `afterglow-rpc` OS worker.
- **Public web only:** pure-Rust `obvhs` 0.3.2 CWBVH8 through
  `IPL_SCENETYPE_CUSTOM` inside the audio Web Worker.

The web tracer is linked into the same Emscripten module as Steam Audio. Rays do
not cross JavaScript. Its closest-hit, any-hit, and batched callbacks use a fixed
traversal stack and allocate nothing after scene construction. A local SIMD128
kernel intersects four CWBVH children at once, while a fixed two-worker pthread
pool parallelizes Steam Audio simulation. Both workers are created at bootstrap.

Static triangles, material IDs, and materials are copied into tracer-owned
memory. The evaluated moving door uses an immutable BLAS plus an atomic
translation; moving it transforms rays instead of rebuilding geometry.

## Validated baseline

On the Ryzen 7 6800U, the measured balanced prototype baseline is:

- up to 128 audible direct-ray and nearest-HRTF sources;
- 64 reflected-source slots; the current gate candidate admits 16 on both
  targets;
- 512 global reflection rays and two bounces;
- a 500 ms parametric order-0 tail;
- 30 Hz steady reflection updates with bounded 60 Hz motion bursts.

Five fresh browser-harness launches hosted by CEF measured the web
SIMD+pthread obvhs path at **4.47 ms mean** and **6.235 ms worst p99**, with no
16.667 ms misses. These are public-web backend measurements, not evidence for
the required native CEF path. Every launch
reported two simulation threads and four SIMD lanes, produced valid dynamic IRs,
and had non-zero DSP output. Scalar one-thread obvhs measured 12.27 ms mean, so
the combined path reduced simulation time by 63.6%. The 10,000-triangle test
scene used 1,261 CWBVH nodes and 661,048 reported owned bytes.

The web path was also run against every full-resolution Bistro scene—up to
2,832,120 triangles—using five fresh Worker/WASM builds per scene. All runs
loaded, built, reported two threads/four SIMD lanes, and returned varying valid
IR output. At 512×2, the package-worst p99 was **27.88 ms**: suitable only as a
30 Hz stress tier, not strict 60 Hz. At 1,024×2 it rose to 50.28 ms. Exterior's
CWBVH owned 182,158,408 bytes and built in 2.83 seconds on average inside a
separate fixed-1.5-GiB test module.

Full render meshes are therefore not the production geometry contract. Cook
structural acoustic proxies containing walls, floors, ceilings, doors, portals,
and other large acoustically relevant surfaces. Native Embree makes full Bistro
traversal fast enough, but neither backend makes irrelevant render detail a good
cross-platform memory or geometry policy.

The production design makes `EngineAudio` required and sole. One fixed Worker/
context schedules voices, runs Steam simulation/direct/HRTF/hybrid DSP and
publishes the final mix through a bounded ring. Native uses 128 total voices, 16
complete world-physical voices and eight quanta; public web deliberately uses 16
total, 4 complete world-physical voices and eight quanta. CEF uses a native RPC
OS worker with native Steam Audio/Embree and a native device callback; public
web uses the generated WASM RPC Worker/obvhs and a minimal PCM AudioWorklet. CEF must never instantiate the audio
service as WASM or a Web Worker. Parametric reflection
remains an explicit low-quality tier. Any fatal audio
fault disables all audio and records a high-severity diagnostic; there is no
partial mixer or destination bypass.

Acoustic geometry will be cooked from explicit low-detail glTF nodes shared with
alternate physics/collision authoring. Modular models may carry collider nodes
inside their normal GLB; large worlds may use a companion `*.structural.glb` for
independent cooking and streaming. Blender can export either form directly; an
Afterglow panel writes validated custom properties that the official glTF
exporter carries in `extras`. Primitives carry `physics`, `audio`, or combined usage; each worker still
receives its own specialized cooked representation. Audio automatically streams
pipeline-built traversal tiles around the listener. Runtime may move/resize
prebuilt primitives/convexes and fixed-topology compounds at tick boundaries,
but does not upload arbitrary point/triangle topology. Physics cooking maps
those nodes to Box3D primitives, hulls, static meshes, and optimized baked static
compounds; the Box3D byte blob is target/version-specific derived data, while the
glTF remains the durable source. Core glTF currently has no ratified universal
collider semantics, so names are not the canonical contract and archived/draft
physics extensions are not required.

The earlier public-web four-quantum/16-wet profile failed when a 15.97 ms Worker
interval exceeded 10.67 ms. Rather than ship a second AudioWorklet-owned Steam
lifecycle, the unified Worker architecture now uses the reduced 16/4 profile and
an eight-quantum ring. Its first same-page hardware-WebGPU run completed 38,900
callbacks and 104 Worker-owned simulation updates with zero underruns,
sequence/fatal errors or pump deadline misses; pump mean/max was 0.095/2.265 ms.
Physical capture had no internal silence span above 0.063 ms. The separate Wasm
AudioWorklet DSP experiment remains research evidence only.

The bounded 60-second public-web contention gate passes. The worker now also has
its fixed no-allocation voice scheduler: packed generational handles, reserved
complete-physical slots, explicit nonphysical placements, smooth sample-clock
volume/pause/stop ramps, and generic equal-power crossfades. Backend gain changes
are interpolated inside each 128-frame quantum. Warm-up can now load up to 64
generational resident sounds from strict mono/stereo 48 kHz WAV
PCM16/24/32/float32 data (32 MiB web, 256 MiB native). Steam reads the stable
Rust-owned PCM directly; loop cursors use the device sample clock and one-shots
release automatically. Chunked warm-up RPC also loads checked indexed acoustic
scenes without exceeding the 1 MiB transport frame. A real-assets native gate
played five official Steam Audio speech/noise/impulse WAVs through all three
full-resolution Bistro scenes. Four render-ahead quanta dropped 22–27 callbacks
per 10-second scene while reflection simulation occupied the sole worker; eight
quanta produced zero underruns, sequence errors, or silent frames across 11.25K
callbacks and is now the native default. The web Worker played the same five
sounds for 11,244 callbacks with zero underruns; full Bistro is intentionally not
a web runtime target because normal fixed memory cannot admit render geometry.
This is not yet the completed public service: cooked
Sound metadata and BIG reads, the synchronous page command facade, ECS attachment
poses, streamed/live/procedural PCM producers, failure and sealed-allocation
coverage remain open. Native
completed a 600-second physical-device run with zero underruns through the
native RPC/Embree/ring/device path; final CEF startup composition through
`AppBuilder::on_ready` remains to be integrated.

The remaining shipping gates are structural-proxy acoustic error, traversal-tile
streaming, long hybrid render-ahead scheduling, production PCM producers
including voice chat, simultaneous rendering contention, and measured device
latency/allocation/memory plateau validation. See
[`docs/api/steam-audio.md`](../../../docs/api/steam-audio.md) for the callback
and ownership contract, and the
[spatial-audio integration plan](../../../docs/implementation/spatial-audio-integration-plan.md)
for the proposed production phases and acceptance gates. Spatial audio remains
a validated prototype, not an available engine API, until those gates pass.
