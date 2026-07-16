# Deep Dive: Prism POM with Relaxed Cone Stepping (PPOM-RCS)

The recommended technique for **correct silhouette + high-quality interior
parallax + self-shadowing on an iGPU budget.** This document is the
implementation-grade spec: math, shader pseudocode, precompute, integration,
performance, artifacts, and WebGPU notes.

**Lineage / citations:**
- Prism silhouette: Dachsbacher & Tatarchuk 2007, *Prism Parallax Occlusion
  Mapping with Accurate Silhouette Generation* (INRIA HAL inria-00606806).
- POM interior + self-shadowing: Tatarchuk 2006, *Dynamic Parallax Occlusion
  with Approximate Soft Shadows* (I3D 2006); Brawley & Tatarchuk 2004,
  *Parallax Occlusion Mapping: Self-Shadowing, Perspective, …*
- Cone-step acceleration: Dummer 2006, *Cone Step Mapping*; Donnelly 2005,
  *Per-Pixel Displacement Mapping with Distance Functions* (GPU Gems 2 ch.8).
- Relaxed cone stepping + binary-search refine: Policarpo & Oliveira, GPU Gems
  3 ch.18, *Relaxed Cone Stepping for Relief Mapping*.
- Ray/bilinear-patch prism sides: Ramsey, Potter & Hansen 2004; Reshetov 2019
  ("cool patches"). Tetrahedron prism split: Hirche et al. 2004.

Primary sources saved alongside this file:
`prism-pom-dachsbacher-tatarchuk-2007.md`,
`gpu-gems3-ch18-relaxed-cone-stepping.txt`,
`learnopengl-parallax-shader.txt`, `displacement-review-blekinge-2012.txt`.

---

## 1. Why this combination

The three constraints:

1. **iGPU-fast** → must stay in the fragment shader, no geometry amplification,
   no BVH, no RT cores. ⇒ per-pixel family.
2. **Correct silhouette** ⇒ must ray-march inside an *extruded volume*, not a
   flat texture plane. ⇒ prism/shell, *not* plain POM.
3. **Looks really good** ⇒ interior quality must be relief-grade (correct
   self-occlusion, sub-sample accuracy, self-shadowing), and the march must be
   cheap enough to afford that quality. ⇒ cone-step space-leaping + binary
   search, *not* fixed-step POM.

PPOM-RCS = **Prism** (silhouette) + **Relaxed Cone Stepping** (cheap march) +
**binary search** (accuracy) + **light-ray self-shadow** (depth sell).

| Property | Plain POM | PPOM-RCS |
|----------|-----------|----------|
| Silhouette at grazing angles | straight line (flat poly) | **deforms correctly** |
| Interior march cost | N fixed steps (10–50) | ~15 cone leaps + 6 binary |
| Sub-sample accuracy | linear interp of 2 samples | binary-search refine |
| Self-occlusion | yes | yes |
| Self-shadowing | yes (add a light ray) | yes (cone-accelerated) |
| Casts shadows on neighbors | no | no (needs real geo / RT) |
| Reflections see it | no | no (needs RT) |

---

## 2. Data & precompute

Per material you need a **height map** `H(u,v)` (R8 or R16, normalized [0,1],
0 = surface, 1 = deepest — or the opposite; pick a convention and be
consistent; LearnOpenGL/GPU Gems use *stored depth* where the ray is "inside"
when `ray.z > storedDepth`).

From the height map, **offline precompute** (one-time, per texture; store as
extra channels of the same texture or a companion texture):

### 2.1 Relaxed cone map `C(u,v)` (one channel, [0,1])
Per texel `t_i`, the largest cone ratio (width/height) such that **any viewing
ray traveling inside the cone pierces the heightfield at most once.** Larger =
faster convergence. Pseudocode (GPU Gems 3, Listing 18-1):

```
for each source texel ti:
    radius_C(i) = 1.0                       # clamp to 1.0 for 8-bit storage
    src = (ti.uv, 0.0)
    for each destination texel tj:
        dst = (tj.uv, H(tj))
        ray.origin = dst;  ray.dir = dst - src
        (k,w) = next_intersection(tj, ray, H)   # 2nd hit along the ray
        d = H(k,w)
        if d - H(ti) > 0:                       # dst must be above src
            ratio(i,j) = |src.xy - dst.xy| / (d - H(tj))
            radius_C(i) = min(radius_C(i), ratio(i,j))
    C(ti) = radius_C(i)
```

