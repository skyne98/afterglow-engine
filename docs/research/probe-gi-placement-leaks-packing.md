# Probe GI — adaptive placement, leak suppression, and cache-efficient packing

> Status: research complete, no implementation. Investigated 2026-09-17.
> Companion to `docs/research/probe-based-gi.md`, which covers the technique
> survey, DDGI's base algorithm, and the measured GPU-traversal result that makes
> dynamic probe GI viable on this hardware.
>
> This document answers three follow-ups:
> 1. dynamic, smart, non-uniform probe placement (best quality per unit cost);
> 2. light leaks, thin walls, and the rest of the occlusion failure modes;
> 3. the most cache-efficient kernels and data packing methods.
>
> Method: primary sources only — the ADGI paper (HPG 2022 poster), IS-DDGI
> (PACMCGIT/I3D 2023), DDGI Resampling (SIGGRAPH 2021 talk / CGF 41(6), 2022),
> the NVIDIA CWBVH paper (HPG 2017), the RTXGI-DDGI SDK documentation, the SHARC
> integration guide, Unity's Adaptive Probe Volumes documentation and its actual
> `ProbeGIBaking` / `ProbeVolume` C# source, and the W3C WebGPU specification's
> texture-format capability tables. Where a number comes from this repository or
> from a measurement made for this document, it says so. Everything else is
> labelled as inference.

## 0. Bottom line

**Placement:** do not start with adaptive placement. Start with a **dense uniform
volume plus the RTXGI probe state machine**, then add **importance-weighted ray
allocation** (IS-DDGI) before adding **adaptive spatial sampling** (ADGI). The
papers' own results show why this ordering is right: sleeping/deactivated probes
buy 30–50% for near-zero complexity, and importance-based ray allocation buys
another 1.27–2.47× with no change to probe positions. ADGI's full machinery buys
1.5–2× *on top of* a scene that already has a uniform grid — but it replaces the
deterministic per-probe index with a Markov chain and requires an irradiance
cache, a visibility cache, a guide LUT, and two extra quantized caches. That is
the last step, not the first.

**Leaks:** with no bake (see `docs/research/probe-based-gi.md` §0), the
"cheap deterministic bake-time wins" are gone. Every leak mechanism now has to be
**a bootstrap init pass or a runtime pass**: validity tracing, virtual offset,
relocation, and dilation become one-time GPU work before `GameplaySealed`; the
self-shadow bias, backface handling, and Chebyshev weighting stay per-frame. The
consequence is that thin-wall correctness becomes a **level/authoring constraint
validated at startup** (probe spacing versus minimum wall thickness), because
nothing offline can subdivide or dilate for you.

**Packing:** the single most important discovery for this engine is that the
exact CWBVH8 layout it will traverse in WGSL is already sitting in its own
dependency tree (`obvhs` 0.3.2, `cwbvh/node.rs`) — an 80-byte node with 1-byte
child planes and an octant-invariant traversal. Do not design a new traversal.
Second: WebGPU's format rules force specific choices that the DDGI sources assume
away (§C.5), notably **no in-place read-write storage** on this adapter, which
turns the temporal blend into a ping-pong or render-attachment problem.

## Part A — Dynamic, smart, non-uniform probe placement

### A.1 The problem with uniform grids

A uniform grid spends the same resources on every probe: the same rays, the same
update cadence, the same storage. DDGI's own production paper:

> "In any scene with significant open space, even after adjustment, many probes
> in a uniform 3D grid will not contribute to the final image."

and its stated limitation, restated by ADGI:

> DDGI's "uniform allocation policy dilutes resource concentration in critical
> regions, especially when a large number of probes are present."

Four independent levers exist for fixing that, and they compose:

| Lever | What it changes | Cost/benefit (source-reported) |
|---|---|---|
| Probe states (deactivate/sleep) | Which probes are traced at all | **30–50%** average improvement |
| Relocation + classification | Where probes sit and how they are weighted | Quality; frees probes stuck in geometry |
| Ray allocation | How many rays each awake probe gets | **1.27–2.47×** total, **3.29–6.64×** tracing |
| Adaptive spatial sampling | Which probes exist / are refreshed | **1.5–2×**, enables 10× probe counts |
| Cascades + streaming | Which probes are resident / at what resolution | Constant cost in large worlds |

### A.2 Probe states — the cheapest win (RTXGI)

Covered in detail in the companion document; restated here as the first rung:

- **Deactivated** — probes that remain inside *static* geometry: never traced,
  never updated. Dynamic geometry does not deactivate a probe, because a probe
  that later emerges must turn on correctly.
- **Sleeping / Awake** — a probe must be awake *if and only if it is shading a
  surface or about to*. Camera proximity is explicitly **not** a reason to wake a
  probe.
- **Vigilant** — probes that shade static geometry always trace, because they
  propagate 2nd–nth order diffuse light.
- **Newly Vigilant / Newly Awake** — hysteresis is reduced for probes that just
  appeared (scene start or a scrolling volume), so they converge fast. New
  probes trace for five frames to decide position and state.

Measured effect: 30–50% average performance improvement, and the freed budget is
normally spent on *more rays per active probe*, which lowers safe hysteresis and
therefore converges faster.

### A.3 Importance-based ray allocation (IS-DDGI, I3D 2023)

Liu, Huang, Rocha, Malmros, Zhang (Huawei), *Importance-Based Ray Strategies for
Dynamic Diffuse Global Illumination*, PACMCGIT 6(1), 2023.

- First ray-allocation technique for DDGI, built on **multiple importance
  sampling**: "a set of importance-based ray strategies that analyze, allocate,
  and manage ray resources on the GPU".
- Combines those strategies with **adaptive historical and temporal
  frame-to-frame analysis** for information reuse, plus GPU-side optimizations
  that speed up allocation and reduce memory bandwidth.
