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

## References and findings

Research gathered 2026-08-09. Each entry lists the reference and the verified
findings that matter for this idea.

### 3D-to-sprite baking in production

**Dead Cells: Art Design Deep Dive — Using a 3D pipeline for 2D animation**
(Thomas Vasseur, Gamedeveloper.com, 2018-01-25)
https://www.gamedeveloper.com/production/art-design-deep-dive-using-a-3d-pipeline-for-2d-animation-in-i-dead-cells-i-

Findings:

- Dead Cells is 3D-to-sprite in production: a basic 3D model (character is
  about 50 px on screen) is animated on 2D-style keyframes, then a homebrew
  tool renders each animation frame to a small PNG with no anti-aliasing to
  get the pixel look.
- Each frame is exported together with its normal map and shaded with a toon
  shader for volume. This is the G-buffer sprite idea already proven in a
  shipped game.
- Keyframe pose-to-pose animation: interpolation frames are added before or
  after keyframes, never in-between.
- Adding armor is "attach the asset to the 3D model" — the automatic-fit
  claim, confirmed in practice. Old assets are reused across new monsters.
- Retakes are cheap: move keyframes, adjust poses, re-export. This was the
  main reason for the pipeline.
- Known unsolved issue: pixel flicker on animation. Cell shading at low
  resolution without anti-aliasing is the rendering recipe.
- Roots: 2015 Ludum Dare game ScarKrow; inspiration from King of Fighters,
  Blazblue, Guilty Gear.

**sinanata/unity-3d-to-sprite-baker** (GitHub, MIT, 2026, production in the
shipped game Leap of Legends)
https://github.com/sinanata/unity-3d-to-sprite-baker

Findings:

- Runtime bake at game start: every clip x frame is captured into one packed
  atlas; playback is a flat textured quad. On Leap of Legends' mobile
  low-quality preset this replaces a 60-bone skinned mesh, cutting per-
  character GPU cost to about 5%.
- Measured motivation: on mobile, a visible 3D character can eat 10-15% of
  frame time per instance (skinning, bone-matrix uploads, multi-pass
  lighting, shadow casts).
- Technical recipe: orthographic capture at a far origin, capture-stage
  lighting rig (offscreen stage has no lights — characters render black
  without one), async GPU readback pipelined over three frames (a blocking
  `ReadPixels` stalls 7-30 ms on iOS Metal), one animator yield per frame
  (stepping a clip within one frame silently captures the same pose).
- Origin-aligned quads: the quad's bottom edge sits at the model pivot, so a
  3D character and a baked sprite at the same world position have identical
  foot positions. Bounds-aligned crops make feet float when swapping — the
  same trap this idea must avoid.
- Atlas cost is the combinatorial wall: 10 characters x 8 skins x 5 hats =
  400 atlases at 128 px each is roughly 800 MB. The author explicitly warns
  the pipeline scales to tens of combos, not hundreds. This is the exact
  memory argument for per-item overlay sheets plus VT residency.
- Cache: a content-derived key (prefab name x skin x resolution x FPS) so a
  re-bake after a quality change does not reuse a stale atlas — a simple
  version of the content-hash cache in this idea.
- Do not use when: per-bone procedural animation is needed (eyes tracking,
  hats tracking the cursor), cosmetic combinations are many, heavy GPU post
  effects must be baked, or complex shader networks are required.

### Historical: Daggerfall and Battlespire already did this

**General: Mark Jones Interview** (UESP; Mark Jones was the Daggerfall and
Battlespire artist)
https://en.uesp.net/wiki/General:Mark_Jones_Interview

Findings:

- Battlespire was "the last of the 2D/3D hybrids": high-polygon 3D models
  were built and animated, then "chopped down" into sprites to fit.
- The memory wall is as old as the technique: "sprites take up a lot of
  memory when they are large and have lots and lots of animation".
- Daggerfall work included a fully built, textured, animated 3D dragon that
  never shipped.

**Spriter: Real-Time Sprite Imposter System** (Daggerfall Workshop forums,
MasonFace, 2018-2020)
https://forums.dfworkshop.net/viewtopic.php?t=1613

Findings:

- Daggerfall's original enemy sprites were pre-rendered (baked) from 3D
  models, not hand-drawn: "creating 3D models and scanning them into 2D to
  make good-looking sprites" (Jay_H). The Daggerfall-style sprite aesthetic
  is itself a product of the 3D-to-sprite bake — this idea is a return to
  the original technique with modern machinery.
- MasonFace's "Spriter"/"SpriteWrite" Unity asset is the real-time imposter
  alternative: a 3D model is rendered as a sprite on the fly — no
  pre-rendering, no texture atlas, world lighting is applied onto the
  sprite, custom animations work, and weapon swaps work. This is the
  runtime render-to-sprite billboard branch of this idea, proven in the
  Daggerfall community.
