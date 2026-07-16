# Proven Working Implementations — PPOM-RCS reference catalog

Working, source-available implementations of the **Prism POM + cone-step
relief** family (and its components), gathered for implementation reference.
Primary shader sources saved in `impl-references/`. Ranked by relevance to
afterglow-engine (WebGPU / Three.js).

---

## ⭐ Tier 1 — directly usable for afterglow-engine

### 1. SkyeShark/threejs-silhouette-pom  (Three.js WebGPU / TSL)
- **What:** Silhouette POM for three.js **WebGPU/TSL**. Single TSL function
  `parallaxOcclusionUV()` ray-marches a height map and **clips the silhouette
  through the alpha-test / alpha-to-coverage path**, so relief overhangs the
  geometry. Includes **self-shadowing** (`pom.shadow(lightDir)`), curved-
  surface silhouettes, horizon trimming, inflated-shell overhang, and
  gradient-safe sampling at the marched UV.
- **Why it's #1:** It is literally the engine's stack (Three.js WebGPU/TSL),
  MIT-licensed, with a live demo, updated 2026-07-14. The silhouette approach
  (coverage → `opacityNode` + `alphaTestNode` + `alphaToCoverage`) is the
  cheapest correct-silhouette method on a raster path — no prism geometry
  needed, it carves the outline via alpha test.
- **File:** `impl-references/SkyeShark-threejs-silhouette-pom-ParallaxOcclusion.js`
- **Repo:** https://github.com/SkyeShark/threejs-silhouette-pom
- **Live demo:** https://skyeshark.github.io/threejs-silhouette-pom/
- **License:** MIT.
- **Use as:** the primary template. Add cone-step acceleration (Tier 2 below)
  to cut the march cost.

---

## Tier 2 — the cone-step / relief acceleration (the "RCS" half)

### 2. GPU Gems 3 CD — `ReliefMapping.fx`  (canonical reference, HLSL)
- **What:** The **official reference implementation** of GPU Gems 3 ch.18.
  Contains three complete pixel shaders in one file:
  - `ray_intersect_relief()` — linear (15 steps) + binary (6 steps) search.
  - `ray_intersect_relaxedcone()` — **relaxed cone stepping (15) + binary (8)**,
    the exact algorithm we want.
  - `setup_ray()` with the **depth-bias** trick (`db=1-(1-(1-v.z)²)²`) that
    suppresses grazing-angle swimming, and border-clamp alpha.
  - Plus the full VS (tangent-space view/light dir) + normal-mapping PS.
- **Why:** Cleanest, most-copied formulation of the cone-step march. The
  cone-map layout it assumes (height in alpha `.w`, cone ratio in blue `.z`)
  is the de-facto standard.
- **File:** `impl-references/GPUGems3-CD-ReliefMapping.fx`
- **Repo:** https://github.com/QianMo/GPU-Gems-Book-Source-Code (`GPU-Gems-3-CD-Content/content/18/demo/{xna,fxcomposer}/`)
- **License:** NVIDIA GPU Gems (free, attribution).
- **Use as:** port `ray_intersect_relaxedcone` to WGSL verbatim; it's ~20 lines.

### 3. Bundas102/robust-cone-map  (Falcor 7 / DX12 / Slang, 2024 paper)
- **What:** Reference impl of **Robust Cone Step Mapping** (EGSR 2024,
  Bán et al.) — fixes the bilinear-interpolation & relaxed-cone correctness
  bugs of classic CSM. Built on Falcor 7.0 (DX12). Includes a full ladder:
  bump / parallax / linear / **cone step** / **QDM (quadtree displacement)** /
  **Seidel MaxMip tracing**, plus `Refinement.slang` and
  `IntersectBilinearPatch.slang` (Reshetov "cool patches" — the exact
  ray/bilinear-patch routine for prism sides).
- **Why:** The modern, correctness-fixed cone step. The `CONSERVATIVE_STEP`
  cell-border stepping option is the robust variant. The bilinear-patch
  shader is the prism-silhouette side-intersection ready to use.
- **Files:** `impl-references/Bundas102-robust-cone-map-FindIntersection.slang`,
  `...-Refinement.slang`, `...-IntersectBilinearPatch.slang`
- **Repo:** https://github.com/Bundas102/robust-cone-map  (+ predecessor
  `Bundas102/falcor-conemap`, EG 2022 "Quick cone map generation on the GPU")