A GPU preprocess shader (GPU Gems 3 Listing 18-2) does this in `search_steps`
(~128) per texel; O(n²·search_steps) but runs once. Output: a single-channel
cone map. Store it in, e.g., the blue channel; height in alpha.

### 2.2 (Optional) Max-mip pyramid for horizon AO/shadow
Instead of averaging when downsampling, store the **maximum** height per 2×2.
Used by the horizon approximation (§5.2) to cheaply skip small-scale distant
detail. Standard `generateMipmap` won't do this — build it manually with a max
reduction.

### 2.3 Geometry: extrude prisms (CPU/mesh-preprocess)
For each base triangle with vertices `V0,V1,V2` and per-vertex normals
`N0,N1,N2`, build a **prism** = the triangle extruded along the (interpolated)
normal by `±heightScale`:

```
top_i    = V_i + heightScale * N_i      # max displacement extent
bottom_i = V_i - heightScale * N_i      # (or 0 if displacement is one-sided)
```

The displaced surface `S(u,v) = P(u,v) + D(u,v)·N'(u,v)` lives **inside** this
prism so long as `D(u,v) ≤ heightScale`.

**Prism faces:** 2 triangles (top, bottom) + 3 side **slabs**, each a bilinear
patch (because adjacent vertex normals aren't coplanar). Two rasterization
options:

- **Tetrahedra (Hirche 2004):** split each prism into 3 tetrahedra by
  consistently choosing the slab diagonal so adjacent prisms share the same
  diagonal (no gaps, no overlap). Simplest; per-tetrahedron texture gradient is
  constant. Good default.
- **Direct ray/bilinear patch (Dachsbacher–Tatarchuk / Reshetov):** rasterize
  each slab as 2 triangles with the diagonal chosen so the triangle normal
  points *outward*, then compute the true ray/bilinear-patch intersection per
  pixel. Higher quality (C¹-ish at edges, no "buckling"); more math.

> **PDM note:** Hoetzlein 2025 shows that making the offset triangles *parallel*
> to the base (scale each normal by `1/(N_i·N_g)`) makes the world→texture
> projection **linear**, avoiding per-sample tangent transforms. Worth adopting
> here too — it simplifies the march and removes a class of artifacts.

---

## 3. The view-ray march (interior)

Goal: given the fragment's tangent-space view direction `V` and entry UV
`(s,t)`, find where the view ray pierces the displaced surface.

**Step 0 — scale the ray** so `V.z = 1` (GPU Gems 3, Eq. 1–4):
```
ray_dir = V / V.z
ray_ratio = |ray_dir.xy|          # |d(uv)/dz|, the parallax slope
```

**Step 1 — relaxed cone space-leap** (fixed `cone_steps`, e.g. 15):
```
pos = (s, t, 0)                   # ray position in (uv, depth) space
for i in 0..cone_steps:
    tex = sample(reliefMap, pos.xy)
    cone_ratio = tex.b            # C(u,v)
    height = saturate(tex.a - pos.z)   # gap to surface (clamped ≥0)
    d = cone_ratio * height / (ray_ratio + cone_ratio)
    pos += ray_dir * d
```
Each leap advances `d` along the ray — the conservative distance to the cone's
surface in the ray's direction. `saturate` stops on the first texel where the
ray is **under** the relief (i.e. `pos.z > H`). After this loop the ray has
pierced the surface **at most once** (the relaxed-cone guarantee).

**Step 2 — binary-search refine** (fixed `binary_steps`, e.g. 6):
```
range = 0.5 * ray_dir * pos.z
p = entry + range                # midpoint of [entry-stop, stop]
for i in 0..binary_steps:
    tex = sample(reliefMap, p.xy)
    range *= 0.5
    if p.z < tex.a:              # outside (above) surface → go forward
        p += range
    else:                        # inside (below) surface → go back
        p -= range
finalUV = p.xy
```
6 binary steps → 1/64 texel accuracy. Final intersection UV = `p.xy`.

**Why not POM's linear-interp trick instead of binary search?** Binary search
is more accurate and, after cone stepping, only 6 samples. POM's two-sample
linear interp is cheaper per-step but needs many more linear steps to get
there; RCS already did the leaping. (You *can* hybridize: do a few cone steps,
then a couple linear steps, then 4 binary — tune to the iGPU.)

### Texture-space vs prism-space march
The march above is in **2D texture space + 1D depth** (the classic relief
formulation). The prism only changes **entry/exit**: the view ray enters the
prism at a side slab (or the top face) rather than at the base triangle. You
compute the entry point `s0` and its UV by intersecting the view ray with the
prism's bilinear-patch sides (or tetrahedron faces), then run the same 2D
march. **If the ray exits a side before hitting the surface, you get a correct
silhouette hit on the side** — that's the whole point.

---

## 4. Silhouette: the prism intersection

This is what plain POM lacks. For each fragment of a *rendered slab face*
(rasterize the 3 side slabs + top face of each prism):

1. Compute the **entry point** `s0` and entry UV by ray/triangle (top/bottom
   face) or ray/bilinear-patch (slab side) intersection with the view ray.
2. Compute **exit t-value** `t_max` the same way.
3. Run the cone/binary march (§3) from `s0`. If the march finds a surface hit
   before `t_max`, shade it. **If no hit before `t_max`, the ray exits through
   a side → shade the side hit** — this is the correct silhouette, including
   overhangs if your height map has them and you use relaxed cones (which allow
   rays to enter and not leave).
4. Compute the shading normal at the hit from the height-map gradient (finite
   differences in UV) + the base interpolated normal `N'`.

**Watertightness:** adjacent prisms must share the same slab diagonal (tetra-
hedron approach) or use the consistent outward-facing diagonal so there are no
gaps/overlaps between neighbors. Hirche 2004 gives the consistent triangulation
rule.

**Early rejection:** during vertex processing, cull prisms whose view ray can't
possibly intersect (backface top face + ray above max extent, etc.) — avoids
wasted fragment work, important on iGPUs.

---

## 5. Self-shadowing & ambient occlusion

### 5.1 Hard/soft self-shadow (light ray)
From the surface hit point `P` (UV `(pu,pv)`, depth `ph`), trace a ray toward
the light in tangent space `L`:

```
lray = L / L.z                    # light direction in (uv, depth) space
lpos = (pu, pv, ph)
shadow = 0
for k in 0..shadowSteps:
    lpos += lray * step
    h = sample(reliefMap, lpos.xy).a
    if h < lpos.z:                # surface is above the light ray → blocked
        # hard: shadow = 1; break
        # soft: shadow = max(shadow, falloff(k))   # closer blocker = darker
        shadow = max(shadow, 1.0 - k/shadowSteps)
        break
visibility = 1.0 - shadow * intensity
```

- **Soft shadows** (Tatarchuk 2006): fire a few (2–4) jittered light rays and
  average, or use the *distance to the first blocker* as a penumbra weight
  (closer blocker → sharper, farther → softer). The snippet above uses the
  latter (cheap, single ray).
- The **light ray can be cone-step accelerated too** — sample `C(u,v)` and
  leap, just like the view ray (GPU Gems 3 Fig. 18-6 explicitly shows
  cone-accelerated shadow rays). This is what keeps self-shadowing affordable
  on an iGPU: a handful of cone leaps instead of 20+ fixed steps.
- Only do this for the **dominant directional / key light**, not every point
  light. Per-point-light self-shadowing is usually overkill for walls.

### 5.2 Horizon-based ambient occlusion (Dachsbacher–Tatarchuk)
Cheap AO approximation using the **max-mip pyramid** (§2.2): sample `k` height
values at distances `2^k` texels along several (4–8) directions, using mip
level `k` (so distant small detail is ignored). The max elevation seen sets the
horizon angle → AO. Per-pixel rotate the directions to hide the pattern.
- Cost: 4 directions ≈ 35% perf drop, 8 directions ≈ 60% (X1950 numbers, old
  hardware; modern iGPUs are relatively better at the texture reads but the
  ratio is indicative). Use 4 for walls.
- This gives contact darkening in grooves/corners that the light ray misses.

---

## 6. Shading & normal reconstruction

At the hit `(pu, pv)`:
- **Diffuse/specular color**: sample albedo at `finalUV`.
- **Normal**: either (a) sample a pre-baked tangent-space normal map at
  `finalUV`, or (b) reconstruct from the height gradient:
  `N = normalize(N' - (∂H/∂u)·T - (∂H/∂v)·B)` where `T,B` are the tangent/
  bitangent. (a) is cheaper and usual; (b) avoids a separate normal-map
  authoring step but is noisier.
- **Normal correction (PDM, Hoetzlein 2025 §5.1):** the finite-difference
  displaced normal contains the flat geometric normal `N_g`:
  `N_s = N_g + ∇D ⊗ ∇_P N'`. To remove base-triangle faceting on low-poly
  meshes without a C¹ intermediate surface, replace it:
  `N'_s = N_s - N_g + N'`. Cheap, big quality win on coarse base geometry.
- Light with the (corrected) normal + visibility from §5.

---

## 7. Performance budget (iGPU)

Texture fetches dominate (dependent reads). Per shaded fragment, rough cost:

| Stage | Fetches (typical) |
|-------|-------------------|
| Cone space-leap | `cone_steps` ≈ 15 (height+cone, 1 fetch ea.) |
| Binary refine | `binary_steps` ≈ 6 |
| Self-shadow (1 light, cone-accel) | ~8–12 |
| AO (4 dirs) | ~16 (cheap, lower-res mip) |
| Albedo + normal | 2 |
| **Total hero-fragment** | **~45–50 dependent fetches** |

Tune knobs:
- `cone_steps` 10–20, `binary_steps` 4–8. 15+6 is the GPU-Gems sweet spot.
- **Adaptive step count by view angle** (LearnOpenGL): `numLayers = mix(max,
  min, max(dot(N, V), 0))` — fewer samples head-on, more at grazing. Applies to
  cone steps too (fewer leaps head-on).
- **LOD by distance**: drop self-shadow + AO beyond N meters; drop to plain
  normal map beyond M meters. Walls far away don't need POM.
- **Stencil/early-Z**: render opaque depth first; PPOM only where visible.
- **Register pressure matters more than ALU** on iGPUs (GPU Gems 3 §18.4.2
  explicitly: avoid branches in the loops — branches bump register count →
  fewer concurrent threads → dependent-read latency exposed). Keep loops
  branch-free and fixed-length.

### Measured on fox-laptop iGPU (Radeon 680M, 2026-07-15)

Benchmarked the `SkyeShark/threejs-silhouette-pom` reference impl (fixed-step
steep-parallax POM, **not** cone-step accelerated — so this is the *slower*
variant) in the afterglow CEF shell (Chromium 149, WebGPU on the real radeon
adapter `amd/rdna-2`, vsync ON). Canvas = **2880×1800** (full native panel,
DPR 2 of the 1440×900 logical window). Full features = silhouette +
self-shadow + relief shadow maps.

| Quality (POM layers) | Features | FPS | p50 | p99 | max | <60fps /300 |
|----------------------|----------|----:|----:|----:|----:|----:|
| normalmap (parallax off) | — | 59.8 | 16.67 | 16.68 | 33.36 | 1 |
| low [8,32] | silh+sshadow+relief | 30.5 | 33.35 | 66.70 | 66.70 | 262 |
| medium [16,96] | silh+sshadow+relief | 20.4 | 50.03 | 100.05 | 150.07 | 282 |
| high [32,160] | silh+sshadow+relief | 13.4 | 66.71 | 216.78 | 233.45 | 287 |

**Reading:** at *full native panel res* only normalmap holds 60 Hz; full POM
(self-shadow + relief shadows) is 30 / 20 / 13 fps at low / medium / high.
The iGPU is genuinely stressed by per-pixel ray marching + a shadow pass at
5.18 M pixels. Crucially this is the **un-accelerated** fixed-step march —
cone-step space-leaping (the RCS half of PPOM-RCS) cuts the march fetches
roughly 3–5×, so the low/medium numbers should move comfortably past 60 once
accelerated. Also: 2880×1800 is the full panel; a real game renders walls at
~1440×900 (DPR 1 = 1/4 the pixels → ~3–4× faster), putting even medium quality
in budget.

For contrast, the same demo on fox-workstation (RTX 3090, vsync OFF) ran
1546 / 917 / 493 / 389 fps for normalmap / low / medium / high — a dGPU is
~25–30× the iGPU throughput on this workload.

Historical anchors: Dachsbacher–Tatarchuk 2007 hit 59 fps on a 7K-tri cylinder
with shadows on a **Radeon X1950 (2006)** — that's ancient silicon. A modern
iGPU (Radeon 680M / Intel Xe) is ~50–100× that throughput, so a wall-bound
scene is comfortably in budget at 60 fps with margin.

---

## 8. Artifacts & mitigations

| Artifact | Cause | Fix |
|----------|-------|-----|
| **Silhouette still flat** | You skipped the prism (plain POM). | Render the side slabs; march in the prism volume (§4). |
| **"Swimming"/wobble on move** | Too few cone steps; ray stops short. | Raise `cone_steps` to 15+; ensure binary search runs. |
| **Banding / layer steps** | Fixed-step POM leftovers. | Use cone+binary, not linear steps. |
| **Buckling at prism seams** | Tetrahedron diagonal mismatch / non-parallel offsets. | Consistent diagonal (Hirche); or parallel-offset prisms (PDM §4); or ray/bilinear sides. |
| **Faceting across base triangles** | `N_g` leaking into displaced normal. | PDM normal correction `N'_s = N_s - N_g + N'` (§6). |
| **Cone-map mip shimmer** | Averaged mip of cone map = wrong cones. | Build cone mip as **min** (conservative), or sample nearest. (GPU Gems 3 §18.5) |
| **Thin features missed** | March step `dt` > feature thickness. | Stochastic jitter of first sample (PDM §5.3) — integrates over frames, free. |
| **Shadow acne / peter-panning** | Light-ray step too coarse / bias. | Add a small depth bias; cone-accelerate the shadow ray. |
| **Texel-resolution shimmer at distance** | High-freq height map aliased. | Distance LOD falloff → normal map. |
| **Seams at UV tile boundaries** | Cone map computed across seams. | Compute cone map per-tile with edge replication; or pad. |

---

## 9. WebGPU / afterglow-engine integration notes

- **Everything is a fragment shader** — fully portable to WebGPU/WGSL. No
  tessellation stage, no mesh shaders, no RT needed. ✅
- **Mesh preprocess (prism extrusion)** can be done offline in the asset
  pipeline (Basis/asset worker) and stored as extra geometry, or generated in a
  **compute pass** at load (WebGPU compute is fine for this).
- **Cone-map precompute** is a one-time, per-texture job — ideal for the
  `afterglow-assets-worker` (async `#[rpc]`): load height map → generate cone
  map + max-mip on a worker → hand the textures to the renderer. Matches the
  engine's "lazy, incremental, tracked" allocation rule (keep the cone map in
  `EngineMemory`-tracked texture pool).