- SquidKamer's Daggerfall Bestiary project does the opposite: new enemy
  sprites pre-rendered from 3D models into sprite sheets.

**Daggerfall Unity source — `DaggerfallMobileUnit.cs`** (Interkarma/
daggerfall-unity)
https://github.com/Interkarma/daggerfall-unity/blob/master/Assets/Scripts/Internal/DaggerfallMobileUnit.cs

Findings:

- The class doc is explicit: "A billboard class for classic Daggerfall mobile
  sprites with 8 orientations". Classic sprites were camera-facing
  billboards with 8 discrete yaw orientations.
- `transform.LookAt(viewDirection)` on the horizontal plane confirms the
  billboard behaviour; animations are frame lists per state (idle, primary
  attack, ranged attack, spell, flying transform).

### HD-2D (Octopath Traveler)

**HD-2D** (Wikipedia)
https://en.wikipedia.org/wiki/HD-2D

Findings:

- HD-2D = 2D pixel art and billboard sprites combined with fully 3D
  environments, plus modern effects (dynamic lighting, depth of field,
  tilt-shift) for a diorama look.
- Early Octopath prototypes used Final Fantasy VI sprite assets while the
  team experimented with lighting; a point light was added so characters
  cast shadows on the environment.
- Square Enix trademarked the term in 2019; the style now spans Triangle
  Strategy, Live A Live, Dragon Quest remakes, and clones (Wandering Sword,
  Eiyuden Chronicle).

**Octopath Traveler's HD-2D art style — Unreal Engine spotlight** (Acquire
interview, 2019)
https://www.unrealengine.com/en-US/spotlights/octopath-traveler-s-hd-2d-art-style-and-story-make-for-a-jrpg-dream-come-true

Findings:

- The stated recipe: "fuse 2D pixels together with a 3D environment", built
  on UE4. Reference look: PS1-era games with 2D characters on pre-rendered
  3D backgrounds.
- Team size evidence: at peak only six programmers; heavy reliance on UE4
  Blueprint and its optimization tools.
- The point-light-plus-VFX trick was added specifically because plain VFX
  "didn't look good enough" — lighting is the element that sells the
  hybrid look.

### Dynamically lit sprites (normal maps)

**Lighting with 2D normal maps** (GDQuest, Pablo Fonovich, 2020)
https://www.gdquest.com/tutorial/godot/2d/lighting-with-normal-maps/

Findings:

- Godot supports this natively: a `Sprite` node has a `Normal Map`
  property; `Light2D` has a `Height` property that sets the light's Z
  distance above the sprite plane and controls how strong the emboss
  effect is; `CanvasModulate` tints the whole scene as ambient light.
- Works for sprites, animated characters, animated sprites, tilemaps, and
  seamless textures. This is engine-level support for the lighting half of
  this idea.

**SpriteIlluminator** (CodeAndWeb)
https://www.codeandweb.com/spriteilluminator

Findings:

- A normal map editor purpose-built for 2D sprites; per-pixel facing
  direction encoded in RGB.
- Requires a 3D-based renderer (OpenGL, WebGL, Metal, Vulkan, DirectX); it
  cannot work on pure 2D canvas renderers. Phaser, PixiJS, Unity, and Godot
  support normal-mapped sprites out of the box; cocos2d-x needs a custom
  shader.

**Laigter** (azagaya, GitHub, automatic normal map generator for sprites)
https://github.com/azagaya/laigter

Findings:

- Generates normal, parallax, specular, and occlusion maps for existing 2D
  sprites, with an in-game preview. Relevant as the alternative when there
  is no 3D source to bake from — but the maps are approximations from the
  image, not true surface data.

**paweljarosz/normal-map-lighting-2d-sprites-defold** (GitHub)
https://github.com/paweljarosz/normal-map-lighting-2d-sprites-defold

Findings:

- A reference implementation of lit 2D pixel-art sprites in one draw call
  (OpenGL fragment program), confirming the one-pass-per-sprite cost model
  in this idea.

### Virtual texturing residency (the atlas half)

**Granite SDK 5.0 whitepaper** (Graphine, 2018)
https://graphinesoftware.com/sites/default/files/shared/whitepaper_granite_sdk5.pdf

Findings:

- Tiles are typically 128 x 128 texels and are the basic unit of streaming;
  atlases can be up to 256K x 256K.
- Two modes: classic mip streaming (full pyramid allocated, mips streamed)
  and fine-grained virtual texturing (cache texture of tiles, shader
  indirection).
- "Stacked Texture": multiple layers (diffuse, normal, roughness) that share
  UVs are bundled into one logical unit and streamed together. This is
  exactly the G-buffer page concept in this idea — albedo, normal, and mask
  layers as one stacked page.