- **Paper:** https://diglib.eg.org/handle/10.2312/sr20241146
- **License:** Falcor (BSD-ish) + paper code.
- **Use as:** reference for (a) the robust cone-step march, (b) the
  ray/bilinear-patch prism-side intersection, (c) QDM/MaxMip as future
  acceleration beyond cone maps.

### 4. bbalazs2002/ConeStepMapping  (modern C++/OpenGL 4.3, GLSL)
- **What:** A custom modern OpenGL framework implementing **prism + cone-step
  mapping together** — exactly the PPOM-RCS combination. `Glsl_common.glsl`
  has:
  - `intersectUnitPrism()` — unit-prism ray intersection (near/far t) →
    **entry/exit for the silhouette march**.
  - `findIntersection_coneStepMapping()` — t-parameterized cone-step march
    inside the prism, with hit/miss flags (max-steps / past-prism-exit /
    cone-collapsed).
  - `getNextStep()` — the cone-intersection math (tangent-of-cone form).
- **Why:** This is the most complete open **prism + cone-step** reference —
  the silhouette (prism exit → miss → discard) logic is all there in GLSL,
  close to WGSL.
- **File:** `impl-references/bbalazs-ConeStepMapping-Glsl_common.glsl`
- **Repo:** https://github.com/bbalazs2002/ConeStepMapping (+ `..._thesis`)
- **License:** check repo (academic thesis project).
- **Use as:** the prism-silhouette + cone-step glue logic; port the prism
  intersection and the march-terminates-on-prism-exit handling.

---

## Tier 3 — POM + self-shadowing (the "POM" half, concrete shaders)

### 5. iamyoukou/normalMapping  (C++/OpenGL, GLSL)
- **What:** POM with **self-shadowing** sample. `fsPOM.glsl` has
  `calcShadow()` — marches a light ray through the heightfield, soft shadow
  via per-step `(currLayerDepth - depth) / layerDepth` falloff. Adaptive
  layer count by light angle.
- **File:** `impl-references/iamyoukou-fsPOM-selfshadow.glsl`
- **Repo:** https://github.com/iamyoukou/normalMapping
- **License:** check repo.

### 6. piellardj/parallax-mapping  (C++/OpenGL, GLSL)
- **What:** POM with steep-parallax + **linear-interp refine + soft
  self-shadow**. Cited by iamyoukou as the "better effect" soft shadow.
  Clean `parallax.frag` with the full march + shadow in ~140 lines.
- **File:** `impl-references/piellardj-parallax-softshadow.frag`
- **Repo:** https://github.com/piellardj/parallax-mapping
- **License:** check repo.

### 7. DOWNPOURDIGITAL/glsl-parallax-occlusion-mapping  (glslify, GLSL)
- **What:** POM as a reusable **glslify** function — the most minimal,
  dependency-free POM march you can find. Good for a from-scratch WGSL port.
- **Repo:** https://github.com/DOWNPOURDIGITAL/glsl-parallax-occlusion-mapping

### 8. amsXYZ/POM  (POM + self-occlusion shader)
- **Repo:** https://github.com/amsXYZ/POM

### 9. LearnOpenGL POM  (GLSL, tutorial)
- **What:** The canonical steep-parallax → POM-with-linear-interp tutorial
  shader (saved earlier as `learnopengl-parallax-shader.txt` in parent dir).
  Adaptive `numLayers = mix(maxLayers, minLayers, N·V)`.
- **URL:** https://learnopengl.com/Advanced-Lighting/Parallax-Mapping

---

## Tier 4 — cone-map / acceleration-structure generators

You need these to **precompute** the cone map (one-time, per height texture —
ideal for the `afterglow-assets-worker`).

### 10. Ryan-DowlingSoka/ReliefMapping  (Unreal Engine 5 plugin)
- **What:** UE5 plugin that **generates Relaxed Cone Step Maps** in-engine.
  Includes `Shaders/reliefmapping.ush` with `ray_intersect_relaxedcone()`
  (same lineage as the GPU Gems FX).
- **Repo:** https://github.com/Ryan-DowlingSoka/ReliefMapping
- **Use as:** reference for the cone-map **generation** step (the GPU Gems 3
  `depth2relaxedcone` precompute shader, Listing 18-2, is also in
  `gpu-gems3-ch18-relaxed-cone-stepping.txt` in the parent dir).

