# What is the highest-quality TAA implementation available today?

**Date:** 2026-07-21
**Verdict:** There is no single "best" TAA — quality is per-implementation.
The strongest cross-checked references for a non-ML, WebGPU/WGSL engine are
**Filmic SMAA / Dynamic TAA (Activision)** for the technique, **Intel TAA**
for the open-source reference resolve, **Decima's 2-frame no-accumulation
design** for zero-smear architecture, and **k-DOP Clipping (SIGGRAPH Asia
2024)** for next-gen ghosting mitigation. This doc records the cross-checked
evidence, source-code locations for each, and the extracted formulas /
pseudocode from the paper-only sources.

---

## 1. The landscape (cross-checked)

Tiering below is the result of three rounds of cross-checking against primary
sources (papers, READMEs, NVIDIA Research, Digital Foundry articles) rather
than forum anecdotes. Each entry notes the evidence strength.

### Tier 1 — Best of the best

**DLAA (DLSS 4 Transformer model) — NVIDIA.** Digital Foundry (primary
source, Alex Battaglia, Feb 2024): *"The ultimate form of TAA right now is
Nvidia's DLAA (effectively DLSS rendering at native resolution) which can
actually look significantly better even than standard super-sampling."* The
DLSS 4 transformer model (Jan 2025) uses a vision transformer with double the
parameters of the prior CNN, evaluating relative importance of every pixel
across the whole frame over multiple frames. TechPowerUp: *"The ghosting and
smearing artifacts in low contrast scenarios are gone as well, producing a
very stable image in motion."* **Caveat (cross-checked):** not uniformly
"zero smear." GamersNexus found counterexamples (*"the sword moves across
the arm again, leaving behind a bad smear... the model switch isn't just a
straight upgrade and comes with tradeoffs"*), and Digital Foundry noted
volumetric-fog ghosting in 4.0 fixed in 4.5. Hardware-gated to RTX Tensor
Cores — **not viable for WebGPU.**

**Filmic SMAA / Dynamic TAA — Activision (Jorge Jimenez).** The strongest
*expert-endorsed non-ML* technique. Alex Tardif (graphics programmer, author
of the canonical TAA Starter Pack): *"To me, Activision's work with Filmic
SMAA in CoD is simply the best of the state of the art solutions available...
The morphological component makes for a massive improvement in sharpness/
clarity over the more common style of TAA. Truth be told I'm not sure why
more studios aren't adopting this methodology."* A hybrid morphological +
temporal approach (SMAA T2x lineage) that keeps sharpness via edge detection
while adding temporal stability. Papers only (§3) — base SMAA T2x is open
source (§2).

**Decima Engine TAA — Guerrilla Games.** Primary source: Decima SIGGRAPH
2017 paper. Cited repeatedly by the r/FuckTAA community (otherwise TAA-hostile)
as the only truly great shipped TAA. Unique 2-frame no-accumulation design:
reuses the previous frame's *raw input*, not accumulated output, preventing
long ghosting trails. Paired with FXAA for edge cleanup. Paper only (§3) —
proprietary, no source.

**id Tech 6/7 TAA — id Software.** **Downgraded to Soft evidence after
cross-check.** The "ridiculously sharp with minimal ghosting" quote is a
ResetEra forum anecdote, not Digital Foundry. The Simon Coenen DOOM Eternal
Graphics Study's "customized velocity buffer... little ghosting and player
gun excluded to prevent smearing" describes **motion blur**, not TAA
(misattribution found in round 2). The only authoritative praise is DF's 2016
Doom face-off: *"Temporal Super-Sampling Anti-Aliasing... effectively
eliminates edge and in-surface aliasing."* Technically functional, no
authoritative top-tier endorsement.

### Tier 1.5 — Strong open-source references

**Intel TAA (GameTechDev/TAA).** Best open-source *pure TAA* reference
resolve. YCoCg variance clipping, longest-velocity-vector dilation, bicubic
history filter, depth-threshold history rejection. Discontinued (README
states Intel ceased development) but still the cleanest code to study. Full
source (§2).

**k-DOP Clipping — Tampere University (SIGGRAPH Asia 2024).** Replaces AABB
color clipping with k-Discrete Oriented Polytopes for more robust ghosting
mitigation. 0.2 ms overhead. **Attribution correction:** Tampere University
(Julius Ikkala, Tuomas Lauttia, Pekka Jääskeläinen, Markku Mäkitalo), **not
NVIDIA** (misattributed in round 1). MIT-0 licensed, GLSL, drop-in. Full
source (§2).

### Tier 2 — Exceptional but gated / contested

**FSR 3.1/4 Native AA — AMD.** **Downgraded after cross-check.** The "FSR 3
Native AA crisper than DLAA" claim was a single Steam forum anecdote (RDR2),
not broad consensus. Rigorous benchmarks (TechPowerUp, TechSpot) consistently
rank DLSS ahead; FSR 4 (RDNA 4) "closed the gap significantly" but remains
behind. FSR 3 has documented noise in grass/shadows. Not a serious contender.

**UE5 TSR.** Better than UE4 TAA (confirmed), less ghosting. vs DLSS/FSR is
contested — some claim "TSR has less ghosting than DLAA," others place it
behind DLSS. Heavily dependent on screen percentage (200% eliminates most
ghosting).

**NVIDIA ATAA (Adaptive TAA, 2018).** Extends TAA with adaptive ray tracing;
*"removes blurring and ghosting artifacts"* (NVIDIA Research, primary
source). Never shipped in a game; superseded by DLSS/DLAA.

### Tier 3 — High-quality non-temporal (for comparison)

**SMAA T2x.** Not pure TAA but includes temporal supersampling. r/FuckTAA:
*"SMAA T2x is a stability sweet spot."* Best choice if you want zero temporal
artifacts but still some temporal edge sampling.

---

## 2. Source code locations

### Filmic SMAA / Dynamic TAA (Activision) — ⚠️ Partially available

Filmic SMAA itself is **NOT open-sourced** by Activision (paper/PPTX only).
The base SMAA (including the T2x temporal variant) that Filmic SMAA builds on
IS open source.

**Base SMAA + T2x temporal** — Jorge Jimenez's original:
- **Repo:** `github.com/iryoku/smaa` (license: NOASSERTION/custom, free for
  commercial use per README)
- **Key file:** `SMAA.hlsl` — single self-contained HLSL header
- **T2x temporal support:** confirmed present via `SMAA_REPROJECTION` macro —
  velocity-weighted reprojection to remove ghosting during temporal
  supersampling. Key snippet:
  ```hlsl
  #define SMAA_REPROJECTION 0
  #define SMAA_REPROJECTION_WEIGHT_SCALE 30.0
  // ...
  float weight = 0.5 * saturate(1.0 - sqrt(delta) * SMAA_REPROJECTION_WEIGHT_SCALE);
  ```
- **T2x announcement:** `iryoku.com/smaa-t2x-source-code-released/` (Oct
  2011, integrated into CryEngine 3 with Tiago Sousa)

**GLSL/WebGL port** (directly relevant for afterglow-engine WebGPU porting):
- **Repo:** `github.com/dmnsgn/glsl-smaa` (license: MIT)
- **Files:** `smaa-blend.frag/.vert`, `presets.glsl`, example shaders under
  `examples/shaders/`
- **Caveat:** covers the **spatial SMAA 1x** passes (edge detection + blend)
  only. The **T2x temporal reprojection pass is NOT in this port** — port it
  from `SMAA.hlsl`'s `SMAA_REPROJECTION` block.

**Paper-only (not source):**
- Filmic SMAA slides/pseudocode:
  `research.activision.com/publications/archives/filmic-smaasharp-morphological-and-temporal-antialiasing`
  PPTX: `advances.realtimerendering.com/s2016/Filmic SMAA v7.pptx`
- Dynamic TAA and Upsampling in CoD (2020 evolution) PDF:
  `activision.com/cdn/research/Dynamic_Temporal_Antialiasing_and_Upsampling_in_Call_of_Duty_v4.pdf`

### Intel TAA — ✅ Full source

- **Repo:** `github.com/GameTechDev/TAA` (license: NOASSERTION — Intel
  discontinued, but freely usable)
- **Status:** README states *"DISCONTINUATION OF PROJECT — Intel has ceased
  development."* Static reference, not maintained.

Key files (all under `MiniEngine/Core/`):

| File | Purpose |
|---|---|
| `Shaders/TAAResolve.hlsl` | **Main TAA resolve pixel shader** — canonical reference |
| `Shaders/ResolveTAACS.hlsl` | Compute shader variant (uses TGSM/shared memory) |
| `Shaders/SharpenTAACS.hlsl` | Post-TAA sharpening compute pass |
| `Shaders/TemporalBlendCS.hlsl` | Temporal blend compute |
| `TemporalEffects.cpp` / `.h` | C++ orchestration: history buffer mgmt, jitter, dispatch |

Critical `#define` switches (verified in README):
- `USE_VARIANCE_CLIPPING` — the main anti-ghosting (variance/AABB clip)
- `USE_YCOCG_SPACE` — YCoCg color space for clipping (best quality)
- `USE_LONGEST_VELOCITY_VECTOR` — velocity dilation for edge AA (9 or 5 samples)
- `USE_BICUBIC_FILTER` — 5-tap Catmull-Rom history sampling
- `USE_DEPTH_THRESHOLD` — depth-based history rejection
- `USE_FP16`, `USE_TGSM` — performance optimizations

**For afterglow-engine:** Port `TAAResolve.hlsl` to WGSL. It's HLSL but
straightforward. The compute variant (`ResolveTAACS.hlsl`) maps well to
WebGPU compute shaders.

### Decima 2-frame no-accumulation TAA — ❌ No source (proprietary)

**No source code exists publicly.** Guerrilla Games' Decima engine is
proprietary. Only the SIGGRAPH 2017 paper describes the architecture (§3C).
The paper explicitly defers reprojection/rejection code: *"For other aspects
of our TAA solution like temporal reprojection and neighborhood-based
rejection criteria, we used some more traditional approaches, so we won't
cover those here."* Use Intel TAA's variance clipping or k-DOP as the
"traditional approaches" Decima refers to.

### k-DOP Clipping — ✅ Full source, GLSL

- **Repo:** `github.com/vga-group/taa-kdop-optimizer` (license: **MIT-0** —
  most permissive, ideal)
- **Paper:** `webpages.tuni.fi/vga/publications/k_DOP_Clipping.html`

| File | Purpose |
|---|---|
| **`kdop_clipping.glsl`** | **The actual shader — copy-pastable drop-in replacement for RGB/YCoCg color clipping in any TAA** |
| `kdop_volume.hh` | C++ k-DOP bounding volume helpers |
| `sphere_optimizer.cc` | Tool to precompute optimized axis sets (sphere-bounding) |
| `image_optimizer.cc` | Tool to optimize axes against a reference image |

`kdop_clipping.glsl` is MIT-0, GLSL ES, self-contained. A drop-in function
replacing the existing color-clamp/clip step. Pre-baked 32-DOP axis set (16
axes) from the paper, with comments on generating cheaper sets.
`neighborhood_size = 9` (3×3) default, configurable. Header comment:
*"This function is a drop-in replacement for RGB or YCoCg color clipping in
temporal anti-aliasing... The k-DOP approximation is more accurate than
AABB-based RGB or YCoCg clipping."*

**For afterglow-engine:** The single most directly-usable artifact. GLSL→WGSL
is trivial (pure math, no sampler tricks). Drop into the Intel TAA resolve in
place of the `clip_aabb` function. 0.2 ms overhead per the paper.

### Bonus: Playdead (INSIDE) — canonical clip_aabb source

- **Repo:** `github.com/playdeadgames/temporal` (license: **MIT**)
- **Files:** `Assets/Shaders/TemporalReprojection.shader` (contains
  `clip_aabb`), `Assets/Shaders/VelocityBuffer.shader`,
  `Assets/Scripts/TemporalReprojection.cs`, `Assets/Scripts/FrustumJitter.cs`,
  `GDC2016_Temporal_Reprojection_AA_INSIDE.pdf`
- Unity/HLSL format (`.shader` files), needs adaptation, but the `clip_aabb`
  function is the most-cited variance clipping reference in TAA literature.

### Summary table

| Recommendation | Source available? | Language | License | Port effort → WGSL |
|---|---|---|---|---|
| **Filmic SMAA** | ❌ Paper only | — | — | Reimplement from slides |
| **Base SMAA T2x** | ✅ `iryoku/smaa` | HLSL | Custom (free) | Medium (single header) |
| **SMAA spatial (GLSL)** | ✅ `dmnsgn/glsl-smaa` | GLSL ES | MIT | Low (spatial only, no T2x) |
| **Intel TAA** | ✅ `GameTechDev/TAA` | HLSL/C++ | NOASSERTION | Medium (main resolve) |
| **Decima TAA** | ❌ Paper only | — | — | Architecture guidance only |
| **k-DOP Clipping** | ✅ `vga-group/taa-kdop-optimizer` | **GLSL** | **MIT-0** | **Lowest** (drop-in, pure math) |
| **Playdead clip_aabb** | ✅ `playdeadgames/temporal` | Unity/HLSL | MIT | Low (one function) |

### Recommended afterglow-engine implementation path

1. Start with **Intel TAA's `TAAResolve.hlsl`** as the resolve skeleton
   (variance clipping + YCoCg + longest-velocity + bicubic history)
2. Replace its `clip_aabb` with **`kdop_clipping.glsl`** from vga-group
   (MIT-0, GLSL→WGSL trivial, 0.2 ms, formally better ghosting mitigation)
3. For the spatial AA companion, port **SMAA T2x from `iryoku/smaa`** (or use
   the `dmnsgn/glsl-smaa` GLSL spatial passes as a WebGPU-friendly reference)
4. For zero-smear architecture inspiration (not code), study the **Decima
   paper's** 2-frame no-accumulation design (§3C)
5. Drop in **Activision's 1-sample spatio-temporal bicubic** (§3B4) to replace
   the expensive history bicubic — 5× cheaper, sharper

The two MIT/MIT-0 GLSL artifacts (k-DOP + glsl-smaa) are the lowest-friction
wins for a WebGPU/WGSL engine.

---

## 3. Paper-only sources: extracted formulas & pseudocode

Every formula, shader snippet, and algorithmic step extracted from the three
paper-only sources. Some code blocks in the PDFs suffered OCR garbling —
flagged where present, reconstructed from surrounding math where unambiguous.

### 3A. Filmic SMAA v7 (Jimenez, SIGGRAPH 2016)

`advances.realtimerendering.com/s2016/Filmic SMAA v7.pptx` (143 slides)

#### 3A1. Architecture: three decoupled components

Filmic SMAA separates temporal supersampling from temporal filtering (the key
architectural insight):

```
Temporal Resolve:      Frame N-1 (raw) + Frame N (raw) → Antialiased Image
                                    ↓
Temporal Filter:      Antialiased Image → Exponential History Buffer (recursive)
```

> *"Using an exponential history is a process that can be decoupled from
> supersampling with subpixel jitters... Objects in motion will have AA even
> if no jitter is used."*

Filmic SMAA T2x = morphological (SMAA 1x) + 2× temporal supersampling +
exponential history temporal filter. Contrast with plain SMAA T2x which has
no temporal filter (→ ghosts on velocity-less objects).

#### 3A2. Morphological edge suppression — factorized implementation

Replaces SMAA's local-contrast edge detection. Instead of searching for edge
endpoints, it scores *all patterns passing through the current edge* vs *all
patterns not passing through*, and suppresses the edge if the non-passing
pattern wins. `v[]` = vertical neighbors, `h[]` = horizontal neighbors, `vv`
= an extra value, `k1`/`k2` = tuning constants:

```hlsl
float t1 = v[0] - h[0] - h[1];
float t2 = v[2] - h[3] - h[4];
float t3 = h[0] - h[1];
float t4 = h[3] - h[4];
float t5 = v[0] - v[1];
float t6 = v[2] - v[1];
float t7 = v[0] + t1;
float t8 = v[2] + t2;
float t9 = v[2] + t5;

float pattern1 = max3(t1 + t2 + v[1], k1 * (abs(t3 + t4) - t9),
                      max3(t3 + t8 - vv, t4 + t7 - vv,
                           max3(-t4 + t7, -t3 + t8, abs(t3 - t4) - vv) - v[4]) - t9);
float pattern2 = max(mad(-2.0, v[1], -min(t1, t2)),
                     mad(k2, max(abs(t3) + t5, abs(t4) + t6), -v[1]));

return max(2.0 * threshold, pattern2) < pattern1;
```

The equivalent factorized form (clearer, same result):

```hlsl
float pattern1 = 0.0;
pattern1 = max(pattern1, v[0] + v[1] + v[2] - (h[0] + h[1] + h[3] + h[4]));
pattern1 = max(pattern1, k1 * (h[0] + v[1] + h[3] - (v[0] + h[1] + v[2] + h[4])));
pattern1 = max(pattern1, k1 * (h[1] + v[1] + h[4] - (v[0] + h[0] + v[2] + h[3])));
pattern1 = max(pattern1, h[0] + v[1] + h[4] - (v[0] + h[1] + h[3] + v[2] + v[4] + vv));
pattern1 = max(pattern1, h[3] + v[1] + h[1] - (h[0] + v[0] + v[2] + h[4] + v[4] + vv));
pattern1 = max(pattern1, h[0] + v[1] + v[2] - (v[0] + h[1] + h[3] + h[4] + vv));
pattern1 = max(pattern1, h[1] + v[1] + v[2] - (h[0] + v[0] + h[3] + h[4] + v[4]));
pattern1 = max(pattern1, h[3] + v[1] + v[0] - (h[0] + h[1] + v[2] + h[4] + vv));
pattern1 = max(pattern1, h[4] + v[1] + v[0] - (h[0] + h[1] + v[2] + h[3] + v[4]));

float pattern2 = 0.0;
pattern2 = max(pattern2, h[0] + h[1] - (v[0] + v[1]));
pattern2 = max(pattern2, h[3] + h[4] - (v[1] + v[2]));
pattern2 = max(pattern2, k2 * (h[0] + v[0] - (h[1] + v[1])));
pattern2 = max(pattern2, k2 * (h[1] + v[0] - (h[0] + v[1])));
pattern2 = max(pattern2, k2 * (h[3] + v[2] - (h[4] + v[1])));
pattern2 = max(pattern2, k2 * (h[4] + v[2] - (h[3] + v[1])));

return 2.0 * threshold < pattern1 && pattern2 - pattern1 < v[1];
```

#### 3A3. History bicubic filter (5-sample Catmull-Rom) — full shader

The canonical sharp history resampling filter.
`SMAA_FILMIC_REPROJECTION_SHARPNESS` (0–100) controls the Catmull-Rom C
parameter. Optimized 9→5 bilinear-tap version (corners dropped):

```hlsl
float3 SMAAFilterHistory(SMAATexture2D colorTex, float2 texcoord, float4 rtMetrics)
{
    float2 position = rtMetrics.zw * texcoord;
    float2 centerPosition = floor(position - 0.5) + 0.5;
    float2 f = position - centerPosition;
    float2 f2 = f * f;
    float2 f3 = f * f2;

    float c = SMAA_FILMIC_REPROJECTION_SHARPNESS / 100.0;
    float2 w0 =        -c  * f3 +  2.0 * c         * f2 - c * f;
    float2 w1 =  (2.0 - c) * f3 - (3.0 - c)        * f2         + 1.0;
    float2 w2 = -(2.0 - c) * f3 + (3.0 -  2.0 * c) * f2 + c * f;
    float2 w3 =         c  * f3 -                c * f2;

    float2 w12 = w1 + w2;
    float2 tc12 = rtMetrics.xy * (centerPosition + w2 / w12);
    float3 centerColor = SMAASample(colorTex, float2(tc12.x, tc12.y)).rgb;

    float2 tc0 = rtMetrics.xy * (centerPosition - 1.0);
    float2 tc3 = rtMetrics.xy * (centerPosition + 2.0);
    float4 color =
        float4(SMAASample(colorTex, float2(tc12.x, tc0.y )).rgb, 1.0) * (w12.x * w0.y ) +
        float4(SMAASample(colorTex, float2(tc0.x,  tc12.y)).rgb, 1.0) * (w0.x  * w12.y) +
        float4(centerColor,                                      1.0) * (w12.x * w12.y) +
        float4(SMAASample(colorTex, float2(tc3.x,  tc12.y)).rgb, 1.0) * (w3.x  * w12.y) +
        float4(SMAASample(colorTex, float2(tc12.x, tc3.y )).rgb, 1.0) * (w12.x * w3.y );
    return color.rgb * rcp(color.a);
}
```

#### 3A4. Temporal spatial contrast tracking (anti-flicker weighting)

Detects flickering pixels by tracking spatial contrast (neighborhood min/max
luma) across frames, and weights history down when it flickers:

```hlsl
// Spatial contrast weight from neighborhood min/max luma:
spatialContrast.weight = SMAAContrastWeight(SMAA_FILMIC_WEIGHTING_SPATIAL_MIN,
                                             SMAA_FILMIC_WEIGHTING_SPATIAL_MAX,
                                             result.minLuma, result.maxLuma);

// Temporal delta of spatial contrast = flicker detector:
float spatialContrastWeight = abs(currentSpatialContrast.weight -
                                  historySpatialContrast.weight);

// Convergence time: high normally, very high on flicker:
float convergenceTime = lerp(lerp(SMAA_FILMIC_WEIGHTING_STRENGTH_LOW,
                                  SMAA_FILMIC_WEIGHTING_STRENGTH_HIGH,
                                  temporalContrastWeight),
                             SMAA_FILMIC_WEIGHTING_STRENGTH_FLICKER, // very high
                             spatialContrastWeight);

float weight = SMAAFilmicStrength(framesPerSecondRcp, convergenceTime);
return lerp(currentColor, historyColor, weight);
```

#### 3A5. FPS-aware exponential blend factor

Makes the temporal filter framerate-independent (same convergence in seconds
regardless of FPS):

```hlsl
float Alpha(float framesPerSecondRcp, float convergenceTime)
{
    return exp(-framesPerSecondRcp / convergenceTime);
}
```

#### 3A6. Depth-tested disocclusion (extends neighborhood clamp)

When AABB clamp fails on high-frequency neighborhoods, use half-res
nearest-depth comparison between current and previous frame depth. Also use
the "responsive AA" pass for alpha-blended objects (no history). The 0.6-scaled
neighborhood (blurred clamp) is an alternative to variance clipping:

```hlsl
// Blurred (low-pass) neighborhood for tighter, more stable clamp:
neighborhood[0] = SMAASample(colorTex, mad(SMAA_RT_METRICS.xy, 0.6 * float2(-1.0, -1.0), texcoord)).rgb;
neighborhood[1] = SMAASample(colorTex, mad(SMAA_RT_METRICS.xy, 0.6 * float2(-1.0,  1.0), texcoord)).rgb;
neighborhood[2] = SMAASample(colorTex, mad(SMAA_RT_METRICS.xy, 0.6 * float2( 1.0, -1.0), texcoord)).rgb;
neighborhood[3] = SMAASample(colorTex, mad(SMAA_RT_METRICS.xy, 0.6 * float2( 1.0,  1.0), texcoord)).rgb;
```

#### 3A7. Supersampling derivatives (mip bias for rotated-grid jitter)

For 2× rotated-grid temporal supersampling, the effective derivative is
smaller (diagonal), so textures get blurrier. Bias the mip selection by the
diagonal factor √0.5 = 0.7071:

```hlsl
// Three equivalent ways to apply the bias:
SampleGrad(..., 0.7071 * ddx, 0.7071 * ddy);      // explicit
SampleBias(..., log2(0.7071));                     // cheaper
// Best: set texture sampler LOD bias = log2(0.7071)

// Mip selection math (OpenGL spec 3.9.11):
float grad = max(length(ddx), length(ddy));
float mip  = log2(grad);
// desired:
float mip  = log2(0.7071 * grad);   // == log2(0.7071) + log2(grad)
```

#### 3A8. Unjittering diffuse UVs (alternative to mip bias)

Removes the jitter-induced texture blur by offsetting the diffuse/normal UVs
back to the pixel center using derivatives (free, loses shading AA on normals
— apply to diffuse only):

```hlsl
input.texcoord += ddx(input.texcoord) * subpixelOffset.x;
input.texcoord -= ddy(input.texcoord) * subpixelOffset.y;
```

#### 3A9. Low-footprint velocity buffers (8-bit, two-piece linear)

Replaces 16-bit velocity with 8-bit using a two-piece linear encoding (0–40px
range, smooth transition, constant precision over longer range than gamma):

> *"8-bit Gamma [Sousa2013]... We opted for a two-piece linear function:
> 0..40 pixels range, not completely linear: smooth transition, constant
> precision over a longer range than a gamma."*

#### 3A10. Faster closest velocity (half-res, gather-based)

Closest-depth velocity dilation reduced from 10 samples to 3 gathers:

```
2 gathers for velocity (GatherRed + GatherGreen) + 1 gather for depth
→ half-res + 8-bit velocity: 7.91 MB → 0.98 MB for 1080p
```

#### 3A11. Performance (PS4 @ 1080p)

- Deferred queues + dual edge blending: **0.16 ms saved**
- LDS (Gather4 preload): **0.14 ms saved**
- Total morphological: **0.475 ms** (down from 0.774 ms) — *faster than FXAA*

---

### 3B. Dynamic TAA & Upsampling in CoD (Jimenez, SIGGRAPH 2017 / Digital Dragons 2018)

`activision.com/cdn/research/Dynamic_Temporal_Antialiasing_and_Upsampling_in_Call_of_Duty_v4.pdf`
(122 slides)

#### 3B1. Filmic SMAA components (recap, p7)

Three components: **Morphological** + **Temporal Supersampling** + **Temporal
Filtering**. The Neighborhood Clamp origin is cited as [Lottes2011].

#### 3B2. Dynamic AA Algorithm (TU2x, horizontal upsample) — full pseudocode

Interlaces previous (even columns) and current (odd columns) frames into a 2×
virtual image, then bilinearly downsamples to output res. Four steps:

**Step 1** — find output pixel position in virtual 2× image, fractional =
bilinear weight:

```
upsampledPosition = 2.0 * (outputCoord) - 0.5
weight = frac(upsampledPosition)
```

**Step 2** — odd positions cross 2-pixel block boundaries; offset current or
previous:

```hlsl
int  mod2           = SMAMod2(upsampledPosition);
float currentOffset  = mod2 &&  subsampleIndex ? 1.0 : 0.0;  // |c|p| -> |.|p|c|
float previousOffset = mod2 && !subsampleIndex ? 1.0 : 0.0;  // |p|c| -> |.|c|p|
```

**Step 3** — apply offsets, snap to texel centers, convert to texture coords
(⚠️ OCR-garbled in PDF; reconstructed from structure):

```hlsl
texcoord.x          = (1.0 / inputDimensions.x) * (floor(0.5 * (floor(upsampledPosition) + currentOffset)  + 0.25) + 0.5);
previousTexcoord.x  = (1.0 / inputDimensions.x) * (floor(0.5 * (floor(upsampledPosition) + previousOffset) + 0.25) + 0.5);
```

**Step 4** — if previous is on the left, reverse the weight; blend:

```hlsl
bool subsampleSwap = SMAAXor(subsampleIndex, mod2);
weight      = subsampleSwap ? weight : 1.0 - weight;
outputColor = lerp(currentColor, previousColor, weight);
```

#### 3B3. Temporal checkerboard / differential blend (TU4x, 4× upsample) — full shader

Extends [Berghoff2016] differential blend to a temporal checkerboard.
Reconstructs missing samples by picking the neighbor blend (horizontal vs
vertical) with lowest color difference:

```hlsl
// Two candidate 4-neighborhoods (current/previous swapped):
float3 neighborhood1[4] = {
    currentNeighborhood[SMAA_NEIGHBORHOOD_WEST],   // W
    currentColor,                                  // E
    previousColor,                                 // N
    previousNeighborhood[SMAA_NEIGHBORHOOD_SOUTH]  // S
};
float3 neighborhood2[4] = {
    previousColor,                                 // W
    previousNeighborhood[SMAA_NEIGHBORHOOD_EAST],  // E
    currentNeighborhood[SMAA_NEIGHBORHOOD_NORTH],  // N
    currentColor                                   // S
};

float3 weights = SMADifferentBlendCalculateWeight(neighborhood1, neighborhood2);
float3 previousReconstructedColor = SMADifferentBlend(neighborhood1, weights); // p'
float3 currentReconstructedColor  = SMADifferentBlend(neighborhood2, weights); // c'

previousColor = lerp(previousColor, previousReconstructedColor, 0.5);
currentColor  = lerp(currentColor,  currentReconstructedColor,  0.5);
```

The differential blend itself (west+east pair weighted by `weights.x`,
north+south by `weights.y`, normalized by `weights.z`):

```hlsl
float3 SMADifferentBlend(float3 neighborhood[4], float3 weights)
{
    float4 color = 0.0;
    color += float4(neighborhood[SMAA_NEIGHBORHOOD_WEST]  + neighborhood[SMAA_NEIGHBORHOOD_EAST],  1.0) * weights.x;
    color += float4(neighborhood[SMAA_NEIGHBORHOOD_NORTH] + neighborhood[SMAA_NEIGHBORHOOD_SOUTH], 1.0) * weights.y;
    return (0.5 * weights.z) * color.rgb;
}
```

#### 3B4. 1-sample spatio-temporal bicubic — the key derivation

**The most important formula in the paper.** Replaces 5–9 sample bicubic
history filtering with **1 sample + 4 neighborhood taps already needed for
the clamp.**

**Neighborhood approximation** (estimate history neighbors `o` from current
frame `c`):

```
o_w ≈ o_m + (c_w - c_m)
o_e ≈ o_m + (c_e - c_m)
o_n ≈ o_m + (c_n - c_m)
o_s ≈ o_m + (c_s - c_m)
```

> *"Very good match if reprojecting inside of an object. On edges they will be
> different — slightly reintroduces some aliasing but much sharper details.
> It actually brings back real details rather than sharpening the history
> buffer."*

**Mitchell-Netravali bicubic** (the expensive baseline being simplified):

$$f(x) = \begin{cases} (12-9B-6C)|x|^3 + (-18+12B+6C)|x|^2 + (6-2B), & |x|<1 \\ (-B-6C)|x|^3 + (6B+30C)|x|^2 + (-12B-48C)x + (8B+24C), & 1 \le |x| \le 2 \\ 0, & \text{otherwise} \end{cases}$$

**Weight ratios** (deriving the simplification):

- Left pixel weight: $m_0(x) = \frac{w_0(x)}{w_{12}(x)} = \frac{x(1+(x-2)x)}{(x-1)x-1}$
- Right pixel weight: $m_3(x) = \frac{w_3(x)}{w_{12}(x)} = \frac{(1-x)x}{(x-1)x-1}$
- Combined (assuming left≈right): $m_{03}(x) = m_0(x) + m_3(x)$
- **Fitted approximation:** $m_{03}'(x) = x(0.8x - 0.8)$

**Final simplified shader** (replaces the entire bicubic):

```hlsl
m03           = x * (0.8 * x - 0.8);
color         = lerp(left, right, x);
filteredColor = (m03 * color + 1.0 * historyColor) / (m03 + 1.0);
```

**Performance (PS4):**

| Variant | Vector ALU | Vector mem | Est. cycles |
|---|---|---|---|
| 9-sample spatial bicubic | 78 | 9 | 1868 |
| 5-sample spatial bicubic | 69 | 4+1 | 978 (1.91×) |
| **1-sample spatio-temporal** | **51** | **1+4** | **372 (5×)** |

#### 3B5. Dynamic (velocity-aware) subpixel jittering

Halton-16 isn't always better than 1×/2× jitter in motion — optimal sampling
depends on velocity. Alter the jitter scale by per-pixel velocity in the
vertex shader:

```hlsl
float2 scale = 0.5 + 0.5 * cos((3.141592 / jitterDistance) * velocity);
svPosition.xy += scale * jitter.xy * svPosition.w;
// velocity ~0.5px → no jitter; velocity ~1px → 2× jitter
```

#### 3B6. Halfres velocity packing (float lexicographic trick)

Pack depth (10-bit) + velocity (22-bit) into one UINT. Exploit that
**IEEE-754 floats are lexicographically ordered** to select closest velocity
with a single `min`:

```hlsl
// Lexicographic ordering property:
// min(x, y) == asfloat(min(asuint(x), asuint(y)))

uint velDepth1, velDepth2;
uint nearestVel = min(velDepth1, velDepth2);   // one instruction per sample
// (depth in MSBs so min picks closest depth → its velocity)
```

Reduces Gathers from 3 → 1; footprint 7.91 MB → 0.98 MB.

#### 3B7. TU4x performance (PS4 @ 1080p output)

| Input | TU2x | TU4x |
|---|---|---|
| 1920×1080 | 0.8–0.99 ms | — |
| 1440×1080 | 0.75–0.94 ms | 0.95–1.2 ms |
| 960×1080 | 0.67–0.9 ms | 1.0–1.26 ms |

---

### 3C. Decima Engine 2-frame TAA (de Carpentier, SIGGRAPH 2017)

`advances.realtimerendering.com/s2017/DecimaSiggraph2017.pdf` (pp. 28–43).
No shader code — architecture and pseudocode only.

#### 3C1. Core design: no accumulation buffer (the zero-smear decision)

> *"Instead of somehow accumulating the output from previous frames, we chose
> to only reuse the **input** to the AA system from the previous frame, but
> not the previous output. That is, we use the **raw render** of the previous
> frame and the current frame, but nothing more. That way, we prevent any
> long ghosting trails caused by failed rejects, and we reach a stable and
> final result in **only two frames**, making our technique very responsive."*

**Trade-off explicitly stated:** more responsive, less ghosting, at the cost
of less temporal stability than full-accumulation TAA.

#### 3C2. Edge-sampled jitter (flipquad-like, not corners)

> *"We never render at the pixel centers, but only on the pixel's edges. We
> render **horizontally between** the pixel centers for odd frames, and
> **vertically between** pixel centers for even frames."*

(⚠️ Note: pixel **corners** are used only for the separate 2160p checkerboard
technique, not the 1080p TAA.)

#### 3C3. Two-pass resolve

**Pass 1 — FXAA** on the jittered frame (samples sit at pixel edges).

**Pass 2 — sharpen + reproject + blend** (single shader does both):
- Reads the **other** history buffer (previous frame's *sharpened* output)
- Rejects or reprojects
- On accept: blend 50/50 with current sharpened frame → backbuffer
- On reject: output current frame **unsharpened** (to hide aliasing where
  history is invalid)

```hlsl
// Pseudocode (reconstructed from pp.42-43):
sharpenedCurrent = Sharpen(currentFrame, localContrast);
write(sharpenedCurrent, historyBufferA);          // ping
historySample    = read(historyBufferB);            // pong (previous sharpened)

if (ReprojectAndAccept(historySample, velocity, depth)) {
    output = lerp(sharpenedCurrent, historySample, 0.5);   // 4 effective samples
} else {
    output = currentFrame;                          // unsharpened, raw
}
write(output, backbuffer);
swap(historyBufferA, historyBufferB);
```

#### 3C4. Sharpening kernel

4-tap resampling with negative lobes (Catmull-Rom-like but cheaper), with
**local-contrast-driven sharpening amount**:

> *"We use a 4-tap resampling kernel with negative lobes to counter any
> texture blurring. This is a bit like Catmull-Rom resampling, but cheaper,
> and the amount of sharpening is determined based on local contrast."*

#### 3C5. Effective sample count

- History accepted: **4 samples/pixel** (2 from current, 2 from previous,
  both edge-sampled)
- History rejected: **2 samples/pixel** (current frame, already FXAA'd)
- Pattern ≈ patented **FLIPQUAD** (but sharper — samples closer to center;
  no MSAA hardware needed; correct mip sampling for free)

#### 3C6. Budget

**FXAA + TAA + UI compositing ≤ 1 ms/frame on PS4 @ 1080p.**

#### 3C7. YCoCg for FXAA luminance (checkerboard path)

For the 2160p checkerboard variant, FXAA runs in **YCoCg space** so it can
gather 4 luminance values per texture gather (FXAA operates on luminance):

> *"The YCoCg tangram is sampled by the FXAA pass... the reason we use YCoCg
> is because FXAA does most of its work on luminance data, and this allows us
> to sample 4 luminance values per texture gather."*

#### 3C8. What the paper does NOT provide

Explicitly deferred: *"For other aspects of our TAA solution like temporal
reprojection and neighborhood-based rejection criteria, we used some more
traditional approaches, so we won't cover those here."* → Use Intel TAA's
variance clipping or k-DOP for the rejection step.

---

## 4. Implementation priority for afterglow-engine

From these extractions, the highest-value formulas to port to WGSL:

1. **§3B4 — 1-sample spatio-temporal bicubic**
   (`m03 = x*(0.8x-0.8); filteredColor = (m03*lerp(left,right,x) + historyColor)/(m03+1)`)
   — 5× cheaper than full bicubic, sharper, and the neighborhood taps are
   already needed for the clamp. Drop-in for the Intel TAA history filter.
2. **§3A3 — 5-sample Catmull-Rom history filter** — if you want the classic
   sharp filter (fallback when §3B4's edge aliasing is unacceptable).
3. **§3B3 — Differential blend** — if you want Decima-style 2-frame
   reconstruction with spatial fallback.
4. **§3A4 + §3A5 — Spatial contrast tracking + FPS-aware exponential blend**
   — the anti-flicker + framerate-independent convergence, drop into any TAA
   resolve.
5. **§3B5 — Velocity-aware jitter** — cheap quality win for in-motion
   stability.
6. **§3A7/§3A8 — Mip bias / UV unjitter** — eliminates jitter-induced
   texture blur for free.
7. **§3C1–§3C5 — Decima 2-frame architecture** — the zero-smear structural
   decision (raw-input history, 50/50 blend, unsharpened-on-reject).

The single most portable, high-impact artifact is **§3B4's 3-line
spatio-temporal bicubic** — pure math, 3 lines of WGSL, replaces the most
expensive part of any TAA resolve.

---

## 5. Cross-check provenance

Three rounds of cross-checking were performed. Key corrections found and
applied:

- **Decima "samples from pixel corners"** — WRONG. The 1080p TAA samples
  from pixel **edges** (horizontally/vertically between centers). Corners
  are the separate 2160p checkerboard technique. Corrected in §3C2.
- **k-DOP attribution to "NVIDIA et al."** — WRONG. Authors are Julius
  Ikkala, Tuomas Lauttia, Pekka Jääskeläinen, Markku Mäkitalo (Tampere
  University, Finland). Corrected in §1.
- **id Tech "ridiculously sharp" quote attributed to Digital Foundry** —
  WRONG. It is a ResetEra forum anecdote. The Simon Coenen DOOM Eternal
  study's "customized velocity buffer... little ghosting... gun excluded"
  describes **motion blur**, not TAA. id Tech TAA downgraded to Soft evidence.
- **DLSS 4 Transformer "zero smear"** — OVERSTATED. GamersNexus found
  counterexamples (sword smear); DF noted volumetric fog ghosting in 4.0
  (fixed in 4.5). Presented with caveats in §1.
- **FSR 3 Native AA "crisper than DLAA"** — was a single Steam forum
  anecdote, not consensus. Downgraded to Tier 2 / contested.

Primary sources verified directly (not via search snippets):
- Intel TAA README (`gh api repos/GameTechDev/TAA/readme`)
- Decima SIGGRAPH 2017 paper (full 70-page PDF fetched)
- Digital Foundry TAA article (full text fetched)
- k-DOP Clipping SIGGRAPH Asia 2024 Technical Communications page
- NVIDIA Survey of TAA (Yang, Liu, Salvi 2020) — Wiley + NVIDIA Research +
  Semantic Scholar
- Filmic SMAA v7 PPTX (143 slides fetched)
- Dynamic TAA in CoD PDF (122 slides fetched)
- Alex Tardif "Failed Adventure in Avoiding TAA" (full text fetched)
- Simon Coenen DOOM Eternal Graphics Study (full text fetched)

Sources for the cross-check itself are recorded inline above; this doc is the
canonical record of the decision.