- Tile sets can be stored on disk **or procedurally generated in memory on
  the fly** — the runtime-baked sprite tile is a supported pattern, not a
  novelty.
- Residency is screen-driven: tiles load only when visible, at the required
  mip. Production use on millions of devices.

**Virtual texturing at PlayerUnknown Productions** (Dmytro Pustovoitov,
lead render programmer)
https://playerunknownproductions.net/news/virtual-texturing

Findings:

- Residency model: texture fully in/out, per-mip, or per-tile (128 x 128
  regions). Textures are only partially visible, so only the visible parts
  need to be resident.
- Measured result: 70-80% less texture memory on GPU, including the cost of
  indirection textures, feedback buffers, and readback buffers.
- Pipeline: render feedback -> double/triple-buffered readback -> CPU
  processing -> async disk load -> GPU copy.
- Uses hardware reserved resources and sampler feedback; explicitly notes
  both are optional and easy to replace with software implementations —
  relevant for WebGPU, where the same features are emerging.
- DirectStorage is a good fit for many small tile chunks; deferred due to
  priority.
- Caveat that matters for sprite pages: tile streaming works best with big
  textures; small textures give less efficient savings. Sprite atlases are
  big textures, so this is a good fit.

**Sparse Virtual Textures** (Sean Barrett, GDC 2008 talk + demo)
https://silverspaceship.com/src/svt/

Findings:

- The canonical public SVT reference; inspired by Carmack's MegaTexture.
  Simulates very large textures with a small physical cache and a pixel
  shader mapping virtual to physical.
- Explicitly designed for "large quantities of smaller textures" packed
  into one big texture or multiple page tables — sprite sheets qualify.
- Rendering into the SVT: build procedural pages a block at a time by
  rendering into a texture; Barrett did it in software and states hardware
  is feasible. This directly supports the bake-into-VT residency model of
  this idea.
- Decals/overlays are handled page-by-page (invalidate, rebuild, or render
  onto each intersecting page) — the same shape as equipment overlay
  sprites.
- DXT matters for sampling bandwidth, not just memory — compression choice
  is a runtime performance decision.
- See also the existing `kb-virtual-texturing` knowledge base and this
  repo's `virtual-texturing.md` for id Tech 5 / Rage and the afterglow VT
  runtime audit.

### Deterministic caches (the bake-cache precedent)

**Shader caches** (Emulation General Wiki)
https://emulation.gametechwiki.com/index.php/Shader_caches

Findings:

- GPU pipeline caches are the established precedent for a per-machine
  content cache: compiled output is keyed to the GPU and driver, stored
  locally, and reused across runs. Recompiling at runtime causes stutter.
- Cache validity breaks when the emulator's shader handling changes — so
  the cache carries a version. This mirrors the determinism problem: bake
  output must be versioned against rig, baker, and shader changes.
- Caches are shared between machines only when output is portable;
  otherwise each machine builds its own first-run cache (Steam
  pre-caching is the mainstream example of warming that cache in advance).

**Vulkan pipeline cache sample** (Khronos / Vulkan Documentation Project)
https://docs.vulkan.org/samples/latest/samples/performance/pipeline_cache/README.html

Findings:

- Creating graphics pipelines compiles shader modules internally and is a
  significant frame-time cost if done at runtime; a `VkPipelineCache`
  object persists the compiled result across runs and is supplied when
  creating pipelines. The API-level form of the same idea: compile once,
  cache by identity, reuse.

## What the research confirms or changes

- The Daggerfall sprite look itself comes from 3D-to-sprite baking: the
  original enemy sprites were rendered from 3D models. This idea is a
  return to the original technique, not a new invention.
- Production numbers to cite: 400 atlas combos at 128 px is about 800 MB
  (unity-3d-to-sprite-baker); a baked sprite quad costs about 5% of a
  60-bone skinned mesh; on mobile one visible 3D character can cost 10-15%
  of frame time per instance.
- Dead Cells already exports per-frame normal maps and uses a toon shader:
  the G-buffer sprite half of this idea is shipped practice.
- Both branches of the design are proven in the Daggerfall community:
  pre-rendered bake (original enemies, Daggerfall Bestiary) and real-time
  imposter (Spriter/SpriteWrite, with world lighting on the sprite).
- Granite's Stacked Texture (diffuse/normal/masks as one unit) and its
  on-the-fly procedural tile sets, plus Barrett's page-at-a-time render
  into the SVT, validate the bake-into-VT residency model.
- PUP's 70-80% VRAM saving is the anchor number for the residency benefit;
  the big-texture caveat means sprite atlases should be large shared
  atlases, not small per-character textures.
- Shader and pipeline caches are the established per-machine content-cache
  pattern; bake caches should follow the same rules (key on inputs +
  versions, warm in advance, accept per-machine divergence).
