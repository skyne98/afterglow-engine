# Perceptual virtual-texture priority score — source audit

Date: 2026-07-25

## Question

Is there a known-good texture/tile streaming priority system that balances:

- importance near the expected screen center;
- physical closeness to the camera;
- screen coverage;
- and the quality of the fallback currently displayed?

No audited implementation provides that exact combination for file-backed VT
pages. Three strong public references provide the pieces needed for a small
Afterglow score without inventing an ungrounded heuristic.

## Selected sources

### Zhang et al. — perceptual GPU texture-streaming weight

Alex Zhang, Kan Chen, Henry Johan, and Marius Erdt, “High-performance
adaptive texture streaming and rendering of large 3D cities,” *The Visual
Computer* 38, 1245–1262 (2022), DOI:
<https://doi.org/10.1007/s00371-021-02152-z>.

The paper is open access under CC BY 4.0. Its implemented system writes
visibility/LOD metadata from rasterized fragments, compacts it on GPU, reads it
to CPU, and streams in descending weight order under bounded load-time and byte
budgets. The evaluated implementation used an RTX 2080 Ti, three city datasets,
a deterministic camera benchmark, and comparison against Unity 2020.2.

Its per-fragment weight is:

```text
W = 1 + 8 wd + 8 wm
```

where:

- the base `1`, summed over fragments, is screen coverage;
- `wd` increases foreground/camera-close importance;
- `wm` increases importance when the currently displayed mip is coarse.

The weight is atomically summed for every fragment belonging to one streamed
texture. The paper reports that adding distance and displayed-mip terms to
coverage produces a smoother distribution of texture resolution from
foreground to background than fragment count alone. Its benchmark also found
that arbitrary refinement order in Unity produced low-resolution blotchy areas,
while the proposed priority refined prominent foreground textures more
coherently.

Relevant formulas and policy:

```text
wd = (1 - cw)^4
cw = min(1 / gl_FragCoord.w, 1)

wm = min(M / Mmax, 1)
```

The exact distance normalization is tailored to that renderer/dataset. The
reusable result is the **per-fragment additive structure and equal 8× ranges for
camera distance and displayed quality**, not its one-unit cutoff.

The paper also:

- renders beyond the visible FOV at base weight to anticipate camera movement;
- rejects low total weights with a cutoff;
- uses a 16 ms maximum load interval;
- adapts byte workload and mip bias under frame pressure.

Afterglow should not copy its CPU parallel sort, sparse-image ring replacement,
or frame-latency controller. Those conflict with Afterglow's fixed priority
queues, second-chance atlas, and deterministic budgets.

### CesiumJS — foveated center plus camera distance

CesiumGS, CesiumJS merged streaming-optimization implementation, commit
`ab441f23f1447981705369e219fe71842200558e`:

- [`Source/Scene/Cesium3DTile.js`](https://github.com/CesiumGS/cesium/blob/ab441f23f1447981705369e219fe71842200558e/Source/Scene/Cesium3DTile.js)
- [`Source/Scene/Cesium3DTilesetTraversal.js`](https://github.com/CesiumGS/cesium/blob/ab441f23f1447981705369e219fe71842200558e/Source/Scene/Cesium3DTilesetTraversal.js)
- merged PR: <https://github.com/CesiumGS/cesium/pull/7774>

Cesium is a production open-source world/3D-tiles engine, not a VT page
streamer. Its useful mechanisms are:

1. a foveated factor based on angular distance from the camera view centerline;
2. explicit distance-to-camera priority;
3. normalization of priority attributes over the requested set;
4. stable composition of several priority dimensions into one sortable key;
5. deferral of peripheral requests while the camera is moving.

Its foveated factor is conceptually:

```text
foveatedFactor = 1 - abs(dot(cameraDirection, directionToTile))
```

A tile intersecting the centerline receives zero penalty. Cesium then
normalizes foveation and distance and places them in separate decimal ranges of
one priority number. That makes foveation lexicographically stronger than its
preferred distance sort in this historical implementation.

Afterglow should borrow the **camera-relative foveated term and normalized
integer composition**, but not Cesium's tree traversal, dynamic arrays, sorts,
or strict center-over-distance ordering. The requested close-edge-versus-deep-
corridor behavior requires equal, competing terms.

### id Tech 5 / RAGE — resident-quality deficit plus coverage

The public RAGE synthesis is in
[`id-tech-virtual-texturing-audit.md`](id-tech-virtual-texturing-audit.md), based
on J. M. P. van Waveren's 2012 *Software Virtual Textures* paper.

RAGE prioritizes a page first by the gap between desired mip and nearest
resident mip, then improves/breaks priority with feedback sample count. This is
the page-level analogue of Zhang et al.'s displayed-mip contribution and is a
better fit for Afterglow than absolute mip number because Afterglow always has a
resident parent/tail fallback.

Afterglow should use **desired-to-resident mip gap**, not merely desired mip.
That prioritizes the largest visible quality correction.

## References rejected as the score source

- Unreal's public API exposes only `Normal`/`High` page-request priority and does
  not publish the concrete SVT score. EULA-gated source was not used.
- GameTechDev's `SamplerFeedbackStreaming` is a strong file-backed lifecycle
  reference but does not provide the required perceptual center/distance score.
- Wicked and Dagor implement generated terrain caches, not this file-backed
  page-streaming policy.
- id Tech 4 `idMegaTexture` has no feedback priority queue.

## Selected Afterglow adaptation

The smallest evidence-backed score is Zhang's additive fragment weight, with:

- Cesium-style foveation measured around the **100 ms predicted camera**;
- Zhang-style foreground distance;
- Zhang/RAGE-style currently displayed quality deficit;
- fragment count supplied by existing feedback coverage.

Quantize all perceptual terms to the same 3-bit range:

```text
qf ∈ [0, 7]  predicted-center closeness
qd ∈ [0, 7]  camera-distance closeness
qg ∈ [0, 7]  desired-to-resident mip gap
```

For each feedback pixel of a page:

```text
pixelWeight = 1 + qf + qd
```

For one deduplicated page:

```text
Wpage = Σ pixelWeight + coverage × qg
```

This is the integer form of:

```text
coverage + foveation contribution + distance contribution
         + displayed-quality contribution
```

All three non-coverage terms have equal maximum influence. Therefore:

- a deep center page receives high `qf` but low `qd`;
- a close edge page receives low `qf` but high `qd`;
- the page with the larger visible quality deficit receives higher `qg`;
- broad visible pages accumulate more base/weighted fragments.

This deliberately replaces the earlier absolute “center always wins” rule. The
new user requirement is a balance: expected center matters strongly, but camera-
close edge detail must be able to outrank a deep corridor.

### Encoding without another feedback target

Current feedback word zero uses:

```text
bit 31       valid
bits 17–27   page y
bits 6–16    page x
bits 0–5     mip
```

Bits 28–30 are unused. Store `qd` there. `qf` remains a CPU calculation from the
feedback pixel's screen coordinate, and `qg` remains a CPU calculation from the
packed page table. The RG32Uint target, readback size, page identity, and word
one texture ID remain unchanged.

Use logarithmic camera-relative depth between the active camera's near/far
planes for the first measured prototype:

```text
d = clamp(log(viewDistance / near) / log(far / near), 0, 1)
qd = round(7 × (1 - d))
```

This avoids assuming world units are meters and gives useful resolution to the
foreground. Linear versus logarithmic normalization is a measured shader test;
logarithmic is the recommended default.

Use eight equal-area foveated bands from the existing squared radial score:

```text
qf = 7 - min(7, screenPriority >> 5)
```

Compute `qg` as the desired-to-nearest-resident mip difference, clamped to 7.

### Fixed-queue adaptation

Zhang et al. sort exact weights. Afterglow must not sort or allocate in sealed
runtime. Cap weight accumulation at 255 feedback samples per page, then map the
bounded integer `Wpage` through one fixed geometric bucket function. Keep
approximately the current scheduler lane count by composing:

```text
importance bucket → parent/exact → channel
```

No separate radial hierarchy, distance queue, heap, or general score object is
needed. The bucket function and thresholds must be unit-tested for monotonicity.

## Verdict

Use the composed integer score above. It is not claimed to be a formula shipped
unchanged by one game. It is a minimal page-level adaptation of three publicly
verifiable, known-good mechanisms:

- additive per-fragment coverage/distance/displayed-quality weighting from an
  evaluated peer-reviewed texture streamer;
- foveated center priority from Cesium;
- desired-versus-resident mip deficit and sample count from RAGE.

The weights are deliberately equal and fixed. Do not expose tuning sliders until
RTX 3090 and Radeon 680M corridor/edge tests prove a different ratio is needed.
