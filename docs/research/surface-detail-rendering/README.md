# Surface Detail — Making Flat Walls Not Look Flat (SOTA Survey)

Research dossier: every technique game engines / renderers use to add
geometric-looking detail to a flat polygon (a wall), from cheapest per-pixel
tricks to SOTA ray-traced displacement. Compiled 2026-07. See `INDEX.md` and
the per-source `.md` files in this directory for primary-source evidence.

---

## TL;DR ladder (cheap → expensive, raster → ray traced)

| Tier | Technique | Real geometry? | Silhouette correct? | Self-shadow / refl? | Cost |
|------|-----------|:--:|:--:|:--:|------|
| 0 | **Bump / Normal mapping** | no | no | no | ~free (1 sample) |
| 1 | **Parallax (offset) mapping** | no | no | no | 1 sample |
| 2 | **Steep parallax** | no | no | partial | ∝ step count |
| 3 | **Parallax Occlusion Mapping (POM)** | no | no | yes (self) | ∝ step count |
| 4 | **Relief mapping** (binary search) | no | no | yes | > POM |
| 5 | **Cone Step Mapping (CSM) / Relaxed (RCS) / ARCSM** | no | no | yes | << POM via leap |
| 6 | **Per-pixel distance functions** (3D tex) | no | no | yes | 1-ish step + 3D tex |
| 7 | **Prism POM / Shell mapping** | no | **yes** (silhouette) | yes | raster + prism ray |
| 8 | **Hardware tessellation + displacement** | **yes** | yes | yes (real) | vertex amp + mem |
| 9 | **Mesh shaders / micro-tessellation** | yes | yes | yes | modern API |
| 10 | **Nanite / virtualized cluster geometry** | yes (baked) | yes | yes | cluster LOD streaming |
| 11 | **Ray-traced displacement** (D-BVH, RMIP, nonlinear, PDM, Micro-Mesh) | yes | yes | yes (true) | RT core / BVH |

---

## 1. Per-pixel shading tricks (no geometry — the "fake depth" family)

All operate on a height/depth texture in the fragment shader. **They never
modify geometry**, so the polygon silhouette stays flat and they cast no real
shadows onto neighbors. Cheapest and most common.

- **Bump mapping (Blinn 1978) / Normal mapping**: perturb the shading normal
  only. Industry baseline. Zero parallax → looks flat at grazing angles.
- **Parallax / offset mapping (Kaneko 2001)**: shift UVs by a single height
  sample scaled by view angle. Cheap; "swims" and breaks at grazing angles.
  Offset-limiting clamps the worst artifacts.
- **Steep parallax**: sample the heightfield in N layers along the view ray,
  take the first below-surface layer. Gives self-occlusion; visible banding.
- **Parallax Occlusion Mapping / POM (Tatarchuk 2005)**: steep parallax +
  linear interpolation between the last two samples for sub-step accuracy +
  optional self-shadowing rays. **The industry workhorse** — Unreal
  `BumpOffset`, Unity HDRP `Parallax`, CryEngine. Cost scales with step count
  (tens of samples).
- **Relief mapping (Oliveira 2000, Policarpo 2007)**: iterative ray/heightfield
  intersection with binary-search refinement. Higher quality than POM, higher
  cost.
- **Cone Step Mapping (Donnelly & Wloka 2005 / Dummer 2006)**: precompute a
  cone map (per-texel max slope) and **space-leap** along the ray
  conservatively — far fewer steps than fixed-step POM. Heightfield-only (no
  overhangs).
- **Relaxed Cone Stepping (GPU Gems 3 ch.18)**: relaxes cones so rays may
  *enter* but not leave the relief → renders **non-height-field structures**
  (overhangs, multiple layers, impostors).
- **Anisotropic Cone Step Mapping (Chen & Chuang 2009)**: two-direction cones
  for anisotropic surfaces.
- **ARCSM (Axiom VFX, 2025)** — current raster SOTA claim: precomputed accel
  structure + custom HLSL, "fidelity of thousands of POM steps at almost the
  cost of a normal map."
- **Robust Cone Step Mapping (2024)**: fixes bilinear-interpolation & relaxed-
  cone correctness bugs in classic CSM.
- **Per-pixel displacement with distance functions (GPU Gems 2 ch.8)**: store
  a 3D distance-field texture; near single-step intersection. Needs a 3D tex.
- **View-dependent displacement mapping (Wang 2003)**: SVD-precomputed.
- **NeuMIP (Kuznetsov 2021)**: neural multiresolution materials.

## 2. Silhouette-correct raster (still no real geometry, but the *edge* looks right)

The big visible flaw of tier 1 is that the wall's silhouette is a straight
line. These fix that by extruding a prism/shell around each triangle and
ray-marching inside it.

- **Prism Parallax Occlusion Mapping / PPOM (Dachsbacher & Tatarchuk 2007)**:
  extrude prisms, intersect the view ray with the prism (split into tetrahedra)
  → correct silhouettes.
- **Shell / curved shell mapping (Porumbescu 2005; Jeschke 2007)**: bijective
  map between texture and "shell space" (the volume between two offset
  surfaces) to render meso-geometry; curved version removes discontinuities via
  Coons patches (solves a cubic per step).

## 3. Real geometry generation (raster pipeline)

Actually move vertices / create micro-triangles. Correct silhouettes, real
shadows, real reflections — at the cost of vertex amplification and memory.