- **Reported: similar visual quality to DDGI at 1.27–2.47× total DDGI time,
  and 3.29–6.64× in probe ray tracing time.** "Most speedup of IS-DDGI comes from
  probes ray tracing speedup."

Why this matters here: it changes **only the dispatch**, not the data layout or
the probe grid. It is therefore the lowest-risk second step — no index scheme
change, no new cache — and it directly attacks the measured dominant cost
(traversal, §C.6).

### A.4 Adaptive spatial sampling (ADGI, HPG 2022)

Datta, Goli, Zhang (McGill / Huawei-AMD), *Adaptive Dynamic Global Illumination*,
High Performance Graphics 2022 (poster).

The most complete published answer to "place effort where it matters". Structure:

**Step 1 — 8 pilot rays per probe**, one per octant. Each returns hit distance and
incoming irradiance. These populate a per-probe, per-octant **guide LUT** "such
that each probe has eight texels corresponding to an octant".

**Step 2 — five heuristics** evaluated from those pilot rays:

| Heuristic | Definition (verbatim from the paper) |
|---|---|
| Distance from camera `f_c` | `1` if `‖p−c‖ < k`, else `e^−(‖p−c‖−k)` |
| Probe visibility `f_v` | `e^−2t/s`, where `t = trace(p, −ω)` is the nearest surface distance and `s` the grid-voxel diagonal |
| Incoming radiance `f_r` | `min(r, β)/β`, where `r = lum(p, ω)` |
| Visibility change `f_Δv` | temporal gradient `f_v^t − f_v^{t−1}` → temporal trigger `T_r(·, θ)` (pulse → decaying signal) → **5×5×5 spatial and 3×3 directional convolution** |
| Radiance change `f_Δr` | same pipeline applied to probe radiance |

The paper keeps position and direction fixed when differencing across frames "to
avoid noisy gradients", and the convolutions "smooth out uncertainties in a single
texel" while acting "as a weak predictor of possible locations of the dynamic
geometry in the next frame".

**Step 3 — composition**:

```
f_s(p, ω) = f_c · f_v                                  (static)
f_d(p, ω) = f_c · f_v · (f_Δv + μ · f_Δr)              (dynamic),  μ = 2
```

The static term modulates the dynamic term, i.e. changes matter most **near the
camera and near surfaces**.

**Step 4 — sample the guide with a temporally coherent Markov chain**
(Metropolis). Each chain instance has its own state memory; the chain is
initialized from the previous frame's state (`x_0 ← S[j]`, `S[j] ← x_{i+1}`), which
is what makes it temporally coherent and parallel across threads.

**Step 5 — trace the adaptive samples, then update caches** with a
moving-average accumulation using an explicit sample count `N ← min(n+1, N_max)`:
early samples dominate (fast response, noisy), later samples converge.

Reported results:

- Bistro Exterior with **192 × 64 × 192 = 2,359,296 probes** at **15.6 ms** vs
  Q-DDGI at **26.8 ms**; other scenes 9.86 vs 13.2 ms and 19.7 vs 31.6 ms.
- **1.5–2× performance improvement**, "an order of magnitude increase in probe
  count", "reduces temporal lag and minimizes reliance on temporal blur".
- "Our adaptive sampling stages have a fixed upper bound on the compute
  requirement and also decouples sampling from the number of probes."
- **4× memory reduction** from the encoding scheme (§C.4).
- GBuffer and direct illumination are separate: **+2 ms and +3 ms** in their
  measurement, on top of the 15.6 ms.
- A separate experiment is limited to a 32×32×32 grid "due to memory
  constraints" — worth noting, because it shows the memory ceiling bites before
  the compute ceiling does.

**Engine-fit caveats** (this repository's rules, not the paper's problem): a
Markov chain per probe with per-chain state is not obviously compatible with
"fixed-capacity, allocation-free, O(1) hot paths" — but it can be, if the chain
state is a fixed per-probe slot written in place (which is what `S[j]` is). The
5×5×5 × 3×3 convolution is `5³ × 3² = 1,125` taps per probe per frame for each
of the two change heuristics — that is the part to budget explicitly, and the
part to make incremental (a rotating slab per frame) rather than full-volume.

### A.5 Sparse world-space caches and reservoirs (DDGI Resampling)

Majercik, Müller, Keller, Nowrouzezahrai, McGuire, *Dynamic Diffuse Global
Illumination Resampling*, SIGGRAPH 2021 Talks / CGF 41(6), 2022.

- Derives a unified sampler from the principles of **ReSTIR** many-light direct
  shadowing: "we apply ReSTIR to global illumination by targeting" the same
  estimator, combining **screen-space reservoir resampling** with **sparse
  world-space probes**.
- Querying DDGI at secondary path vertices "employ[s] a similar strategy to final
  gathering to conceal DDGI's limitations (e.g., coarse spatial discretization)
  behind the blurring effect of the primary scattering interaction", with the
  resulting noise removed by spatio-temporal reservoir sampling.
- Reported timings from the paper's figure: pure path tracing **22.5 ms**;
  + secondary DDGI **12.8 ms**; + DDGI resampling **18.4 ms**; + denoising
  **26.6 ms**. The claim is at **equal time and equal quality** versus both
  "purely path traced and purely probe-based baselines".

Two lessons for this engine: (1) **sparse** world-space probes are viable when a
reservoir-based reuse scheme carries the missing detail — probes need not cover
every voxel; (2) the technique composes with a separate denoiser, i.e. it does not
have to solve noise alone.

### A.6 Hash-grid caches with explicit eviction (SHARC)

NVIDIA-RTX/SHARC, integration guide. Directly relevant as the "no fixed grid at
all" end of the spectrum:

- Layout: `Hash entries` (8-byte entries storing hashes), `accumulation`, and
  `resolved data` buffers, each with the same element count. "A solid baseline
  for most scenes can be the usage of 2²² elements" (~4.19 M), power-of-two
  recommended; more elements for high depth complexity, fewer to reduce memory
  pressure "but increases the risk of hash collisions".
- The **Resolve** pass "combines per-frame accumulation data with previously
  resolved data, performs temporal accumulation, **handles stale entry
  eviction**, and resets accumulation data for the next frame."
- Occupancy guidance: "with a static camera around 10–20% of elements should be
  used on average"; "increased occupancy can negatively impact performance, to
  control that we can increase the element count as well as decrease the
  threshold for the stale frames to evict outdated elements more aggressively."
- Memory: **40 bytes per voxel** by default (8 hash + 16 accumulation + 16
  resolved), **64 bytes** with SH encoding (8 + 32 + 24). At 2²² elements that is
  **160 MiB** default, **256 MiB** with directional radiance — a useful scale
  reference for what a sparse cache costs versus a dense probe volume.

**Engine fit: excellent.** A bounded element pool with eviction and a hash lookup
maps onto the engine's existing generational-handle arena + O(1) hash lookup
pattern far more directly than an MCMC guide does. If probe GI ever needs to
cover a world much larger than one volume, this is the shape to copy.

### A.7 Baked adaptive placement (Unity APV) — the shipping reference

- Probes are **placed automatically based on geometry density**, so interiors get
  denser probes with no manual placement; the density/region is configured via
  "Adaptive Probe Volumes" settings rather than per-probe.
- Probes live in **bricks with multiple subdivision levels** — i.e. the grid is
  hierarchical and locally refined, which is how it resolves detail without a
  globally dense grid.
- **Chunked streaming** lets datasets larger than GPU memory page in at runtime.
- **Probe Adjustment Volumes** override the baked data in a region; **Rendering
  Layers** prevent objects from sampling probes on another layer mask.
- Stated limitation: **you cannot move individual probes** — placement is a bake
  setting, not an authored attribute.

### A.8 What this engine's placement actually is

> The delivery shape is **one shot, then interactive iteration** — there are no
> stages. The list below is therefore not an ordering of work; it is the feature
> set, with the sources' leverage numbers attached so the constants can be tuned
> sensibly once it runs.

**In the design:**

1. **Camera-locked scrolling cascades** (×3 spacing per cascade), not an authored
   volume. This is what makes the system map-agnostic: an open world scrolls, a
   small room occupies the innermost cascade, and resident capacity is constant
   either way. The DDGI production paper's multivolume scheme packs all cascades
   into one shared atlas, blends densest-first until accumulated weight reaches
   1.0, falls off linearly over the last grid cell, and documents the scroll-wrap
   clear condition.
2. **Probe state machine** — deactivated (inside static geometry), sleeping/awake
   (only if shading or about to), vigilant (shades static geometry, always traces),
   with a five-frame init for new and scrolled-in probes. Sources report **30–50%**
   for this alone, which is why it is in the system from the start rather than
   treated as an optimization.
3. **Runtime relocation within the voxel** — never around dynamic geometry (stable
   beats lower average error), and never left as a per-level authoring task.
4. **Per-frame full-field sweep** at the reference field size, because the measured
   traversal rate makes it affordable (49,152 rays ≈ 0.16–0.34 ms). Amortization
   is a **knob**, not a stage: it exists for larger fields and weaker GPUs.

**Available as a bounded knob if measured necessary:**

- **Importance-weighted ray allocation** (IS-DDGI: **1.27–2.47×** total,
  **3.29–6.64×** tracing) — changes only how the frame's ray budget is distributed
  across awake probes, so it is a dispatch change, not a data-layout change.
- **Cascade count and spacing multipliers** — the primary lever for world extent.

**Not in the design:**

- **ADGI-style adaptive sampling** (guide LUT, MCMC, per-probe irradiance and
  visibility caches). Its 1.5–2× is real and its 2.36 M-probe result is
  impressive, but it requires the two change heuristics to run 5³ × 3² = 1,125-tap
  convolutions per probe per frame, and it replaces a deterministic index with a
  Markov chain. Cascades plus a state machine cover world extent, which is the
  problem that actually exists here.
- **SHARC-style hash grids.** 2²² elements at 40–64 B/voxel is 160–256 MiB; the
  camera-locked cascade field costs ~3 MB of atlas. Revisit only if a bounded
  resident field provably cannot cover the map.
- **Sparse world-space probes with ReSTIR reservoirs** (DDGI Resampling). This is
  a path-traced pipeline's answer to GI noise; there is no path tracer here.

## Part B — Light leaks, thin walls, and occlusion failures

### B.1 Five distinct causes, five distinct fixes

Leaks are usually reported as one bug but they come from five places:

| # | Cause | Where it shows | Primary fix |
|---|---|---|---|
| 1 | Probe sits inside geometry (wrong sample point) | Dark blotches / bands, or light where it must not be | Relocation (DDGI), Virtual Offset (Unity), deactivation |
| 2 | Probe is valid but *behind a wall* from the receiver | Light bleeding through a wall corner | Visibility weighting (Chebyshev + depth moments), bias terms, layer masks |
| 3 | Two valid probes on opposite sides of a thin wall interpolate across it | Smooth gradient through a solid wall | Brick subdivision below wall thickness; per-probe validity; probe occlusion |
| 4 | Visibility test bias too small (acne) or too large (leak) | Acne or bleed on the same surface | Self-shadow bias / `probeViewBias`, tuned per scene scale |
| 5 | Temporal accumulation keeps stale lighting | Smear after a light moves | Hysteresis reduction for changed probes; gamma-5 encoding; convergence heuristics |

