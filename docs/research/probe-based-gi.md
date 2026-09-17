# Probe-based global illumination — technique survey and engine integration plan

> Status: research complete, no implementation. Investigated 2026-09-17.
> Scope: probe-based global illumination for `afterglow-engine` — the technique
> families, the exact data layouts and math of the leading ones, their artifact
> modes, and a cost model built from this repository's own measured tracer and
> GPU numbers.
>
> Method: primary sources are the DDGI papers (JCGT 8.2 and JCGT 10.2), the NVIDIA
> RTXGI-DDGI SDK documentation, the SHARC integration guide, Epic's Lumen
> technical documentation, Unity's Adaptive Probe Volumes documentation, and a
> WebGPU engine that ships DDGI. Every claim is either cited to a source, quoted
> from this repository's measurements, or explicitly marked as an assumption that
> still needs a prototype gate. Numbers from other hardware are labelled with
> that hardware.

## 0. Bottom line

> **Decision (2026-09-17, user): no bake. Probe GI is runtime-only.** The
> offline-cooked probe volume is dropped. Nothing below that depends on baking is
> part of the design; it is retained as reference for the day a static-GI floor
> is wanted. Revisit only if the weakest tier needs indirect light without the
> runtime volume.

**Recommendation: one stage, fully dynamic.**

A single bounded DDGI-style probe volume, traced every frame by a GPU compute
pass over a runtime-built CWBVH8 BLAS, amortized across frames with the RTXGI
probe state machine and importance-weighted ray allocation, blended into a
ping-pong `rgba16float` octahedral atlas, and sampled per fragment through Three's
`IrradianceNode`.

This is affordable because it was **measured**, not assumed: the 680M traverses a
DDGI-shaped 64-rays-per-probe workload at **~270–490 Mrays/s** (§7.3), so a
2,048-probe volume costs **0.12–0.25 ms/frame** at 1/8 amortization and a
8,192-probe volume 0.21–0.25 ms. Hit shading is free relative to traversal, and
scene triangle count barely matters.

Consequences of having no bake:

- **Bootstrap carries the work a bake would have done**: probe validity,
  virtual offset, relocation, deactivation, and dilation all become one-time GPU
  init passes run before `GameplaySealed`. They are measured and telemetried like
  any other warm-up stage, not hidden in a cook.
- **There is no zero-cost static tier.** The low-end option is fewer probes at a
  slower refresh, or no indirect light at all — not "baked instead".
- **Startup convergence must be bounded and measured.** Probes must be converged
  before gameplay, which costs whatever it costs in the warm-up phase.
- Nothing in `afterglow-pipeline`, no `.big` asset type, and no cooked probe
  payload format are needed for this work at all.

**Locked constraint: the system is purely map-agnostic.** The same code, the same
fixed capacities, and the same constants serve a single room and an open world.
No per-level settings, no authored probe volumes, no map-specific bias constants,
and nothing whose cost scales with map size. The design consequences are
non-optional:

- **The probe field follows the camera**, as camera-locked cascades with
  grid-snapped infinite scrolling. An open world has no sensible authored bounds,
  so authored volumes are not the design. A small room simply occupies the
  innermost cascade — that is the only difference between the two cases.
- **Resident capacity is a constant.** World size changes *which* probes are
  resident, never how much memory or compute exists. Streaming and eviction are
  the mechanism, exactly as assets and acoustic tiles already work in this engine.
- **All constants are expressed relative to the field, not to the level.** Bias
  terms are in units of probe spacing (per cascade), not world units, so nothing
  needs retuning when a map changes scale. This is the map-agnostic answer to
  RTXGI's warning that `probeViewBias` is world-space and scene-scale dependent.
- **The atlas is a 2D packed atlas, not a texture array**, because the resident
  probe count would exceed WebGPU's 256-layer default and the design must not
  depend on requesting a raised limit.
- **Geometry is streamed per tile, not one monolithic BLAS.** Probe tracing reuses
  the same structural proxies and the same tile streaming as the acoustic tracer,
  with per-tile BLAS plus an incrementally maintained TLAS. A single world BLAS
  would make load time and memory scale with map size, which is exactly what this
  constraint forbids.
- **Thin geometry fails closed, not leaky.** With a global spacing constant there
  is geometry finer than the grid on every map; the policy is probe validity,
  virtual offset, and deactivation — a probe inside a wall turns **off** and that
  region simply receives no probe GI. Never a leak. No level is required to keep
  its walls thicker than the spacing.
- **Sky versus interior is handled by the trace, not by data.** Ray misses return
  sky/ambient radiance, so exteriors and interiors fall out of the same code path
  with no baked sky occlusion and no per-map interior/exterior flag.

**Delivery shape: one shot, then interactive iteration.** There are no stages or
phases — the whole field is implemented once as a single coherent system, and from
then on it is tuned by measurement with the user in the loop. Feature sets that
earlier drafts listed as "later" (adaptive placement, hash-grid caches, glossy
reuse) are not stages of this design; they are simply **not in it**.

**Do not start with Lumen-style screen-space final gather.** It requires a
G-buffer (depth + normals + velocity) that this engine does not have. See
`docs/api/` for the renderer's actual pass structure.

## 1. Engine ground truth (what exists today)

Verified in this tree at the current revision:

| Fact | Evidence |
|---|---|
| No lighting subsystem at all | No IBL, no SH, no probes, no `PMREMGenerator`, no `envMap`, no `LightProbe` anywhere in authored source. `DirectionalLight`/`HemisphereLight`/`AmbientLight`/`PointLight` are constructed inline by demos (`demos/*/main.ts`, `afterglow-shell/native_game.ts`) |
| Materials are TSL node graphs | `virtual-texturing/virtual-texture-material.ts` builds `MeshStandardNodeMaterial`/`MeshPhysicalNodeMaterial` with custom sampling nodes. Any probe sampler is another node in the same graph |
| There is an irradiance injection point already | The pinned Three build exposes `IrradianceNode extends LightingNode`, whose `setup()` is `builder.context.irradiance.addAssign(this.node)` — the same mechanism Three's `LightProbe` uses. Indirect diffuse is then `irradiance * BRDF_Lambert(diffuse)`, multiplied by `ambientOcclusion` |
| That build has **no** `gi()` TSL node | `aoNode` exists (16 refs), `giNode`/`gi()` do not (0 refs). The `gi` node in the public TSL docs is a newer Three revision than the pinned one |
| Exactly one authored render target exists | `virtual-texture-feedback-pass.ts` (`RG32Uint`, reduced resolution, async readback). This is the established pattern for an extra pass |
| `.big` container has a `Raw` chunk type | `big-format.ts`: `ChunkMeta.type` includes `'Raw'` (variant 2, no metadata). Cooked probe bytes can ship **with no container format change** |
| Cook is offline-only | `afterglow-pipeline` (Basis UASTC, glTF, R16 displacement, `.big` v6) |
| A BVH tracer already exists in-tree | `afterglow-obvhs-tracer` (Rust `obvhs` 0.3.2, CWBVH8) exposed to web as WASM SIMD128 + pthreads and natively through Embree, currently used only by `afterglow-audio-worker` |
| Runtime discipline | `GameplaySealed` forbids general-purpose runtime allocation; fixed capacities; O(1) hot lookups; `RendererSeal` counts pipelines and asserts zero creations after seal |

