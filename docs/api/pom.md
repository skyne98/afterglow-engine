# Surface-detail / POM API

`crates/afterglow-web/www/engine/surface-detail.ts` exports the reusable
low-core POM shader used by the Dungeon demo.

## `POM_UV_WGSL`

Pass the string to Three.js `wgslFn()`. `pomMarchUV` accepts a resident height
texture/sampler, base UV, tangent-space view direction, height scale, maximum
offset ratio, layer bounds, maximum distance, and fragment view distance. It
returns a displaced UV. Input is **physical height** (`1` exposed, `0`
recessed); the marcher intersects ray depth against `1 - height`.

The bounded tier provides:

- view-angle-adaptive layers (Dungeon: 8–32),
- linear intersection refinement,
- optional smooth distance fade (`maxDistance <= 0` disables it),
- grazing-angle offset limiting (Dungeon ratio cap: 2),
- fixed maximum work and explicit-LOD height samples,
- one shared displaced UV for albedo, normal, roughness, and AO,
- bounded direct-light self-shadow (`POM_SELF_SHADOW_WGSL`),
- no silhouette or secondary relief/depth pass.

Dungeon deliberately disables radial per-fragment fading: its moving distance
contour was visibly a wave of flattening across long walls. Material/coverage
LOD must switch whole surfaces rather than vary relief strength per pixel.

This is the tier measured viable on the Radeon 680M. It intentionally does not
expose the prototype's expensive evaluation modes.

## Virtual-texture composition

`VT_SAMPLE_FROM_LEVEL_WGSL` in `engine/virtual-texture.ts` samples a displaced
UV starting at an already-resolved material mip and walks toward coarser
resident pages. It receives a separate `gradientUV`, preserving the original
continuous screen derivatives rather than deriving across POM control-flow or
physical-atlas page boundaries. A pinned mip tail is the deterministic final
fallback.

The Dungeon uses the official resident 1K **16-bit displacement maps** from the
exact ambientCG materials. AO is lighting information, not geometric height;
using it as pseudo-height was a correctness bug. The 8K albedo, normal,
roughness, and AO runtime channels remain virtual. Keeping displacement
resident avoids asynchronous page seams during a non-uniform POM march. The
resident map uses `flipY=false` to match VT storage; sampled `NormalGL` uses
normal scale `(1,-1)` because the custom VT upload retains top-left row order.
Without that correction, illumination appeared below features for a light above.

Three r185's built-in `parallaxDirection` contains the material `normalView`.
Using it while the normal map itself needs displaced UV creates a dependency
cycle. Dungeon builds the tangent-space view ray from the geometric normal and
explicit tangent instead, matching the standard POM pipeline. Diffuse color's
first material flow marches once and publishes initialized UV/mip properties;
albedo, normal, roughness, and AO all sample those values. Both complete
materials are prewarmed; `P` swaps fixed material references without runtime
pipeline creation.

## Mathematical oracle and regression shapes

`POM_SELF_SHADOW_WGSL` traces from the physical-height hit toward each direct
light. Dungeon uses 8 bounded samples, ratio-2 offset limiting, bias 0.01, and
82% strength. A custom `PhysicalLightingModel` attenuates direct diffuse and
specular only; hemisphere/indirect fill is not incorrectly multiplied.

`marchPomReference(...)` is an allocation-free CPU oracle matching the WGSL
march. Unit coverage uses analytically predictable fields: fully raised,
fully recessed, half-height planes, rising/falling linear ramps, hard steps,
thin ridges, a circular island, direction symmetry, distance fade, layer
bounds, physical-height clamping, and grazing offset limits. These tests caught
and fixed the original height/depth inversion (`height` was incorrectly used as
ray depth). `assertPomGeneratedWgsl()` additionally checks Three's generated
fragment shader during warm-up: exactly one march, geometric TBN, march before
all VT reads, and exactly three linked PBR samples.

## Dungeon controls and automation

- `P`: toggle the prewarmed POM/base material variants.
- `window.__afterglowDungeon.setPomEnabled(boolean)`
- `window.__afterglowDungeon.pomStatus()` reports layer bounds, scale, offset
  cap, fade state, self-shadow samples/strength, height source, and enabled state.

The removed standalone `prototype/pom` and `pom_bench` launcher are superseded
by this engine primitive and the main Dungeon demo.

## Reference audit

The implementation is checked against LearnOpenGL's known-working POM sample:
adaptive 8–32 layers, `view.xy / view.z`, subtracting the per-layer UV delta,
first crossing, and before/after linear refinement. Three r185's official
`parallaxUV` confirms the same un-negated direction/sign convention. Dungeon
adds bounded offset and an 8-sample direct-light shadow ray ported from the
known-working prototype.

- https://learnopengl.com/Advanced-Lighting/Parallax-Mapping
- https://github.com/JoeyDeVries/LearnOpenGL/blob/master/src/5.advanced_lighting/5.3.parallax_occlusion_mapping/5.3.parallax_mapping.fs
- `node_modules/three/src/nodes/accessors/AccessorsUtils.js`
- Natalya Tatarchuk, *Practical Parallax Occlusion Mapping*:
  https://advances.realtimerendering.com/s2006/Tatarchuk-POM.pdf

Corrected Radeon 680M measurements at 2880×1800 physical pixels:

- fixed close wall, POM + self-shadow: 59.87 FPS, p99 16.68 ms, 1/600 below 55;
- prewarmed base at the same pose: 59.77 FPS, p99 16.68 ms, 2/600 below 55;
- moving along the wall: 59.97 FPS, p99 16.68 ms, 0/300 below 55;
- GPU main 1.02–1.09 ms; timestamp total 5.49–7.25 ms.

The isolated missed-vsync events are present in the base variant too and are
not POM saturation. Reducing VT feedback cadence from every 4 frames to every 8
reduced the measured POM misses from 3/300 to the baseline range. See
`docs/benchmarks/dungeon-pom-2026-07-16.md`.