- **Allocation hygiene (AGENTS.md):** the fragment shader must not allocate.
  Cone/height/albedo/normal are bound textures; the march uses only scalar
  locals — no closures, no array literals, no Map/Set. Pass uniforms
  (`heightScale`, `cone_steps`, `binary_steps`, `shadowSteps`, light data) via
  a fixed UBO. ✅ compliant.
- **VT (virtual texturing) interaction:** if walls use the engine's virtual
  texture system, the cone map must be a **resident VT page** too (cone ratios
  are per-texel and must match the resident height pages). Feed forward the
  same page residency to the cone-map texture. Min-mip cone fallback = nearest
  sampling of the coarsest cone page (conservative).
- **No built-in Three.js helper** — this is custom WGSL. Implement as a
  `ShaderMaterial`/TSL node with the prism geometry + the march function.
- **Future RT path:** when WebGPU gets an RT extension, the *same height maps*
  feed PDM (Hoetzlein 2025) for true reflections/shadows of the displaced
  surface — the art pipeline is forward-compatible.

---

## 10. Minimal WGSL sketch (interior march — no prism, for clarity)

```wgsl
// uniforms: height_scale, cone_steps (i32), binary_steps (i32)
// tex: relief_map (height in .a, cone_ratio in .b)

fn ray_intersect_rcs(relief: texture_2d<f32>, samp: sampler,
                     entry_uv: vec2<f32>, view_dir_ts: vec3<f32>) -> vec2<f32> {
  var dir = view_dir_ts / view_dir_ts.z;        // scale so dz=1
  let ray_ratio = length(dir.xy);

  // --- relaxed cone space-leap ---
  var pos = vec3<f32>(entry_uv, 0.0);
  for (var i: i32 = 0; i < cone_steps; i = i + 1) {
    let tex = textureSample(relief, samp, pos.xy);
    let cone_ratio = tex.b;
    let height = clamp(tex.a - pos.z, 0.0, 1.0);
    let d = cone_ratio * height / (ray_ratio + cone_ratio);
    pos = pos + dir * d;
  }

  // --- binary search refine over [entry+0.5*range, stop] ---
  var rng = 0.5 * dir * pos.z;
  var p = vec3<f32>(entry_uv, 0.0) + rng;
  for (var i: i32 = 0; i < binary_steps; i = i + 1) {
    let tex = textureSample(relief, samp, p.xy);
    rng = rng * 0.5;
    if (p.z < tex.a) { p = p + rng; }          // above surface → forward
    else             { p = p - rng; }          // below surface → back
  }
  return p.xy;
}
```
Add the prism entry/exit (§4) around this, the self-shadow light ray (§5.1),
and the PDM normal correction (§6). That's the full PPOM-RCS.