Two consequences that decide the design space:

- **Probe GI is the engine's first lighting subsystem**, not an upgrade. It must
  therefore also decide things a lighting system normally owns: a light
  description, a cook representation, an indirect-lighting identity, and how
  demos declare lights. Those decisions are listed in §11.
- **GPU ray tracing is unavailable in WebGPU.** WGSL has no ray query. Every
  ray must be a compute-shader BVH traversal (§3.8).

## 2. Taxonomy

| Family | Data model | Dynamic lights | Dynamic geometry | Needs ray tracing | Memory per probe |
|---|---|---|---|---|---|
| Baked SH probe grid / volume (Unity APV, Unreal volumetric lightmaps) | SH L1/L2 per probe on a grid | No (bake) | Yes, lit correctly by sampling | No (offline) | 24–54 B + visibility |
| Baked SH probe volume + sky occlusion | above, plus a per-probe sky visibility channel | Sky direction only | Yes | No | +~6–12 B |
| DDGI (RTXGI) | Octahedral irradiance + octahedral distance moments, 1px border | Yes | Yes | Yes, per frame | ~1.7–2.1 KB |
| DDGI cascades / multivolume | Several DDGI volumes at differing spacing, packed in one atlas | Yes | Yes | Yes | as DDGI × volumes |
| World-space radiance cache (Lumen, SHARC) | Sparse hash-grid radiance probes, SH L1/L2-ish, screen probes too | Yes | Yes | Yes | sparse, view-dependent |
| Reflection probe / GGX-prefiltered cubemap | mip-chained prefiltered cubemap per probe | Yes (re-render) | Yes | No | 100 KB–2 MB |
| Neural radiance cache (RTXGI 2.0 NRC) | MLP weights + per-pixel training | Yes | Yes | Yes | model-sized |

The engine's existing primitives map cleanly onto rows 1–4 and 6. Row 5 is
deferred (§6). Row 7 is out of scope.

## 3. DDGI in detail

Primary source: Majercik, Guertin, Nowrouzezahrai, McGuire, *Dynamic Diffuse
Global Illumination with Ray-Traced Irradiance Fields*, JCGT 8(2), 2019.
Production refinements: Majercik, Marrs, Spjut, McGuire, *Scaling Probe-Based
Real-Time Dynamic Global Illumination for Production*, JCGT 10(2), 2021 — the
same algorithm as incorporated into RTXGI, Unity, and Unreal.

### 3.1 Representation

- **Probe**: a point with values for directions on the sphere.
- **Irradiance**: octahedral parameterization of the sphere unrolled to the unit
  square, all probes packed into one atlas. Paper: **8 × 8 texels**, `RGB10A2`.
- **Distance and distance²**: two channels of the same octahedral map at
  **16 × 16 texels**, `GL_RG16F`. Distance moments are what make visibility
  testable at shading time.
- **1-texel border** around every probe, so hardware bilinear filtering can
  cross the octahedral seam without sampling a neighbour probe.
- RTXGI stores the same data in **texture arrays** (one slice per probe) rather
  than a packed atlas, with explicit `probeNumIrradianceTexels` (including
  border) and `probeNumIrradianceInteriorTexels` fields.

Octahedral mapping is from Cigolle et al., JCGT 3(2), 2014.

### 3.2 Probe grid

- Uniform 3D grid, per-axis spacing independently scalable, origin and counts
  from a `DDGIVolumeDesc`.
- Paper results use grids from **4 × 2 × 4** up to **64 × 64** probes; the
  largest scenes use **32 × 8 × 32 = 8,192 probes**.
- Paper probe spacing: **1–2 m**. RTXGI production guidance: **one probe every
  2–3 m is usually enough**, and *sparse grids often look better than dense
  ones* because dense grids localize each probe's influence and can reveal the
  grid's structure.

### 3.3 Update