- **Displacement + hardware tessellation (Cook 1984; DX11 tessellation 2009;
  Nießner & Loop 2013 analytic)**: adaptively tessellate the base mesh and
  displace vertices by the height map. Classic, broadly supported (DX11/GL).
- **Mesh shaders / micro-tessellation**: modern (DX12 Ultimate / Vulkan)
  programmable primitive generation — adaptive micro-triangles on-GPU.
- **Nanite (UE5, Karis 2021)**: virtualized geometry — source geometry
  compressed into triangle **cluster LODs** streamed/rendered on the fly. The
  SOTA answer to "detailed walls without paying per-poly." Displacement is
  **baked at import** (no runtime displacement maps). Cluster-LOD + software
  rasterization for tiny triangles.

## 4. Ray-traced displacement (true self-occlusion, shadows, reflections, silhouettes)

The SOTA for physically correct detail. Uses RT cores / BVHs.

- **D-BVH / Tessellation-Free Displacement (Thonat 2021)**: min-max mipmap
  over affine intervals of the base geometry; per-pixel ray tracing through an
  image pyramid. Real-time, artifact-free, no tessellation.
- **RMIP (Thonat 2023)**: displacement RT via inversion + oblong bounding;
  bidirectional 2D↔3D mapping, anisotropic acceleration. Needs BLAS rebuild on
  edit.
- **Nonlinear Ray Tracing (Ogaki SA2023)**: cubic-equation mapping of world
  rays into canonical prism volumes; broad-phase BVH + min-max mipmaps.
- **Micro-Mesh / Displacement Micro-Map (NVIDIA 2023)**: hardware RT
  primitive (`VK_NV_displacement_micromap`, RTX 40). Barycentric-compressed
  micro-triangles, displacement from a depth texture. Primarily a **ray-tracing
  compression** mechanism for detailed static assets.
- **Locally-adaptive LOD RT (Haydel 2023)**: per-triangle 1-to-4 subdivision
  tree, per-triangle LOD. **Benthin & Peters 2023**: Nanite clusters inserted
  into a dynamic GPU BVH per frame for RT of micro-poly geometry.
- **Projective Displacement Mapping / PDM (Hoetzlein 2025)** — newest: direct
  sampling, **no BLAS**, hardware TLAS BVH over prisms only. Parallel-offset
  prisms (linear world→texture projection), ray/bilinear-patch sides, smoothed
  displaced normals, stochastic thin-feature sampling. Editing the height
  texture needs **zero** rebuild. 15k tris + 1.8 MB → detailed RT scene.

---

## What shipping engines actually do (2025)

- **Unreal 5**: POM (`BumpOffset` material node), limited hardware
  tessellation, **Nanite** (baked displacement, no runtime displacement maps),
  RTX Micro-Mesh via toolkit. Runtime POM is the "flat wall with depth" answer.
- **Unity HDRP**: Parallax/POM in the Lit shader, DX11 tessellation option. No
  Nanite equivalent.
- **Three.js / WebGPU**: **nothing built-in** — you write POM/relief/cone-step
  in a fragment shader yourself.

---

## Viability for a WebGPU engine (afterglow-engine context)

| Technique | WebGPU viable? | Notes |
|-----------|:--:|------|
| Normal / Parallax / POM / Relief / Cone-step / ARCSM | ✅ yes | Pure fragment-shader work; trivially portable. **This is the realistic SOTA tier for the web.** |
| Per-pixel distance functions (3D tex) | ✅ yes | WebGPU has 3D textures. |
| Prism POM / Shell mapping | ✅ yes | Fragment shader + prism geometry; just more math. |
| Hardware tessellation + displacement | ❌ no | **WebGPU has no fixed-function tessellation stage.** Must emulate via compute-pass vertex generation or instanced quads. |
| Mesh shaders | ❌ no | Not in WebGPU core (proposal stage). |
| Nanite-style cluster LOD | ⚠️ software only | Fully implementable in compute + software rasterization, but heavy. No hardware assist. |
| Ray-traced displacement (D-BVH/RMIP/PDM/Micro-Mesh) | ❌ no HW RT | WebGPU has **no hardware ray tracing API** yet (compute-side software RT possible but slow). Micro-Mesh is NVIDIA-only HW. |

**Practical conclusion for a WebGPU engine:** the per-pixel family (POM →
relief → cone-step / ARCSM) is the entire realistic surface-detail toolbox
today, optionally augmented with silhouette-correct prism/shell mapping. For
*real* geometric detail you would hand-roll a compute-based adaptive
tessellation or a Nanite-style cluster-LOD system in software. Hardware
ray-traced displacement is off the table until WebGPU ships an RT extension.

---

## Recommendations / selection guide

- **Default for walls:** POM with offset-limiting and ~16–32 steps; add
  self-shadowing rays only for hero assets. Best cost/quality on web.
- **When you need overhangs / layered detail (brick gaps, ivy, grates):**
  relaxed cone stepping (RCS) or ARCSM if you can afford the precompute.
- **When the flat silhouette reads as a bug (close-up, side view):** prism/shell
  mapping, or accept the cost of real tessellation done in compute.
- **When you have a baked high-poly sculpt and want infinite detail without
  per-poly cost:** a software Nanite-style cluster-LOD system (large effort).
- **Desktop/console with RTX:** Micro-Mesh (compression) or PDM (editing) are
  the ray-traced SOTA — not reachable from WebGPU today.