### B.2 DDGI's runtime toolkit (exact parameters)

From the two DDGI papers and the RTXGI docs:

- **Self-shadow bias** — one tunable parameter replacing three:
  ```
  BiasVector = (n · 0.2 + ω_o · 0.8) · (0.75 · minDistanceBetweenProbes) · TunableShadowBias
  ```
  default `TunableShadowBias = 0.3`; add it to the sample point **for the
  visibility test only**. Higher values are needed when depth variance is higher,
  e.g. at lower ray counts. The paper notes a single tunable is easier to fit than
  the previous per-scene bias triple.
- **View bias** (`probeViewBias`) — a **world-space** offset along the camera view
  ray, pushing the shaded point deeper into the probe's voxel where
  mean-distance variance is lower. RTXGI warns explicitly that because it is
  world-space, "**the SDK's default value likely won't be what your content
  requires**" — it must be derived from scene scale.
- **Backface handling** — rays that hit backfaces "record a value of 0 for
  irradiance and shorten their depth values by 80%", so the probe sees backfaces
  as shadowed rather than lit. This is the mechanism that keeps a probe inside a
  wall from lighting the room.
- **Chebyshev visibility weighting** — mean- and variance-biased Chebyshev
  interpolants over the stored distance moments (mean distance + mean squared
  distance).
- **Zero-thickness planes are not valid walls.** RTXGI states it outright: if
  leaking occurs "and zero thickness planes are not used for walls", the fix is
  `probeViewBias`. Conversely, a zero-thickness wall gives the probe no thickness
  to reason about.
- **Relocation** within the grid voxel (init/bake time only, never around dynamic
  geometry), then **deactivation** if still inside static geometry.
- **Complement with ray-traced AO.** DDGI is a low-frequency signal by
  construction; RTXGI pairs it with RTAO rather than trying to make probes carry
  high-frequency occlusion.

**What the reference demo measured against these rules** (§7.6 of
`probe-based-gi.md`). Three of them are implemented and two were wrong in ways worth
recording:

- The self-shadow bias and the ray-origin bias are **world-space metres**, not
  fractions of the cascade spacing. The spacing-scaled form gave the 9 m cascade a
  **2.0 m** offset — a teleport through any thin wall, and (measured) open sky
  reported from inside a sealed room.
- Backface handling is implemented as documented (zero irradiance, depth × 0.2) and
  confines probes inside walls correctly — but the **validity test** that pairs with it must
  not carry a distance limit. `probeInit` counted a backface only when its hit was closer
  than `spacing × 0.5`, so a probe half a metre inside a 1 m slab (its surface exactly at the
  limit) stayed *valid* with an all-backface sweep and E = 0: 64 of the near cascade's 256
  probes sat in the ground layer doing nothing but voting zero into every cage near the
  floor. Any backface hit means the ray left a solid, so the limit is gone; front faces are
  untouched, so a probe merely near a wall is unaffected.
- **Cascade shares must sum to 1.** Each cascade's own boundary fade is *not* a share: a
  coarse cascade is at fade ≈ 1 for a point deep inside the fine cascade's box, so it
  outvotes the fine one inside that box (measured: 74 % of a floor point one metre from the
  camera, with the 9 m cascade at E = 1.27 against the honest in-room 0.08). Weight by
  `fade × Π(1 − fade_finer)` instead — that is what RTXGI's "blend densest-first, stop at
  total weight 1.0" means in practice — and the camera-locked brightness step goes with it.
- **Hysteresis is for the whole field**, not for the probes that happen to have a seed. The
  seed test replaced the freshness test when seeding landed, and because a seed exists on
  exactly one frame the field ended up with `h = 0` everywhere: no accumulation at all, and
  a wrapped band that snapped to its own value one frame after inheriting its neighbour.
- A reseeded band must **inherit a neighbour** rather than start from no history: the
  JCGT 2021 "fast-convergence heuristics" exist for this, and reseeding a scroll band with
  `hysteresis = 0` next to probes averaging ~14 frames is visible as a hard step on every
  lit surface while the light moves. Seed the fresh probe from the inward neighbour (one
  cell back along the scrolled axis) and use the normal hysteresis.
- Relocation must be **bounded by the cell**. `max(farthest, …) × 0.6` is unbounded
  because `farthest` is `reach × 10` for any missing ray; it moved probes up to 24 m
  in the simple scene. Bounded to one cell, relocation reports 0 in both scenes.
- Backface handling is implemented as documented (zero irradiance, depth × 0.2) and
  confines probes inside walls correctly.
- Chebyshev **cubing** and the sub-0.2 **weight sharpening** are documented leak
  mitigations that measured no benefit in this engine (sealing the room's doorway and
  toggling Chebyshev moved the leaking wall by 1 %) while tiling dim surfaces with the
  probe grid — a query near a cage-cell boundary is up to 1.5 spacings from its probes,
  their distance terms dip, and cubing the dip twice over turns it into hard blocks.
  Apply the gate plain unless a measurement says otherwise.
- The measured leak at a ~10:1 geometry-to-spacing ratio is *cage interpolation*, not
  probe content: in a sealed room the interior probes carry exactly zero false light
  and the leak is entirely outside probes mixed in across the roof (76 %) and the
  walls (72 %). Chebyshev weighting, cubing and weight sharpening reduce it but cannot
  remove it, because the gate is evaluated per probe in a *direction*, not per
  occluder. RTAO removes it only with enough rays to avoid visible speckle. A
  query→probe moment fetch is geometrically right and was rejected for grain in this
  engine — but that measurement predates the atlas-fetch mapping fix, and re-measured
  afterwards the high-frequency residual is identical (55.1/59.8/75.2 along the normal
  against 52.6/57.1/76.1 along query→probe) while the sealed-room leak moves 80.5 → 78.2.
  It is back in, and the small size of the win is explained: the distance map is 8×8 and
  *bilinearly interpolated*, so the moments a per-query direction would sharpen are smeared
  before the shader sees them. This is the case Unity's Sky Occlusion exists for and the one
  RTXGI pairs with RTAO.