For each awake/vigilant probe, cast a ray distribution over the sphere
(spherical Fibonacci point sets per the paper's reference list). Per ray, shade
the hit point with the same shading routine used for camera pixels, and
accumulate the cosine-weighted radiance:

```
irradianceAccum = Σ_rays max(0, texelDir · rayDir) * rayRadiance
newIrradiance[texelDir] = lerp(oldIrradiance[texelDir],
                               perceptualEncode(irradianceAccum),
                               hysteresis)
```

- `hysteresis` in the paper is **0.85–0.98**; higher = smoother and slower to
  converge.
- The paper's reference implementation **updates every probe every frame** and
  dispatches the same ray count per probe. It explicitly tried and rejected
  camera-radius and distance-varying selective schemes as
  parameter-heavy; instead it recommends probe streaming and probe LODs
  (cascades) for large worlds.
- RTXGI's production answer to that cost is the probe state machine (§3.6).
- Rays that hit a backface record **irradiance 0** and **shorten their recorded
  depth by 80%**, so the probe treats backface surfaces as shadowed rather than
  lighting them.

### 3.4 Query

For a shading point and normal:

1. find the 8 surrounding probes (the corners of the grid cell);
2. per probe compute a weight from the **trilinear probe weight**, a
   **backface weight** (is the probe behind the point relative to the normal?),
   and a **visibility evaluation** (Chebyshev over the distance moments, biased —
   §3.5);
3. sample each probe's octahedral map in the direction of the normal;
4. sum with those weights.

Chebyshev visibility uses the probe's mean and mean-square distance along the
sampled direction; the paper applies **mean- and variance-biased** Chebyshev
interpolants to control residual leaking.

### 3.5 Bias terms (light leaks and self-shadowing)

The 2021 paper collapses three previously separate, scene-tuned biases into one
named **self-shadow bias**:

```
BiasVector = (n * 0.2 + ω_o * 0.8) * (0.75 * minDistanceBetweenProbes) * TunableShadowBias
```

- `n` = surface normal, `ω_o` = direction from the sample point to the camera.
- `TunableShadowBias` default **0.3**; higher values are needed when depth
  variance is higher, e.g. with lower ray counts.
- The bias vector is added to the sample point **for the visibility test only**.
- RTXGI additionally documents `probeViewBias`: a **world-space** offset along
  the camera view ray, applied to the shaded point to push it deeper into the
  probe's voxel where mean-distance variance is lower. Because it is a
  world-space value it is **scene-scale dependent** and RTXGI warns its default
  will likely not fit your content.
- Leak fix for thin/zero-thickness walls: zero-thickness planes are not valid
  wall geometry for probe visibility.

### 3.6 Probe states (RTXGI, the biggest single cost saving)

The 2021 paper reports **30–50% average performance improvement** from probe
states, and notes the freed budget is normally spent on *more rays per active
probe* (which lowers safe hysteresis and converges faster) rather than on more
probes.

States and their rule:

- **Deactivated**: probes that remain inside static geometry. Never traced,
  never updated. Only static geometry is considered, so a probe inside dynamic
  geometry is unaffected and turns on when appropriate.
- **Sleeping / Awake**: a probe needs to be awake *if and only if it is shading
  a surface or about to*. Probes that shade static geometry are **Vigilant**
  (always trace) because they propagate 2nd–nth order diffuse light. Camera
  proximity alone is not a reason to wake a probe.
- **Newly Vigilant / Newly Awake**: probes that just appeared (scene init or a
  scrolling volume) get reduced hysteresis so they converge fast.
- Initialization traces rays for **five frames** to decide each new probe's
  position and state, then dynamic-object AABBs are extended by one grid cell
  plus the self-shadow bias to wake sleeping probes conservatively.

### 3.7 Probe relocation

Probes inside static geometry are moved **within their own grid voxel**:
iteratively through the closest visible backface, then away from close front
faces to maximize surface visibility. Probes are **never** moved around dynamic
geometry — the paper states a stable result is preferred over an unstable result
with lower average error. RTXGI ships this as `ProbeRelocationCS.hlsl` with a
matching reset entry point.

### 3.8 What WebGPU changes

| DDGI assumption | WebGPU reality |
|---|---|
| Hardware ray tracing, 1.5–1.66 GRays/s (RTX 2080 Ti, paper Table 1) | No ray query in WGSL. Traversal must be a compute shader over a BVH. Expect **orders of magnitude** less throughput on an iGPU (see §7) |
| `RGB10A2` irradiance | `rgb10a2unorm` is render-attachment-capable in core WebGPU, but storage access requires the optional `texture-formats-tier1` feature and in-place read-write storage requires `texture-formats-tier2`. The validated 680M adapter exposes **neither** (it does expose `rg11b10ufloat-renderable` on its own). So a compute-written irradiance atlas must use a core storage format (`rgba16float`), and DDGI's temporal blend — which reads the previous value and writes the new one — cannot be a single read-write storage pass. Use a **ping-pong atlas pair** or render-attachment blending |
| Large texture arrays | WebGPU `maxTextureArrayLayers` defaults to **256** (the 2,048 slice figure in RTXGI's docs is a D3D12/Vulkan limit). Either request a higher device limit or keep probe counts per volume ≤ array limit |
| Subgroup-free traversal | The validated 680M adapter advertises the `subgroups` feature, so subgroup-aware traversal is available |
| Timer queries | Present: the adapter advertises `timestamp-query`, and the engine already reads per-pass GPU timings |

The 680M adapter limits observed at shell launch: `maxTextureDimension2D`
16384, `maxBindGroups` 8, `maxComputeInvocationsPerWorkgroup` 1024,
`maxStorageBufferBindingSize` 2147483644.

### 3.9 Cascades, scrolling, multi-volume

- **Multiple volumes** at decreasing resolution cascade outward from the camera,
  packed into **one shared texture**, so all volumes update in one trace
  dispatch. Blending walks volumes **densest first**, accumulating weight, and
  stops once total weight reaches 1.0. Falloff is linear over the last grid cell
  along each axis.
- **Infinite scrolling** moves the volume origin with the camera, clearing
  probe slices that wrap. RTXGI documents the wrap condition explicitly.
- The 2021 paper also reuses the irradiance atlas as **prefiltered radiance** for
  recursive glossy reflection, which the authors note forces second-order glossy
  to maximum roughness; they list multiple octahedral radiance maps with
  different cosine powers as future work.

### 3.10 Stated limitations (from the sources, not inferred)

- Low-frequency signal only: high-frequency radiometric and geometric detail is
  not reproduced. RTXGI pairs DDGI with ray-traced AO for this reason.
- Irradiance accumulates temporally, so there is an unavoidable minimum latency
  on lighting changes.
- Probe data is memory-intensive in large environments.
- Ghosting persists for small bright lights (e.g. flashlights). The authors
  decline to fix it globally because it would destabilize other regions.
- No per-probe ray budgeting; the authors found control of (rays per probe,
  probe count) sufficient and judged per-probe apportioning too complex.

## 4. Baked probe volumes (APV-style)

Primary source: Unity Adaptive Probe Volumes documentation (URP/HDRP 17.x).

- Probes are **placed automatically based on geometry density**, so interiors
  get denser probes without manual placement. (Contrast: classic Light Probe
  Groups are manually placed.)
- Data model: **SH per probe**, sampled **per pixel** ("each part of the car
  samples its nearest probes") rather than per object.
- **Streaming** is provided for large open worlds.
- **Sky occlusion**: a baked per-probe sky visibility term lets a moving sky
  drive indirect lighting at runtime **without rebaking**.
- **Scenario blending** blends between multiple baked lighting scenarios (day /
  night) with one animated blend variable.
- Stated limitation: **probe positions cannot be adjusted** inside a volume —
  you influence them by density/bake settings, not by hand.

Why this matters for this engine: the pieces that APV adds on top of naive SH
probes — automatic density, streaming, sky occlusion, scenario blending — all
map onto primitives the engine already owns (§9). The one APV property the
engine needs but does not have is a **probe validity/occlusion term**, because
SH probes have no distance moments to reject occluded probes (that is DDGI's
job). Options: a per-probe baked visibility cube (6–12 B), Unity-style per-pixel
probe occlusion, or probe dilation/interior tests at cook time.

## 5. SH math and storage

Foundation: Ramamoorthi and Hanrahan, *An Efficient Representation for
Irradiance Environment Maps*, SIGGRAPH 2001.

- Irradiance is a very low-frequency function of direction. **L2 (9
  coefficients per channel)** is the standard quality point for diffuse
  irradiance; L1 (4 coefficients) is the cheap tier and is what many engines
  store for real-time probe volumes.
- Order determines representable frequency, not brightness: L2 cannot represent
  a small bright source as a sharp highlight. Bright, small, or strongly
  directional sources in a low-order SH probe show **ringing and negative
  lobes**, i.e. light appearing on the wrong side of the probe.
- Baked-probe storage cost, per probe, at half precision:
  - L1: 4 coeffs × 3 channels × 2 B = **24 B**
  - L2: 9 coeffs × 3 channels × 2 B = **54 B**
  - plus a validity/occlusion term (~4–16 B)
- Compare DDGI: ~1.7 KB (paper formats) to ~2.1 KB (RGBA16F) per probe. Baked
  SH L2 is roughly **35× smaller**.
- Evaluation at runtime: sample SH with the shading normal; with normalized
  direction this is 4 or 9 multiplies per channel. This is cheap enough to run
  per fragment in the existing material node graph.
- Convolution: because irradiance is the cosine-convolved radiance, an
  irradiance-environment map is a direct (already-convolved) SH vector; baking
  can therefore accumulate SH in the same pass that gathers radiance.

## 6. World-space radiance caches (why deferred)

- **Lumen** separates the problem: a world-space radiance cache plus a
  screen-space radiance cache, with **final gather** carrying lighting from
  scene texels to screen pixels. UE 5.8 documents an "Irradiance Field Gather"
  that "places World Space Radiance Cache probes around pixels, pre-calculates
  their irradiance, and interpolates to pixels with probe occlusion" — i.e. the
  DDGI-shaped subset of Lumen.
- **SHARC** (RTXGI 2.0) is a spatially hashed radiance cache with three passes
  (update, resolve, render/query) that traces only a sparse subset of screen
  pixels (reported as ~4%, a 5×5 grid) and serves radiance at any hit point.

Both are **view-dependent radiance caches for path tracing**, not diffuse
irradiance fields, and both assume hardware ray tracing at high ray throughput.
They are the right end-state for a path-traced pipeline and the wrong entry
point for a forward WebGPU engine on an iGPU. Recorded here so the door is
explicitly marked: if the engine ever gets a path-traced reference mode, SHARC's
update/resolve/query split is the design to copy.

## 7. Cost model for this engine

All ray rates below are from **this repository's own measurements** on the
6800U/680M laptop, not from the papers' RTX 2080 Ti.

### 7.1 Measured tracer throughput

| Configuration | Measured | Source |
|---|---|---|
| Web, WASM SIMD128, 2 pthreads, 512 rays × 2 bounces | 4.47 ms mean / 6.235 ms worst p99 | `docs/api/steam-audio.md` |
| Native, Embree, 2 threads, 512 rays × 2 bounces | 3.39 ms mean / 4.316 ms worst p99 | `docs/api/steam-audio.md` |
| Native, Embree, 2 threads, 512×2, package-worst over 3 Bistro scenes | 3.90 ms p99 | `docs/api/steam-audio.md` |

That is roughly **230 ray-hits/ms (web, 2 threads)** and **~300 ray-hits/ms
(native, 2 threads)** — a full ray budget of ~2–3 ms per *second* of probe
updates at 30 Hz in a worker.

### 7.2 What that means for DDGI

Rays needed for one full probe sweep, at 64 rays/probe (paper's largest-scene
setting):

| Probe count | Rays/sweep | Web worker time | Native worker time |
|---:|---:|---:|---:|
| 512 | 32,768 | ~142 ms | ~109 ms |
| 2,048 | 131,072 | ~570 ms | ~437 ms |
| 8,192 | 524,288 | ~2,280 ms | ~1,747 ms |

Even amortized 1/8 per frame, a 2,048-probe volume costs **71 ms of worker CPU
per frame** on web. That is not a frame budget; it is a background job.

**Conclusion: CPU/worker tracing cannot drive dynamic DDGI on this hardware.**
It is sufficient for *static or slowly changing* probe volumes — a 512-probe
volume refreshing over ~1 second — which is a legitimate product decision (the
WebGPU engine surveyed in §12 exposes exactly this: a switch to stop updating
probes once a static scene has converged).

### 7.3 GPU compute tracing — measured, and it passes

The 680M is an RDNA2 part with 12 CUs. A standalone raw-WebGPU benchmark (binary
BVH, 48-entry stack, leaf 4, no child ordering, spherical-Fibonacci ray sets,
64 rays/probe, 25 dispatches per configuration, run through `afterglow-shell` on
this laptop) measured a closest-hit traversal at:

| Triangles | Nodes | Probes | Rays/dispatch | Mode | ms/dispatch | rays/ms | @1/8 per frame |
|---:|---:|---:|---:|---|---:|---:|---:|
| 24,576 | 16,383 | 2,048 | 131,072 | trace | 1.636 | 80,106 | 0.205 ms |
| 24,576 | 16,383 | 2,048 | 131,072 | shade | 0.964 | 135,950 | 0.121 ms |
| 24,576 | 16,383 | 8,192 | 524,288 | trace | 1.680 | 312,084 | 0.210 ms |
| 24,576 | 16,383 | 8,192 | 524,288 | shade | 1.079 | 485,722 | 0.135 ms |
| 96,000 | 65,535 | 2,048 | 131,072 | trace | 0.919 | 142,594 | 0.115 ms |
| 96,000 | 65,535 | 8,192 | 524,288 | trace | 1.959 | 267,592 | 0.245 ms |
| 96,000 | 65,535 | 8,192 | 524,288 | shade | 1.960 | 267,467 | 0.245 ms |

**~270–490 Mrays/s**, i.e. a full 8,192-probe × 64-ray sweep in 1.1–2.0 ms and
a 2,048-probe volume in ~0.9–1.6 ms. This is roughly **1,000× the CPU worker
path** measured in §7.1, and it invalidates the CPU-based pessimism: **dynamic
probe GI is affordable on this hardware.** Consequences:

- Shading at the hit is free relative to traversal (shade rows are within noise
  of trace rows).
- Triangle count is nearly irrelevant (24k vs 96k tris differ by ~15%): BVH
  depth is logarithmic.
- 1/8-amortized updates cost 0.12–0.25 ms/frame at these volumes.

Caveats that remain: this is a binary BVH with no child ordering, no octahedral
atlas accumulation, no border updates, no hysteresis blend pass, and no probe
classification; it ran in isolation at 640×480 with no scene rendering. A
CWBVH8 traversal should be faster per ray (the paper reports 1.9–2.1× over
previous wide-BVH kernels), a 1M-triangle scene slower, and a real DDGI update
loop adds several more dispatches. Per-dispatch measurements also carry up to
one 120 Hz frame of fence latency (8.33 ms ÷ 25 iterations ≈ 0.33 ms) plus
boost/thermal variance — hence the wide ranges.

### 7.4 Frame budget reality check

The Dungeon's measured GPU cost on the 680M at 1440×900 DPR 2 is scene+output
means of **4.19/4.28/5.84 ms non-POM and 6.56/5.49/8.29 ms POM** across the
three canonical poses, with **10.49 ms** worst-pose POM p99
(`docs/api/virtual-texturing.md`).

- At 60 Hz (16.67 ms) there is roughly 8–10 ms of headroom — enough for a
  1–2 ms amortized GI pass.
- At **120 Hz (8.33 ms) the Dungeon already consumes most of the budget**, so
  any GI pass is a 60-Hz-tier feature unless POM/VT costs come down. The dev
  laptop reports a 120.000 Hz cadence, so this is the common case there.

### 7.5 Memory

| Layout | Per probe | 512 probes | 2,048 probes | 8,192 probes |
|---|---:|---:|---:|---:|
| DDGI, paper formats (8×8 RGB10A2 + 16×16 RG16F, 1px borders) | ~1.66 KiB | ~0.85 MB | ~3.4 MB | ~13.6 MB |
| DDGI, RGBA16F irradiance | ~2.05 KiB | ~1.0 MB | ~4.2 MB | ~16.8 MB |
| Baked SH L2 + validity | ~58–70 B | ~30–36 KB | ~119–143 KB | ~475–573 KB |

The memory argument for baking first is decisive on an integrated GPU.

### 7.6 The reference demo — measured state

`prototype/probe-gi-demo/` is the design in §10 running as a raw page in
`afterglow-shell` (no web build, no server): 3 camera-locked cascades × 8×4×8 =
768 resident probes, 64 rays/probe = 49,152 rays/sweep, per-probe 8×8 octahedral
maps with a 1-texel border in a 288² `rgba16float` atlas (663 KB per atlas,
ping-pong), fetched through the atlas sampler.

**The tracer is correct; the sampling is where the work was.** Isolating a
world-blocked enclosure makes this measurable. With the room's doorway sealed, and
cage contributions classified by which probes they came from:

| sealed room, cage excludes… | frame mean |
|---|---:|
| nothing (all 768 probes) | 81.6 |
| probes above the roof | 68.7 |
| probes outside in x/z | 53.9 |
| every probe outside the room AABB (incl. under the floor slab) | **0.0** |

Probes *inside* the enclosure carry exactly zero false light — as they should, since
neither the light nor the sky can reach them — and the entire leak is the cage
interpolating probes that are *outside* it: above the roof, beyond the walls, or
under the floor slab. Removing either of the two roofs/walls classes on its own takes
out roughly a third, and removing all of them takes out everything, so no single axis
is to blame. Absolute levels here move with the scene's lighting: they were 45-74
across the earlier configurations and the *ratios* are the finding. The sealed test keeps
its light outside the room at the same intensity (`lightPosAt`), so the interior has no
legitimate source and the whole signal is leak. It is the documented DDGI limitation
at a ratio of roughly 10:1 (3 m and 9 m cascades against 0.25 m geometry), and at
that ratio no probe-side weighting fixes it: the moments that gate a probe are
*directional*, so a probe above a roof stores open sky in the roof's horizontal
direction and passes a visibility test it should fail.

Two remedies were implemented and both were rejected on measurement, which is the
useful part of the record:

- **Fetching distance moments along the query→probe direction** instead of the
  surface normal tests the real occluder and is geometrically right. It was rejected for
  "visible grain and hard-edged patches", but that measurement predates the atlas-fetch
  mapping fix (the fetch landed on the seam texel, blending every sample with the border);
  re-measured with the fix it costs nothing in high-frequency residual and takes the sealed
  leak 80.5 → 78.2, and it is now what the render does. Small because the 8×8 distance map
  is bilinearly interpolated and smears the moments.
- **A per-pixel ray-traced AO pass.** With a 4 m distance-weighted falloff it was too
  weak to see a 16 m room; made hard (4 rays, 40 m, floor 0.05) it does suppress the
  leak to the floor, but the per-ray binary test quantizes into visible speckle at any
  ray count affordable on this GPU, and it darkens legitimately enclosed interiors
  (the simple scene's shadow went near black). Removed: the demo now casts no
  per-fragment rays beyond the direct-light shadow.

Open engine-level decision, with those numbers: accept the leak for geometry thinner
than the probe spacing, spend frame budget on RTAO (with more rays than fit here), or
place probes densely enough to resolve the geometry.

Defects found while validating, all invisible to the host-side checks the demo had:

1. **View-projection omitted the eye translation.** It shaded as if the eye were at
   the world origin while the demo's own NDC-projection log read correctly — that log
   subtracts the eye by hand, so it agreed with the intended matrix, not the uploaded
   one.
2. **The render pass had no depth attachment** (a leftover bisect), so back faces
   overwrote front faces.
3. **The probe term was divided by π twice**, and the atlas stored a mean radiance
   rather than irradiance, leaving "is this E or E/π" ambiguous at every call site.
   The atlas now stores irradiance `E = π·Σ(L·cos)/Σcos`; the surface reflects
   `albedo·E/π`.
4. **The point light saturated the frame** (intensity 45 with a `1/(1+0.09d²)`
   falloff clipped 90 % of lit geometry).
5. **Both shading biases scaled with cascade spacing.** The ray-origin bias was
   `0.225 × spacing`, i.e. **2.0 m** at the 9 m cascade, which teleports ray origins
   through 0.25 m walls and reports open sky from inside a closed room; the sample bias
   was the same. Both are world-space metres now (0.02 m and 0.19 m).
6. **Probe relocation was unbounded.** `max(farthest, …) × 0.6`, with
   `farthest = reach × 10` for any missing ray, moved probes up to **24 m** (190
   relocated probes in the simple scene) — out of their own cell and often out of the
   enclosure they started in, interpolating that region's lighting back in. Bounded to
   one cell, relocation is now 0 in both scenes.
7. **Cascades switched hard instead of blending**, leaving a jagged seam across the
   floor where sampling changed cascade. Adjacent cascades now blend over a one-cell
   overlap; a point fully inside a cascade still costs one cascade's fetches.
8. **The capture harness dropped the third view whenever shell input arrived before
   the last frame mark**, and its base64 encoder built ~690 KB with `+=` and then
   sliced it — a tens-of-milliseconds block that made the shell's surface acquire time
   out (`Invalid Surface Status: Timeout`) and killed the frame loop after the capture
   set. Captures no longer depend on input; base64 is emitted in fixed-size chunks with
   a yield between them.
9. **A "+1.5 texel-centre" atlas mapping was half a texel wrong.** The interior's outer
   edge is *supposed* to land on the border texel: that neighbour keeps the bilinear
   reconstruction continuous across the octahedral seam (an interior texel's direction
   has to fetch at its own centre, and the edge then lands half a texel outside). The
   correct mapping is `slot·tile + 1.0 + uv·interior`; `selftest.mjs` pins it with a
   round trip against the blend's texel directions (residual 4.4e-16).

10. **Chebyshev³ and the sub-0.2 weight sharpening tiled dim surfaces with the cage
    grid** — reported by the user as the cube's lighting "splitting into harsh uniform
    squares" when zoomed in. The weight-sum view (`configs/repro-weights.js`, debug
    mode 3) shows it directly: bright blobs at cage-cell centres with dark seams at the
    cell boundaries, tiling the ground and every dim face. Mechanism: a query near a cell
    boundary is up to one and a half spacings from its cage probes, so their Chebyshev
    distance term is larger and the shared weight *dips* there; cubing that dip and then
    raising small weights to a third power turns a gentle lattice into hard-edged blocks.
    Both amplifiers exist in the sampled reference for leak suppression, and here they
    measured **no** leak benefit (sealing the doorway and toggling Chebyshev moved the
    leaking wall by 1 % — see the table above, which is unchanged by their removal), so
    both are gone and the gate is applied plain (`w *= cheb`).
11. **The cascade handover was weighted by `wsum × fade` instead of `fade`.** The two
    cascades' cage weight sums differ by an order of magnitude (a coarse cage is larger),
    so whichever cascade had the larger sum dominated until its fade was nearly zero and
    the intended one-cell blend collapsed into a hard line: an axis-aligned step across
    the cube at cascade 0's lower y bound, and a rectangular region on the ground. The
    fade alone now decides the handover; each cascade normalizes its own cage internally.

12. **A reseeded scroll band was a visible step across every lit surface** — the case the
    reporter hit by *moving*: the field scrolls in 1 m steps and reseeds the wrapped band,
    which then held a one-frame atlas value next to probes averaging ~14 frames under
    hysteresis, so with the animated light the band boundary was a hard edge (reproduced
    deterministically with `configs/freshband.js`, which forces a reseeded band every
    frame). A reseeded probe now inherits its **inward neighbour's** atlas — the probe one
    cell back along the scrolled axis, which holds a converged value from one cell away —
    and then uses normal hysteresis instead of starting from nothing
    (`seedSource`/`previousTile`/`probeHysteresis` in `gi.wgsl.js`, `seedHost` on the
    host). A/B at identical frames: with inheritance the face is smooth, without it the
    step is back. This is what the JCGT 2021 paper's "fast-convergence heuristics" are for;
    the failure mode was reachable because the band was reseeded with no history at all.
    Two traps in that fix, both hit while landing it: a seed pointing at a *deactivated*
    probe reads a tile that was never written (undefined texels → NaN → wholly black
    frames), and the store must still target the probe's own tile even though the read
    comes from the seed.

13. **A physically correct dark room reads as a bug.** With the light *outside* the
    doorway, and after the leak fixes removed the sky that used to fill the interior, the
    room's floor was lit only by the direct shaft plus one bounce — so it rendered as a
    near-black floor with hard-edged bright rectangles, which the user reported as "a huge
    black square following below us". Nothing was wrong with the GI: the coarse cascade's
    cage for a floor point is one probe *below* the slab (seeing the unlit void) and one
    inside the dark room (seeing unlit interior), so its irradiance is genuinely ~0, and
    the "square" was the shaft boundary (a rectangle, because the doorway is). The scene
    now puts the room's light *inside* it (intensity 5 rather than 31.5, since it sits 2-4 m
    from the surfaces) while `configs/sealed.js` keeps its light outside so the leak test
    still has no legitimate source. Lesson for the engine: a demo that shows the *bounce*
    needs a source that lights the space, or the absence of leak reads as a black frame.

14. **Hysteresis was applied to the wrong probes.** `probeHysteresis` returned `P.misc.y`
    (0.93, "~14 frames" as its own comment says) only when a probe had a seed, and 0.0 for
    every other probe — but `seedFrom` is set on exactly one frame, the frame a band is
    wrapped, so the condition inverted the rule for the whole field. The atlas then held a
    single frame's 64-ray estimate with no temporal accumulation anywhere, and every
    discrete event (a wrapped band, a relocation, the animated light) appeared at full
    amplitude. It also made the seed inheritance self-defeating: a reseeded band took 93 %
    of its neighbour's converged value on the wrap frame and snapped to its own raw value on
    the next one. Now only "reseeding with nothing to inherit" is zero; everything else
    accumulates. Dispatch order matters and is now documented in the shader: blendDistance
    runs before blendRadiance, which clears the fresh bit, so both passes agree.
15. **The scroll frame rendered the previous frame's cascade origins.** `writeParams()` ran
    before `updateScroll()`, so on a scroll frame the atlas had just been written against
    the new grid while the fragment shader's `cascadeOrigin` still named the old cells: the
    render selected the probes one cell over and the next frame snapped back. Isolated with a
    new diagnostic (`CFG.scrollDrift`, `configs/scroll-only.js`): a static camera, static
    light and only the scroll origin drifting, so consecutive captures differ by the field
    alone. `magick compare -metric MAE` reads exactly 0 on every non-wrap frame and 0.077 % /
    0.27 % (worst pixel 3.9 %) on the two wrap frames. Fixed by writing the params after the
    scroll.
16. **Probes inside solid geometry stayed valid.** `probeInit` counted a ray as "enclosed"
    only when its backface hit was closer than `spacing × 0.5` — 0.5 m at the near cascade —
    so a probe half a metre inside the 1 m ground slab, its surface exactly at the limit,
    scored zero and stayed valid with an all-backface sweep. The CPU reference
    (`probecheck.mjs`) puts 64 of cascade 0's 256 probes in that layer (y = −0.5, mean
    E = 0.000, 48 % of the cascade's rays hitting backfaces), each one a zero-radiance vote
    in every cage near the floor. The distance limit is gone — any backface means the ray
    left a solid, so 5 of 8 deactivates, and front faces are untouched, so a probe merely
    *near* a wall is unaffected. The orbit soak now reports 767/768 active.
17. **The cascade handover let the coarse cascade outvote the fine one** — the mechanism
    behind "a huge black square following below us" and behind lighting that changes as the
    camera moves. Each cascade's share was its own boundary fade, and a coarse cascade is at
    fade ≈ 1 for a point deep inside the *fine* cascade's box, so the shares do not sum to 1
    and the coarse level wins by construction. The CPU reference's cage for a floor point 1 m
    from the camera took 74 % of its weight from the 9 m cascade measured at E = 1.27 against
    the honest in-room 0.08, and the share is camera-dependent because the boxes are
    camera-locked. Weighted now by `fade × Π(1 − fade_finer)` (nested), so inside a cascade
    the finer one always wins. CPU floor profile (camera [0,1.6,−3], query walking 1→8 m):
    0.558 0.560 0.561 0.574 0.626 0.644 0.892 1.119 1.163 1.183 1.201 1.221 1.242 before,
    0.092 0.096 0.100 0.101 0.091 0.087 0.147 0.371 0.506 0.596 0.671 0.723 0.756 after
    (honest in-room level ≈ 0.07-0.09). The near field stops being ~8× too bright and the
    step at the camera-locked boundary drops from 0.25 to 0.06 in absolute irradiance.

The **query→probe distance-moment direction was reinstated** in the same pass. It was
rejected earlier for "grain and hard-edged patches", but that measurement predates the
atlas-fetch mapping fix (slot·tile + 1.0 landed on the seam texel, so every fetch was a
50/50 blend with the border). Re-measured twice now: the high-frequency residual of the
floor (mean |image − gaussian|, `magick` on the decoded captures) is 55.1 / 59.8 / 75.2
along the normal against 52.6 / 57.1 / 76.1 along query→probe — no grain difference — while
the CPU reference shows the direction collapsing the weight of probes behind walls (their
stored depth along that ray is the wall, so `md` is large instead of 0). On the GPU the
sealed-room leak moves only 80.5 → 78.2, because the 8×8 distance map is *bilinearly
interpolated* and smears the moments the CPU function evaluates exactly. Kept: it is the
geometrically right form and its measured cost is zero.

Also fixed in the harness, each after it cost a run: the page reads its JS at load, so editing the demo while aninstance is running changes nothing — a report of "still broken" may be a stale process;
capture payloads are now emitted as individually marked `capture-chunk` lines (the begin/end block format broke when the
encoder started yielding between chunks and ordinary log lines landed inside the block);
the `capture-all-done` marker waits for in-flight readbacks, which it did not, so the
runner killed the shell mid-stream and produced truncated captures; a bind group that did
not match its layout invalidated the whole command buffer and rendered *black with no
log line*, so the page now installs `onuncapturederror`; and `run.sh` preflights every
module with `node --check`.

Reproduction entries are checked in: `configs/repro.js` (the reporter's exact camera
pose, read off their HUD), plus `repro-indirect.js` and `repro-weights.js` for the same
pose with those views isolated. `configs/closeup.js` sweeps three distances over the
cube's shadowed side, and `configs/freshband.js` forces a reseeded scroll band every
frame (`CFG.forceFreshBand`), which is the only way to see that class with a static
camera.

`prototype/gi-ref/` is the separate CPU-first harness, so each stage can be diffed
against a hand-checked reference (probe positions exact, ray hit and radiance within
8e-8, atlas blend within 1.1e-7). It is what caught a genuine f64/f32 tie in its own
reference scene that would otherwise have looked like a shader bug.

**Measurement caveat.** Frame-rate numbers from this prototype are comparable only at
a fixed display configuration. With a second display attached the shell's present path can
inflate latency and cap the rate — every ablation tried in that state (a constant-colour
fragment, a 320×200 window, all compute dispatch counts zeroed) measured the same 60.0 fps
with `gpu` reported at ~42 ms, which is present *latency*, not per-frame cost — but the cap
is not unconditional: a 20 s room soak on 2026-09-18 with `card1-DP-2` attached measured
119.4-119.6 fps at 640×360, while 1280×720 readings in the same session ranged 31-79 fps.
An fps delta measured across display states is therefore meaningless; the 115-119 fps
figures earlier in this document were taken with a single display, and the 640×360 numbers
in the defect list above are from the same session as each other.
## 8. Artifact catalogue

| Artifact | Cause | Mitigation (source-backed) |
|---|---|---|
| Light leaking through walls | Probe sees around a thin/zero-thickness wall | Self-shadow bias (§3.5); never use zero-thickness planes for walls; increase `probeViewBias` (world-space, scale dependent) |
| Self-shadow / acne on surfaces | Bilinear-interpolated depth uncertainty near the surface | Self-shadow bias default 0.3; raise it when ray counts are low |
| Backface bleeding | Ray hits a backface and reports its radiance | Backface hits store irradiance 0 and shorten depth 80% (§3.3) |
| Probe stuck inside a wall | Uniform grid cannot avoid geometry | Probe relocation within the voxel (§3.7), then deactivate if still inside static geometry |
| Wasted work on empty space | Many probes never shade anything | Probe state machine: sleeping/deactivated/vigilant (§3.6), 30–50% saving |
| Noise / fireflies | Low rays per probe, small bright sources | More rays on awake probes, higher hysteresis, and the perception gamma which suppresses low-frequency flicker (§3.3, §3.5) |
| Light-to-dark lag | Temporal accumulation | Hysteresis 0.85–0.98 with the **gamma 5 perception encoding** (converges perceptually linearly, and permits a smaller texture format) |
| Ghosting on small bright lights | Accumulated irradiance | Stated unfixed limitation; mitigate by lowering hysteresis on known problem lights (destabilizing elsewhere) |
| Grid structure visible in the image | Probe grid too dense or badly placed | RTXGI: prefer **sparse** grids, 2–3 m spacing |
| Seams at volume boundaries | Hard cut between volumes | Blend densest-first with linear falloff over the last grid cell, **weighted by `fade × Π(1 − fade_finer)` so the shares sum to 1**, and stop at total weight 1.0 (§3.9). Giving each cascade its own fade as its share lets a coarse cascade at fade ≈ 1 outvote the fine one *inside* its own box — measured 74 % of a floor point's weight from the 9 m cascade, and camera-dependent because the boxes are camera-locked |
| A lit region that moves with the camera | Same as above: the levels disagree (honest in-room E ≈ 0.08 against the coarse cascade's leaked 1.27), so whichever cascade dominates decides the brightness, and the cascade boxes follow the camera | Nested weights (previous row). Also deactivate probes inside solids, so the cage has no zero-radiance votes near the floor |
| Debug view differs from what the frame actually shaded | Params written before the scroll moved the grid | `writeParams()` after `updateScroll()`: one frame is enough for the render to index the probes one cell off |
| Everything relights each frame, no smoothing | `probeHysteresis` returning 0 for every unseeded probe | Hysteresis applies to the whole field; only a probe reseeded with nothing to inherit starts from the current frame |
| Scrolling sweep artifacts | Volume origin moves under the camera | Clear wrapped slices; RTXGI documents the wrap condition |
| Ringing / negative lobes in SH | Bright small source in low-order SH | Raise order, clamp/renormalize, or split the source out as a directly shaded light |
| Dynamic object lit by a probe inside it | Probe passes through moving geometry | Backface heuristics handle it; convergence heuristics restore the probe on exit (§3.7) |

## 9. Mapping onto existing engine primitives

| Probe-system need | Existing primitive | Verdict |
|---|---|---|
| Cooked probe data storage | `.big` container's `Raw` chunk | **No format change needed.** `ChunkMeta.type` already includes `Raw` (variant 2) |
| Cook-time bake | `afterglow-pipeline` (offline) | Reuse; a path tracer is new work, but it may run unlimited time offline |
| Ray tracing for the bake | `afterglow-obvhs-tracer` (CWBVH8) — currently audio-only | Reusable natively via Embree; the crate is already a workspace member |
| Runtime sampling in materials | TSL node graph + `IrradianceNode` | Direct fit: `new IrradianceNode(probeNode)` adds irradiance before AO multiplication |
| Probe atlas/pages on the GPU | VT format pools and atlas upload paths | Partially reusable for the GPU-side atlas if Stage 2 lands |
| Streaming large probe volumes | VT paging + `AssetSource` ranged reads + blob store | Reusable pattern; APV-style streaming is the same problem shape |
| Worker-side tracing service | `#[rpc]` worker + `afterglow-rpc` rings | Direct fit for a low-rate CPU tracer if Stage 2 uses workers |
| Fixed capacity / no runtime allocation | `EngineMemory` arenas, generational handles | Probe volumes must be declared capacities reserved before seal |
| Pipeline sealing | `RendererSeal`, `warmRendererVariants` | Any new pass needs warm-up renders per render-target format and zero post-seal pipelines |
| Telemetry | `afterglow-telemetry` | Probe state counts, sweep latency, convergence, active probes are natural fixed metrics |
| Config/manifest | Demo wiring rules | Probes must be a declarative resource with no per-demo bespoke logic |

Not reusable: virtual texturing is **not** a probe atlas. A probe volume is
addressed by grid cell and direction, not by UV; VT's feedback/priority
machinery has no analogue here. Keep them separate.

## 10. The implementation (one shot) and how we iterate

There is one deliverable, not a staged path. It is implemented whole, then tuned
in the loop with the user against measurements.

**The field.** Three camera-locked cascades with grid-snapped infinite scrolling:
near at 1 m spacing, mid at 3 m, far at 9 m (×3 per cascade), each cascade
8 × 4 × 8 = 256 probes, so **768 resident probes** regardless of map size. A small
room occupies the near cascade only; an open world scrolls all three. The same
constants serve both — that is the map-agnostic requirement, not a coincidence.

**Per frame, one submission into the frame's command buffer:**

1. **trace** — 64 spherical rays per awake probe in WGSL over the CWBVH8 BLAS
   (per-tile BLAS + incremental TLAS). At 768 probes that is **49,152 rays**, which
   at the measured 143k–312k rays/ms is **0.16–0.34 ms for a full-field sweep** —
   so sweep every frame rather than amortizing, and keep amortization in reserve
   for larger fields.
2. **blend** — two no-barrier dispatches (radiance, distance) into a **ping-pong
   `rgba16float` 2D packed atlas pair**, with hysteresis and gamma-5 encoding.
   Radiance atlas: 768 × 10×10 = 76,800 texels → **512×512 RGBA16F = 1 MB**.
   Distance atlas (16×16 interior + border, `rg16float`): 768 × 18×18 = 248,832
   texels → **512×512 RG16F = 0.5 MB**. Both ping-ponged: **~3 MB total**.
3. **sample** — per fragment, the 8-probe cage with trilinear × backface ×
   Chebyshev weighting, feeding Three's `IrradianceNode`. All bias terms are
   expressed in the *cascade's own spacing units*, so no constant is world-scaled.

**Init and streaming.** Probe validity (backface sampling rays) → virtual offset →
relocation within the voxel → deactivation → dilation, all as bootstrap GPU passes
before `GameplaySealed`. Scrolling re-seeds wrapped probes through the same
five-frame init path. Ray misses return sky radiance, which is how interiors and
exteriors share one code path with no baked sky data.

**Never fence per dispatch.** Identical work measured 8.277 ms fenced-per-dispatch
versus 1.636 ms batched (§C.6 of the companion document); the update belongs in the
frame's existing submission.

### What we tune together once it runs

Cascade count and spacings; rays per probe; hysteresis; update cadence (every frame
versus amortized); per-cascade bias multipliers; leak mode; resident capacity. All
of these are bounded integers or small constants in one place, and none of them are
per-map.

### Acceptance (criteria, not gates to pass in order)

- Zero pipelines created after `RendererSeal`; zero general-purpose allocation per
  frame; O(1) lookups; all targets preallocated and warmed with one real render per
  format.
- The Dungeon holds its measured 60 Hz numbers at 1440×900 DPR 2 with GI enabled.
- **The map-agnostic test:** the identical binary and identical configuration run a
  single-room scene and an open-world scene with no setting change, no per-level
  asset, and no load-time or memory growth proportional to world size.
- Telemetry plateaus under soak: probe states, sweep time, rays/frame, resident
  count, invalid/dilated counts, atlas bytes.
- Leaks assessed against the artifact table in §8; a probe inside geometry goes off
  rather than leaking.

### Not in this design

ADGI-style adaptive sampling; hash-grid/sparse caches; glossy or reflection reuse of
 the irradiance atlas; screen-space GI companions (they need a G-buffer); any bake,
 cook, or `.big` probe path. These are not later stages — they are absent.

### Still unmeasured

The traversal rate is real, but the loop around it has never run on this GPU: the
**octahedral atlas accumulation, border updates, the ping-pong blend, in-material
8-probe cage sampling, and the end-to-end frame**. Those are the first things to
measure after it runs.

## 11. Decisions required from the user

These are product/architecture decisions, not technical questions a prototype
can settle:

1. **Ambient/indirect identity.** Does the engine get a first-class
   `EngineIndirectLighting` resource (probes + sky + ambient as one thing), or do
   probes remain a demo-owned object? Recommended: a first-class resource, since
   every demo currently hand-rolls its own lights.
2. **Dynamic GI requirement.** Is dynamic indirect lighting (moving sun,
   destructible/dynamic walls) a product requirement, or is baked probes plus a
   dynamic sky term (APV sky occlusion) sufficient? This is the decision that
   gates Stage 2 entirely.
3. **Frame-rate tier.** Is the Dungeon a 60 Hz feature or must it hold 120 Hz?
   At 120 Hz there is currently no headroom for a GI pass on the 680M.
4. **Probe placement policy.** Uniform grid (simple, predictable capacity, needs
   relocation), geometry-density automatic placement (better quality per probe,
   needs a cook-side density algorithm), or manual placement (artist control,
   worst ergonomics)? Recommended: uniform grid with cook-time relocation for
   Stage 1.
5. **Specular/reflection scope.** Diffuse irradiance only, or GGX reflection
   probes as well? Reflection probes imply per-probe cubemap rendering and a
   much larger memory footprint.
6. **Reference quality bar.** Should the pipeline ship a path-traced reference
   mode for validation images, and does it need to be reproducible across
   machines for goldens?
7. **Low-end floor.** Must GI be available with GI *disabled* on the weakest
   tier, i.e. is a documented no-GI fallback an acceptable shipped state?

## 12. Sources

| Source | What it establishes here |
|---|---|
| [DDGI, JCGT 8(2) 2019](https://www.jcgt.org/published/0008/02/01/) | Octahedral irradiance/depth representation and resolutions, hysteresis, amortization policy, ray counts, throughput and per-pass timings on RTX 2080 Ti, stated limitations |
| [Scaling Probe-Based Real-Time DDGI for Production, JCGT 10(2) 2021](https://jcgt.org/published/0010/02/01/) | Self-shadow bias equation and default, gamma-5 perception encoding, fast-convergence heuristics, probe state machine, relocation rules, cascades and volume blending, 30–50% sleeping win, production limitations |
| [RTXGI-DDGI `Algorithms.md`](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/Algorithms.md) | Canonical DDGI description, benefits/limitations, complementarity with ray-traced AO, and the claim that probes can be precomputed for platforms without hardware ray tracing |
| [RTXGI-DDGI `DDGIVolume.md`](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/DDGIVolume.md) | Texture-array layout, 1-texel border and interior/border fields, gamma 5 default and format trade-off, probe relocation and classification CS entry points, probe count limits, probe density guidance (2–3 m, sparse preferred), `probeViewBias` scale dependence, scrolling wrap condition |
| [SHARC `Integration.md`](https://github.com/NVIDIA-RTX/SHARC/blob/main/docs/Integration.md) | Sparse radiance-cache pass structure (update/resolve/render) and its relationship to probe-based caching in RTXGI 2.0 |
| [Lumen technical details (UE 5.8)](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine) and the [Lumen performance guide](https://dev.epicgames.com/documentation/unreal-engine/lumen-performance-guide-for-unreal-engine) | AAA radiance-cache architecture: world-space and screen-space radiance caches, final gather, and the "Irradiance Field Gather" probe subset |
| [Unity Adaptive Probe Volumes](https://docs.unity3d.com/6000.0/Documentation/Manual/urp/probevolumes-concept.html) | Shipping baked-probe design: automatic density-based placement, per-pixel probe sampling, streaming for large worlds, sky occlusion, scenario blending, probe-position limitation |
| [Orillusion GI guide](https://www.orillusion.com/en/guide/advanced/gi.html) | WebGPU DDGI prior art: probe-grid component, compute-based irradiance/G-buffer generation, octahedral sphere-to-square mapping, the need to disable probe updates for static scenes, exposed bias parameters |
| [Ramamoorthi & Hanrahan 2001](https://graphics.stanford.edu/papers/envmap/envmap.pdf) | The SH irradiance derivation underlying probe storage order and accuracy |
| [Cigolle et al., JCGT 3(2) 2014](https://jcgt.org/published/0003/02/01/) | Octahedral parameterization used by both DDGI and RTXGI |
| This repository | `docs/api/steam-audio.md` (measured tracer rates), `docs/api/virtual-texturing.md` (Dungeon GPU timings), `virtual-texture-feedback-pass.ts` (extra-pass pattern), `big-format.ts` (`Raw` chunk), the pinned Three build (`IrradianceNode`, absent `gi()`) |

### What the sources do not establish

- **No source measures a WGSL/compute BVH traversal on an iGPU.** §7.3 is now a repository measurement, but of a binary-BVH proxy harness — not a CWBVH8 traversal and not a full DDGI update loop. Treat it as an upper bound on traversal cost with the caveats listed there. It supersedes the earlier estimate in this document.
- RTXGI's documentation gives D3D12/Vulkan limits (16,384 texels, 2,048 array
  slices). WebGPU's default `maxTextureArrayLayers` is 256; the effective limit
  on the 680M adapter has not been read back here.
- The DDGI papers report no mobile/integrated-GPU results, so their throughput
  tables cannot be extrapolated downward.
- Unity's APV documentation does not specify SH order or per-probe byte layout;
  the L1/L2 byte figures in §7.5 are this document's arithmetic, not a source
  claim.
- No source quantifies the quality cost of the 8-level amortization assumed in
  §7.3; that interaction (hysteresis × update interval) needs a prototype
  measurement.
