# Dynamic Audio

Afterglow's Steam Audio evaluation uses fully runtime acoustic simulation—no
baked probes or impulse responses.

## Scene backends

- **Native:** Steam Audio 4.8.1's embedded Embree 4.4 backend, normally with two
  simulation threads.
- **Web:** pure-Rust `obvhs` 0.3.2 CWBVH8 through
  `IPL_SCENETYPE_CUSTOM` inside the audio Web Worker.

The web tracer is linked into the same Emscripten module as Steam Audio. Rays do
not cross JavaScript. Its closest-hit, any-hit, and batched callbacks use a fixed
traversal stack and allocate nothing after scene construction. A local SIMD128
kernel intersects four CWBVH children at once, while a fixed two-worker pthread
pool parallelizes Steam Audio simulation. Both workers are created at bootstrap.

Static triangles, material IDs, and materials are copied into tracer-owned
memory. The evaluated moving door uses an immutable BLAS plus an atomic
translation; moving it transforms rays instead of rebuilding geometry.

## Selected capacity

On the Ryzen 7 6800U, the current balanced policy is:

- up to 128 audible direct-ray and nearest-HRTF sources;
- up to 64 independently reflected priority sources;
- 512 global reflection rays and two bounces;
- a 500 ms parametric order-0 tail;
- 30 Hz steady reflection updates with bounded 60 Hz motion bursts.

Five fresh CEF launches measured the web SIMD+pthread obvhs path at **4.47 ms
mean** and **6.235 ms worst p99**, with no 16.667 ms misses. Every launch
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

The remaining shipping gates are structural-proxy acoustic error, simultaneous
rendering contention, and real AudioWorklet/device-buffer scheduling. See
[`docs/api/steam-audio.md`](../../../docs/api/steam-audio.md) for the callback
and ownership contract.