### B.3 Unity APV's toolkit (the baked-side reference)

Unity's own definition: "Light Probes inside geometry are called **invalid**
probes… HDRP marks a Light Probe as invalid when the probe fires sampling rays to
capture surrounding light data, but the rays hit the unlit backfaces inside
geometry."

- **Virtual Offset** — "tries to make invalid Light Probes valid, by moving their
  capture points so they're outside any colliders." The offset is derived from the
  sampling ray that escapes the geometry; the sampling-ray length is a configurable
  setting ("the length of the sampling ray Unity uses to find a valid capture
  point"). Documented symptom of *not* doing it: "dark bands".
- **Dilation** — "detects Light Probes that remain invalid after Virtual Offset,
  and gives them data from valid Light Probes nearby." Implemented as a compute
  kernel (`FindKernel("DilateCell")`) operating per probe, indexing
  `chunkIndex = globalProbeIndex / chunkSizeInProbeCount`.
- **Probe Occlusion** — a per-probe `Vector4` carried in the baked probe struct,
  i.e. per-light visibility resolved at bake time.
- **Rendering Layers** — "prevent objects from sampling probes that are on another
  Layer Mask, reducing light leaking in certain scenarios."
- **Brick subdivision** — the practical thin-wall fix: "raise the max subdivision
  so the smallest bricks resolve thin walls instead of bridging them with one
  large cell."
- **Leak-reduction modes** — Unity ships explicit "Performance" and "Quality"
  light-leaking-prevention modes plus **Probe Adjustment Volumes**, i.e. leak
  policy is a first-class, user-visible product decision.
- **Sky occlusion** — stored as L0L1 coefficients plus a direction vector, so
  changing sky light re-lights interiors without a rebake.

### B.4 The per-probe bake payload Unity settles on

From `ProbeGIBaking.Dilate.cs`, the baked probe struct is exactly:

```csharp
struct DilatedProbe {
    Vector3 L0;                                    // SH L2 = 9 × Vector3
    Vector3 L1_0, L1_1, L1_2;
    Vector3 L2_0, L2_1, L2_2, L2_3, L2_4;
    Vector4 SO_L0L1;                               // sky occlusion
    Vector3 SO_Direction;
    Vector4 ProbeOcclusion;                        // per-probe light visibility
}
```

That is 108 B of SH + 16 B sky-occlusion L0L1 + 12 B direction + 16 B probe
occlusion ≈ **152 B per probe**, and it is the reference answer for "what does a
baked probe need besides radiance to suppress leaks": **occlusion and sky
visibility**. Keep that in mind when sizing this engine's cooked probe payload.

### B.5 Recommended leak plan for this engine (no bake)

- **Bootstrap init pass (once, before `GameplaySealed`, telemetried):** probe
  validity test (sampling rays that hit backfaces), **virtual offset** for invalid
  probes along the escaping ray, **relocation** within the grid voxel,
  **deactivation** for probes still inside static geometry, and **dilation** of
  still-invalid probes from valid neighbours. This is exactly Unity's bake-time
  sequence moved into a warm-up compute pass; the BLAS and the probe grid are both
  available at this point, so no offline data is required.
- **No level-side requirement.** Report probe-state counts (valid / invalid /
  deactivated / relocated / dilated) as telemetry, but never as a level-authoring
  gate: with a global spacing constant some geometry is always finer than the grid,
  so the system's contract is **fail closed** — a probe inside a wall deactivates
  and that region gets no probe GI. Turning off is correct; leaking is not.
  Subdivision-per-brick would be the way to *recover* those regions, and it is not
  in this design (see A.8).
- **Runtime (bounded, per-frame):** self-shadow bias with a scene-scale-derived
  constant, backface → irradiance 0 + depth −80%, Chebyshev mean/variance bias,
  hysteresis reduction on changed probes.
- **Policy:** ship the equivalent of Unity's two leak modes (performance =
  cheaper bias/visibility, quality = more conservative) as an explicit setting
  rather than tuning one set of constants for every scene. `probeViewBias` must
  be derived from the level's scale, never hard-coded.
- **What is genuinely lost without a bake:** no per-probe light visibility
  (`ProbeOcclusion`) and no sky-visibility channel, since both are precomputed per
  light. Dynamic lights do not need them; *static* lights would, and without a
  bake those must either be traced like dynamic ones or accept the probe's
  visibility approximation.

## Part C — Cache-efficient kernels and data packing

### C.1 The traversal kernel is already designed — use it

This engine's `afterglow-obvhs-tracer` depends on `obvhs` 0.3.2, whose CWBVH
implementation is the HPG 2017 compressed wide BVH8 (Ylitie, Karras, Laine). The
node layout is fixed and documented in that crate:

```rust
/// A Compressed Wide BVH8 Node. repr(C), Pod, 80 bytes.
pub struct CwBvhNode {
    pub p: Vec3,                  // 12  min point of node AABB
    pub e: [u8; 3],               //  3  shared exponent per axis (octant quantization)
    pub imask: u8,                //  1  which children are internal
    pub child_base_idx: u32,      //  4
    pub primitive_base_idx: u32,  //  4
    pub child_meta: [u8; 8],      //  8  per-child offsets/counts (bit-packed)
    pub child_min_x: [u8; 8],     //  8
    pub child_max_x: [u8; 8],     //  8
    pub child_min_y: [u8; 8],     //  8
    pub child_max_y: [u8; 8],     //  8
    pub child_min_z: [u8; 8],     //  8
    pub child_max_z: [u8; 8],     //  8
}                                 // = 80 bytes
```