### 11. tomosud/RelaxedConeMap  (UE generator)
- **Repo:** https://github.com/tomosud/RelaxedConeMap

### 12. "Get Relief!"  (UE Marketplace, open-source CC-BY)
- Relaxed Cone Step Map generator, free + open. Good reference for the
  authoring/generation UX.
- **URL:** https://www.unrealengine.com/marketplace/en-US/product/get-relief-rcsm-generator

### 13. IrrlichtBAW  (buildaworldnet) — RCSM issue + impl
- Engine-level RCSM integration discussion (quad-channel vs single-channel
  cone map, binary search toggle). Useful for the integration-level decisions.
- **Repo/issue:** https://github.com/buildaworldnet/IrrlichtBAW/issues/226

---

## Tier 5 — related / adjacent references

### 14. Rabbid76/graphics-snippets  (curated link list + code)
- `documentation/normal_parallax_relief.md` — the best curated bibliography of
  normal/parallax/relief/cone-step implementations with working links.
- **URL:** https://github.com/Rabbid76/graphics-snippets/blob/master/documentation/normal_parallax_relief.md

### 15. Shintaro Iguchi — Relaxed Cone Stepping blog
- Working RCS implementation writeup after reading GPU Gems 3 ch.18.
- **URL:** https://shintaro-iguchi.com/2016/01/03/relaxed-cone-stepping/

### 16. Raouf Bejaoui — (Anisotropic) Cone Step Mapping (ArtStation)
- Production anisotropic CSM implementation notes (Splash Damage); links the
  Chen & Chuang 2009 anisotropic paper. Relevant if walls need anisotropic
  relief.
- **URL:** https://www.artstation.com/artwork/o2wa3q

### 17. ARCSM — Axiom VFX (2025, current raster SOTA claim)
- Anisotropic Relaxed Cone Step Mapping; "thousands of POM steps fidelity at
  near-normal-map cost." Custom HLSL + precomputed accel structure. Closed
  source as far as can be found — contact for licensing.
- **URL:** https://www.axiomvfx.com/arcsm

---

## How to combine these for the afterglow-engine impl

1. **Base silhouette POM** → port `SkyeShark/threejs-silhouette-pom` TSL
   function (it already targets our stack, MIT). This alone gives silhouette
   + self-shadow on WebGPU today.
2. **Cut the march cost** → swap the fixed-step march for the relaxed-cone
   march from `GPUGems3-CD-ReliefMapping.fx::ray_intersect_relaxedcone`
   (~20 lines). Keep SkyeShark's coverage/silhouette logic; only the interior
   loop changes.
3. **Cone-map generation** → run the GPU Gems 3 `depth2relaxedcone` precompute
   shader (in `gpu-gems3-ch18-relaxed-cone-stepping.txt`) inside the
   `afterglow-assets-worker`; or reference Ryan-DowlingSoka's UE generator for
   the generation pipeline.
4. **Prism-side correctness (optional, harder silhouette)** → if you want true
   geometric prism extrusion instead of alpha-test silhouette, port
   `bbalazs/ConeStepMapping::intersectUnitPrism` + the robust
   `IntersectBilinearPatch.slang` from Bundas102.
5. **Robustness hardening** → adopt the `CONSERVATIVE_STEP` cell-border
   stepping and the bilinear-safe cone generation from the Bundas102 2024
   paper to avoid the classic CSM artifacts.
6. **Self-shadow** → either keep SkyeShark's built-in `pom.shadow()`, or port
   piellardj's/iamyoukou's soft-shadow light-ray march.

### License summary for direct porting
| Source | License | Port-friendly? |
|--------|---------|----------------|
| SkyeShark/threejs-silhouette-pom | MIT | ✅ direct |
| GPU Gems 3 ch.18 code | NVIDIA free (attribution) | ✅ port |
| Bundas102/robust-cone-map | Falcor BSD + paper | ✅ reference |
| bbalazs/ConeStepMapping | academic (verify) | ⚠️ reference |
| iamyoukou, piellardj, DOWNPOURDIGITAL | verify per-repo | ⚠️ reference |

> Verify each repo's LICENSE before copying code verbatim into the engine;
> the MIT (SkyeShark) and NVIDIA GPU Gems (attribution) ones are the safe
> direct-port sources.
