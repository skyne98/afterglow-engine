# Runtime sprite-bake idea: Daggerfall-style sprites with 3D equipment fit

**Status:** idea — recorded for later decisions, not a plan
**Date:** 2026-08-09
**Scope:** sprite-based character rendering with 3D-equivalent equipment fit,
dynamic lighting on sprites, and a deterministic runtime bake + disk cache
pipeline. This is a candidate rendering path for the engine, not a committed
design.

Related documents:

- [`../implementation/runtime-character-bake-plan.md`](../implementation/runtime-character-bake-plan.md)
- [`../api/persistent-blob-store.md`](../api/persistent-blob-store.md)
- [`../api/engine-memory.md`](../api/engine-memory.md)
- [`../implementation/unified-paged-resources-completion-plan.md`](../implementation/unified-paged-resources-completion-plan.md)
- [`virtual-texturing.md`](virtual-texturing.md)
- [`animation-generation-runtime-models-survey.md`](animation-generation-runtime-models-survey.md)

## The idea in one paragraph

Model the character and all equipment in 3D on one shared skeleton, then bake
the rendered frames to sprite sheets. Because every item shares the skeleton and
camera, it fits any character automatically — the "like 3D" part comes from the
3D work, not from hand-aligned 2D art. Bake a G-buffer per frame (albedo,
normal, depth, AO/roughness/metallic) so the sprites can be lit dynamically at
run time. Do the baking on demand in the background with a deterministic,
content-hashed disk cache, and page the resulting atlas tiles through the
existing virtual-texture system so VRAM stays bounded.

## Why this exists

The goal is a Daggerfall-style visual: stylish 2D sprites of characters, with
the flexibility of a 3D game where any armor, clothing, or item fits any
character and moves with the animation.

Daggerfall's original paper-doll cannot do this. Its layers were hand-drawn 2D
art on a fixed template. They do not follow the animation, so armor slides and
floats when the body moves. The "3D to sprite" route fixes that, because the
alignment is produced by the shared rig, not by hand.

## Core method

1. Build the character in 3D with a standard humanoid skeleton (rig).
2. Model each armor and clothing item on the same base body, skinned to the
   same skeleton. This is where the automatic fit comes from.
3. For different body shapes, use shape keys or a fit-to-body tool so the item
   deforms with the body. The engine already has SurfaceWrap fitting and recipe
   baking in the character bake plan; this idea consumes that output.
4. Render with an orthographic camera and a flat (toon) shader, across N view
   angles and M animation frames. Save the output as sprite sheets.
5. At run time, show and swap sheets per equipment slot.

### Overlay compositing

The body is one sprite sheet. Each equipment item is its own overlay sheet baked
from the same rig and camera. Because they share the skeleton and camera, they
line up with zero manual alignment. This keeps the combination count additive
(body + N items) instead of multiplicative (body x N outfits).

## Dynamic lighting on sprites

Bake a full G-buffer per sprite frame, not just color:

- Albedo (the visible sprite)
- Normal map, tangent space, camera-facing plane
- Depth (optional — enables relief shading and accurate shadows)
- AO + roughness + metallic packed into one texture

At run time, the fragment shader does standard PBR on the quad:

- Reconstruct the normal from the normal map. The quad's tangent frame is the
  screen plane, so the surface becomes a heightfield facing the camera.
- Transform each world light into that frame.
- Compute diffuse + specular from roughness and metallic. The view vector is
  camera forward.
- The sprite reacts to light direction, angle, and color like 3D geometry.

### Shadows

- Receive: sample the scene's shadow map in the sprite shader (2.5D scenes),
  or use a 2D-light system that renders occluder silhouettes (pure 2D).
- Cast: with the baked depth map, render the sprite's relief into the shadow
  map. This gives accurate shadows, not blobs. Without depth, use blob shadows.

### The mirroring gotcha

When a sprite is flipped for left/right facing, the quad's tangent frame flips,
so the baked normal X-axis must flip too. Options:

- Bake two normal sets, one per facing.
- Flip the normal map's R channel in the shader when mirrored.

The depth map has the same issue when used for parallax.

### Limits

View-dependent effects cannot live in one flat sprite: specular highlights,
sharp reflections, and grazing-angle Fresnel. Approximate them — bake a
reflection probe per view angle and look it up via the sprite's normal map.
This works for broad or matte surfaces, and is weak for mirror-like surfaces.

## Honest trade-offs

### Sprites vs 3D on modern GPUs

Sprites win on:

- Vertex and fragment work (one quad per character).
- No runtime skinning, no multi-pass lighting, no shadow-map renders of the
  model — the character is one fragment pass.
- Low-end and mobile hardware, and large crowds (instanced quads).

Sprites lose on:

- Memory and bandwidth. One character at 8 angles x 30 frames with a full
  G-buffer is hundreds of MB of atlas. A 3D character is a few MB.
- Modern GPUs are extremely fast at triangles. A skinned character in a few
  draw calls is nearly free. For a small cast, 3D is usually cheaper.

The deciding factors are memory and asset-pipeline cost, not fill rate.
Sprites were a clear win in the 1996 era because GPUs barely existed. That
constraint no longer exists.

### The skeletal-animation loss

Baking removes runtime animation flexibility:

- No blending, IK, procedural motion (cloth and hair physics, weapon sway,
  aiming at targets), retargeting, or mixed animations.
- Baked frames are a fixed film strip.

This is the biggest real cost of the approach. The modern alternatives that
keep the look without baking:

- Runtime render-to-sprite: render the animated 3D character to an offscreen
  target with a toon shader, then draw it as a billboard. Full animation
  flexibility, one extra render pass per character.
- Octopath-style: real 3D geometry with stylized textures, low-res filtering,
  and camera tricks that read as 2D. Full animation flexibility.

"Sprite look" and "3D rig" are not exclusive. Baked sprites remain right for
strict pixel art, huge crowds, mobile, and fixed simple animations.

## The architecture: runtime deterministic bake + disk cache + VT

This is the core new idea. Bake sprites when needed, in the background, with a
deterministic pipeline and a disk cache.

### Pipeline

```text
(rig, outfit, animation clip, angle, frame)
        |
        v
background bake worker (offscreen render)
        |
        v
encode (BCn) -> content-hash key
        |
        v
disk cache (PersistentBlobStore)  <-- hit: load, miss: store
        |
        v
VT pager residency (bounded VRAM, page per frame tile)
```

- Worker threads render on demand at low priority.
- The disk cache is keyed by a content hash of all inputs.
- Baked frames go into the existing virtual-texture pager, one tile per frame,
  with mip levels.
- Prefetch by animation-state prediction: warm the cache for the equipped
  outfit, nearby NPCs, and likely next animations during load screens and idle
  frames.

### Feasibility

A 256 x 256 toon-shaded frame is about 1 to 5 ms on GPU. A new outfit is about
1,000 frames, so a few seconds of background baking. This is feasible mid-game
if the pipeline is fast and does not bake everything at once.

### Determinism is the hard 10%

- Float math is not bit-identical across GPUs and drivers.
- Accept per-machine caches (content-hash keyed, like Steam's shader cache):
  compile once per machine, reuse forever.
- Or force cross-machine determinism (CPU rasterization, fixed-point math) so
  caches are shareable and buildable on a render farm.
- Hash all inputs: rig, mesh, material, shader source, animation data, and RNG
  seeds. Cloth and noise must be seeded.

### Budgeting

Background baking competes with the game for GPU and CPU. On console and
mobile, do the baking in load screens or at install.

### Precedent

Daggerfall Unity already does runtime 3D-to-sprite conversion on demand. This
idea is the same thing plus caching, determinism, and VT residency.

## How it fits the engine

The engine already has most of the pieces:

- `CharacterBakeWorker` produces fitted, skinned characters (the
  runtime-character-bake plan). This idea consumes that output and renders it
  to sprites.
- `PersistentBlobStore` is the natural disk cache: bounded admission, atomic
  generation, content-hash keys, native and OPFS backends.
- The virtual-texture pager is the natural residency layer: bounded VRAM,
  per-tile publication, LRU/clock eviction.
- The native/Web worker split gives the background bake worker on both targets.

## Open questions

- When is the sprite path better than runtime render-to-sprite or toon 3D?
  Candidate answers: strict pixel art, crowds, fixed camera angles, or a
  shipped look with pre-baked lighting.
- How many angles and frames per animation does the style need?
- Is a per-frame bake cache worth it, or is the VT atlas itself the cache?
- Do sprite characters need to cast and receive shadows, and at what cost?
- How does sprite sorting interleave with the depth-based occlusion of the
  existing 3D scene?
- Does the deterministic bake need cross-machine sharing (render farm), or is
  a per-machine cache enough?

## What to do next

Nothing yet. This is an idea record. If it is picked up, the first step is a
small vertical slice: one character, one outfit, one animation, one angle
set, baked on demand with a `PersistentBlobStore` cache and published into the
VT pager, with a measured first-seen latency.