Bit-level encoding of `child_meta` (verbatim semantics from the crate):

- empty slot → `0`;
- **leaf**: low 5 bits = primitive offset `[0..24)` from `primitive_base_idx`;
  high 3 bits = primitive count in **unary** encoding (e.g. 2 primitives starting
  at base → `0b01100000`; 3 primitives starting at base+2 → `0b11100010`);
- **internal**: high 3 bits = `001`, low 5 bits = child slot index + 24 (range
  `[24..32)`).

Two deliberate deviations from the paper are recorded in the source: min/max are
**interleaved** here, and the implementation **transforms the ray** rather than
computing plane positions (`p.x + bitcast<f32>(e[0] << 23) * child_min_x[0]`).

Other cache-relevant properties of this implementation, read from the source:

- **Octant-invariant traversal**: `oct_inv4 = ray_get_octant_inv4(ray.direction)`
  is computed once per ray and used by `node.get_child_and_index_bits(oct_inv4)`,
  so child ordering needs no per-node sorting — that is the paper's cheap
  octant-aware fixed-order traversal.
- **Compressed stack**: `StackStack<UVec2, TRAVERSAL_STACK_SIZE>` with
  `TRAVERSAL_STACK_SIZE = 32`, and the source's own justification: "BVH8's tend to
  be shallow. A stack of 32 would be very deep even for a large scene with no
  TLAS." Entries are `uvec2` (8 bytes) rather than one full node per level.
- A `current_group: uvec2(0, 0x80000000)` field drives group-wise processing
  (the paper's combination of *triangle postponing* and *dynamic ray fetching* to
  keep SIMD lanes busy).

Paper-reported benefit: **1.9–2.1× improvement in incoherent traversal
performance** versus previous wide-BVH kernels, attributed to reduced memory
traffic plus the compressed stack.

Implementation guidance for the WGSL port: allocate nodes as a storage buffer of
this exact 80-byte struct **in the same field order**, so the Rust builder and the
WGSL traversal agree without a repacking pass; keep the stack at 32 entries of
`vec2<u32>`; compute the octant table once per ray. A WGSL port must be validated
against the existing CPU `intersect_node` behaviour before it is trusted.

Measured evidence that this matters less than expected on this hardware: in this
repository's benchmark, doubling triangles from 24k to 96k changed 524k-ray time
from 1.68 ms to 1.96 ms — a ~15% effect. Node *fetch width* is the lever, not
scene complexity, which is exactly what a wide BVH optimizes.

### C.2 Probe update kernel organization (RTXGI's shipping choices)

The probe ray data layout, quoted from the RTXGI docs:

- The **Probe Ray Data texture array** is "composed of `slices` that correspond to
  planes of probes oriented perpendicular to the coordinate system's up axis."
- "A texture array slice `row` represents a single probe within the horizontal
  plane of probes. The row number is the probe's index *within that slice*."
- "A texture array slice `column` represents rays traced from probes. Column
  number is the ray's index."
- "Each `texel` contains the incoming radiance from and distance to the closest
  surface obtained by ray `column#`."

Why it is cache-efficient: every probe's rays are **contiguous in one row**, so a
workgroup that owns a row reads a coalesced slab and can stage it in shared
memory; neighbouring probes in the same plane sit in adjacent rows, so a warp
processing several probes touches an almost-contiguous region.

Kernel-level settings the SDK exposes are all bandwidth/occupancy trade-offs:

| Define | Effect (from the docs) |
|---|---|
| `RTXGI_DDGI_BLEND_SHARED_MEMORY` | "cache ray radiance and distance values" in shared memory; "can substantially improve performance at the cost of higher register and shared memory use (potentially lowering occupancy)"; requires `RAYS_PER_PROBE` to be known |
| `RTXGI_DDGI_BLEND_SCROLL_SHARED_MEMORY` | scroll-clear tests performed by the group's *first thread* and stored in shared memory "to reduce the compute workload" |
| `RTXGI_DDGI_BLEND_RADIANCE` | two separately compiled shaders — one blends radiance, one blends distance |
| `RTXGI_DDGI_BLEND_RAYS_PER_PROBE` | ray count per probe, required for the shared-memory path |

Also: "irradiance and distance texels for *all probes of a volume* are processed
in parallel, **across two overlapping dispatch calls (i.e. without serializing
barriers)**". Two dispatches that touch disjoint state avoid a barrier — the same
trick applies to a WGSL implementation, but see §C.5 on read-write storage.

### C.3 Thread mapping: ray-per-thread vs probe-per-thread

Both are used in practice, and they differ in what they reuse:

| Mapping | Reuse | Parallelism | Notes |
|---|---|---|---|
| **One thread per ray** | none across rays of a probe; probe origin/octant recomputed per thread | Highest | Natural for wide dispatches; requires an atomic or a reduction to combine per-probe sums unless you organize the workgroup so its threads are exactly one probe's rays |
| **One thread per probe** (loop rays) | probe origin, octant table, atlas addressing, and the accumulating sum live in registers for the whole probe | Lower | Fewer redundant loads; the loop is a long serial dependency chain, and divergence across probes hurts |

Practical hybrid (recommended here): **one workgroup per probe, 64 threads for 64
rays** (or 2 probes per workgroup of 128). That gives coalesced ray reads, exactly
one probe's worth of shared memory, no atomics, and a deterministic reduction. It
also matches the RTXGI row-per-probe layout. The measured benchmark used
thread-per-ray with a global invocation id and no per-probe sharing; the
hybrid is strictly better on reuse and is the thing to implement.

### C.4 Packing schemes, compared

| Scheme | Per-probe (or per-voxel) size | Precision | Notes |
|---|---:|---|---|
| DDGI paper (8×8 `RGB10A2` irradiance + 16×16 `RG16F` distance, 1-texel borders) | ≈ 1.66 KiB | 10-bit RGB, 16-bit moments | Worst memory; best-established |
| DDGI with `RGBA16F` irradiance (RTXGI default) | ≈ 2.05 KiB | half float | Simplest; no decode math |
| **ADGI heuristics LUT** | 32-bit per octant (6-10-10 bits + 6 flag bits) | mixed | 8 octants per probe |
| **ADGI irradiance cache** (8×8 octahedral) | 32-bit per texel: **9-9-8 bit RGB + 6-bit sample count** | custom non-linear compression | "atomic updates on a commodity GPU"; "minimizes memory requirements by 4×" |
| **ADGI visibility cache** | 32-bit per texel: **13-bit mean + 13-bit mean²** + 6 bits | 13-bit moments | Same atomics requirement |
| Unity baked `DilatedProbe` | ≈ 152 B per probe (SH L2 + sky occlusion + probe occlusion) | float | Bake-side reference |
| Cooked SH L2 + validity (this document's arithmetic, half precision) | ≈ 58–70 B per probe | half | Cheapest option; no visibility moments |
| SHARC voxel | 40 B (8 hash + 16 accum + 16 resolved), 64 B with SH encoding | mixed | Sparse, evictable |

**The border tax is real and easy to forget**: a probe whose interior is 8×8 texels
needs 10×10 texels allocated for the 1-texel border — **56% overhead** — and a 6×6
interior needs 8×8 = **78% overhead**. This is why RTXGI exposes both
`probeNumIrradianceTexels` (including border) and `probeNumIrradianceInteriorTexels`
separately, and why the irradiance/distance resolutions (8×8 vs 16×16 in the paper;
typically 6×6/8×8 for irradiance in practice) matter more than they look.

### C.5 WebGPU's hard constraints (verified against the W3C spec)

These invalidate several assumptions the DDGI sources make, so they must be
designed around rather than discovered later.

| Constraint | Consequence |
|---|---|
| `rgb10a2unorm` supports **storage access only with the optional `texture-formats-tier1`** feature (it is render-attachment-capable in core) | The DDGI paper's irradiance format is unavailable to a compute-written atlas on this adapter |
| **Read-write storage textures require `texture-formats-tier2`** | DDGI's temporal blend (read old value, write new) cannot be a single in-place pass; use a **ping-pong atlas pair** or **render-attachment blending** |
| The validated 680M adapter exposes **neither** `texture-formats-tier1` nor `texture-formats-tier2` (it does expose `rg11b10ufloat-renderable` independently) | Confirms both constraints above on the target hardware |
| `rgba16float` is **filterable** and supports read-only/write-only storage in core | This is the right irradiance atlas format: bilinear sampling + compute writes |
| `rgb9e5ufloat` is packed, filterable, but **neither renderable nor storage-capable** | Cannot be a compute-written atlas at all |
| `r32float` is filterable on this adapter (`float32-filterable`) and blendable (`float32-blendable`) | Usable where high precision is needed and the feature is confirmed |
| Bilinear filtering across probe boundaries requires the **1-texel border** | Atlas allocator must budget `(interior + 2)²` per probe |
| Default device limits are far below what the engine requests (this benchmark's default device: `maxStorageBufferBindingSize` 128 MiB, `maxComputeInvocationsPerWorkgroup` 256; the engine's device: 2 GiB and 1024) | Request limits explicitly, and never assume the defaults in a prototype |
| `subgroups` and `timestamp-query` are available on this adapter | Subgroup-aware traversal and real GPU timing are both on the table |

### C.6 Measured: what actually dominates

From this repository's benchmark (binary BVH proxy, 64 rays/probe, 25 dispatches
per configuration, `afterglow-shell` on the Radeon 680M):

- **Traversal dominates.** Adding a full hit-shading evaluation changed 524k-ray
  time from 1.680 ms to 1.079 ms at 24k tris and from 1.959 ms to 1.960 ms at 96k
  tris — i.e. shading is inside the noise. Time spent on *finding* hits is the
  only thing worth optimizing.
- **Scene complexity barely matters** (24k → 96k tris: +15% at 524k rays).
- **Small dispatches are overhead-bound.** In the 3-iteration run, a 131k-ray
  dispatch measured 8.277 ms; in the 25-iteration run the same configuration
  measured 1.636 ms. The work did not change — the measurement amortized a fixed
  per-dispatch/per-fence cost of several milliseconds. **Practical consequence:
  never fence per probe-update dispatch.** Submit the probe update into the
  frame's existing command buffer; the engine already owns the rAF loop, so this
  costs nothing.
- Throughput at scale: **~270–490 Mrays/s**; doubling ray count roughly doubled
  time, so the kernel scales into the range a DDGI volume needs.

Open measurement questions this document does **not** answer: the cost of the
octahedral atlas accumulation and border-update passes, the ping-pong blend pass,
in-material 8-probe cage sampling, and a full end-to-end DDGI update at 2,048 and
8,192 probes. Those are the next prototype measurements.

## Part D — Open decisions for the user

These are the constants and policies of the single implementation, not stage
choices. Everything here is a bounded integer or a small constant in one place.

1. **Cascade count and spacing multipliers.** Recommendation: 3 cascades at
   ×3 spacing (1 m / 3 m / 9 m), each 8 × 4 × 8 = 256 probes, 768 resident. This is
   the world-extent knob and the only thing that differs between scene scales.
2. **Update cadence.** Recommendation: full-field sweep every frame (49,152 rays
   ≈ 0.16–0.34 ms measured); keep amortization as the knob for larger fields or
   weaker GPUs.
3. **Atlas format and blend mechanism.** `rgba16float` ping-pong (core-safe,
   recommended) versus `rgb10a2unorm` with render-attachment blending (half the
   blend bandwidth, no optional feature required, but only usable for the blend
   pass).
4. **Resident capacity.** 768 probes ≈ 3 MB of ping-ponged atlas today. Raising
   capacity is a straight memory-for-quality trade with no per-map cost.
5. **Importance-weighted ray allocation:** build it in from the start, or hold it
   as a knob until a measurement says the uniform budget is insufficient?
   Recommendation: hold it; the reference field already fits the budget.
6. **Leak policy:** one tuned set of constants, or two shipped modes
   (performance/quality) as Unity does?
7. **Low-end tier, now that there is no bake:** fewer probes at a slower refresh,
   or no indirect light at all? There is no zero-cost static fallback any more.

## Sources

| Source | Establishes |
|---|---|
| [ADGI, HPG 2022 poster (Datta, Goli, Zhang)](https://sayan1an.github.io/pdfs/adgi.pdf) | Pilot-ray heuristics (eqs. 10–18), composition (eqs. 19–20), MCMC guide sampling, moving-average caches, 2.36 M probes at 15.6 ms vs 26.8 ms, 1.5–2× improvement, 4× memory reduction, fixed compute bound |
| [IS-DDGI, PACMCGIT 6(1) 2023 (Liu et al.)](https://allenliuzihao.github.io/IS-DDGI/) | MIS-based ray allocation; 1.27–2.47× total DDGI time, 3.29–6.64× probe ray tracing time |
| [DDGI Resampling, SIGGRAPH 2021 / CGF 41(6)](https://tom94.net/data/publications/majercik21dynamic/majercik21dynamic.pdf) | ReSTIR reservoir resampling combined with sparse world-space probes; per-pass timings; equal-time/equal-quality claim |
| [CWBVH, HPG 2017 (Ylitie, Karras, Laine)](https://research.nvidia.com/publication/2017-07_efficient-incoherent-ray-traversal-gpus-through-compressed-wide-bvhs) | 80-byte compressed 8-wide nodes, 1-byte child quantization, compressed traversal stack, octant-aware ordering, triangle postponing + dynamic ray fetching, 1.9–2.1× |
| `obvhs` 0.3.2 `cwbvh/node.rs` and `cwbvh.rs` (this engine's tracer dependency) | Exact node field layout and bit encodings, `TRAVERSAL_STACK_SIZE = 32`, `oct_inv4` traversal, documented deviations from the paper |
| [RTXGI-DDGI `DDGIVolume.md`](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/DDGIVolume.md) | Probe ray data layout (slice = plane, row = probe, column = ray), shared-memory blend/scroll-clear defines, two-dispatch no-barrier blending, probe density guidance, `probeViewBias` warning |
| [RTXGI-DDGI `Algorithms.md`](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/Algorithms.md) | DDGI limitations (low frequency, latency, memory), stated complementarity with ray-traced AO |
| [DDGI production paper, JCGT 10(2) 2021](https://jcgt.org/published/0010/02/01/) | Probe states (30–50%), relocation rules, self-shadow bias equation, gamma-5 encoding, cascade blending, probe sleeping |
| [SHARC integration guide](https://github.com/NVIDIA-RTX/SHARC/blob/main/docs/Integration.md) | Hash-grid layout, 2²² element baseline, stale-entry eviction, occupancy guidance, 40/64 bytes per voxel |
| [Unity APV — fix issues with probe volumes](https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/probevolumes-fixissues.html) | Invalid-probe definition, Virtual Offset, Dilation, Rendering Layers, dark-band symptom |
| [Unity `ProbeGIBaking.Dilate.cs`](https://github.com/Unity-Technologies/Graphics/blob/master/Packages/com.unity.render-pipelines.core/Editor/Lighting/ProbeVolume/ProbeGIBaking.Dilate.cs) | Exact baked probe payload (`DilatedProbe`): SH L2 + sky occlusion L0L1 + direction + probe occlusion; `DilateCell` kernel and chunk indexing |
| [Unity APV concept docs](https://docs.unity3d.com/6000.0/Documentation/Manual/urp/probevolumes-concept.html) | Density-based placement, bricks/subdivision, streaming, per-pixel sampling, probe-position limitation |
| [W3C WebGPU specification, §26.1 texture format capabilities and §25 optional features](https://www.w3.org/TR/webgpu/) | Which formats are filterable/storage-capable, and that `texture-formats-tier1`/`tier2` gate `rgb10a2unorm` storage and read-write storage respectively |
| This repository | Measured traversal benchmark (`prototype/probe-gi-bench/`, results folded into `docs/research/probe-based-gi.md` §7.3), `afterglow-obvhs-tracer` dependency facts, WebGPU adapter feature/limit readouts from the native shell |

### What these sources do not establish

- No source gives a **measured cost for adaptive sampling on an iGPU**; ADGI and
  IS-DDGI report discrete-GPU figures. Their relative savings should transfer;
  their absolute timings should not be quoted for this hardware.
- The ADGI paper is a **poster**; the companion implementation details behind its
  4× memory claim and its chain-state storage are not fully specified in the PDF.
- IS-DDGI's paper body was not available (abstract and project page only), so its
  importance metrics are described here at the level the abstract supports — the
  specific weight functions are **not** documented in this note.
- No source measures the **WGSL** cost of the octahedral atlas, border updates, or
  the ping-pong blend on this hardware; §C.6 lists those as the next measurements.
- Unity's documentation does not publish a byte-level spec for its *chunk* /
  brick pool layout; only the per-probe struct and chunk indexing are visible in
  the public source.
