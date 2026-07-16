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
- smooth distance fade (Dungeon: disabled beyond 3.25 m),
- grazing-angle offset limiting (Dungeon ratio cap: 2),
- fixed maximum work and explicit-LOD height samples,
- one shared displaced UV for albedo, normal, roughness, and AO,
- no silhouette, self-shadow, or secondary relief pass.

This is the tier measured viable on the Radeon 680M. It intentionally does not
expose the prototype's expensive evaluation modes.

## Virtual-texture composition

`VT_SAMPLE_FROM_LEVEL_WGSL` in `engine/virtual-texture.ts` samples a displaced
UV starting at an already-resolved material mip and walks toward coarser
resident pages. It receives a separate `gradientUV`, preserving the original
continuous screen derivatives rather than deriving across POM control-flow or
physical-atlas page boundaries. A pinned mip tail is the deterministic final
fallback.

The Dungeon uses compact, resident 1K ambient-occlusion maps from the exact
same ambientCG materials as height fields. AO is white on exposed stone and
dark in cracks/mortar, giving a conservative pseudo-height convention. The 8K
albedo, normal, roughness, and AO runtime channels remain virtual. Keeping the
height field resident avoids asynchronous page seams during a non-uniform POM
march.

Three r185 builds diffuse color before AO and lazy lighting normals. The
Dungeon marches once in the diffuse flow, assigns the result to one
shader-local property, then samples albedo, normal, roughness, and AO with that
same displaced UV. This preserves channel registration without tripling the
march. Both complete materials are prewarmed; `P` swaps fixed material
references without runtime pipeline creation.

## Mathematical oracle and regression shapes

`marchPomReference(...)` is an allocation-free CPU oracle matching the WGSL
march. Unit coverage uses analytically predictable fields: fully raised,
fully recessed, half-height planes, rising/falling linear ramps, hard steps,
thin ridges, a circular island, direction symmetry, distance fade, layer
bounds, physical-height clamping, and grazing offset limits. These tests caught
and fixed the original height/depth inversion (`height` was incorrectly used as
ray depth).

## Dungeon controls and automation

- `P`: toggle the prewarmed POM/base material variants.
- `window.__afterglowDungeon.setPomEnabled(boolean)`
- `window.__afterglowDungeon.pomStatus()` reports layer bounds, scale, offset
  cap, distance, height source, and enabled state.

The removed standalone `prototype/pom` and `pom_bench` launcher are superseded
by this engine primitive and the main Dungeon demo.

## Validated 680M result (2026-07-16)

At 1440×900 logical / 2880×1800 physical pixels, a close angled stone-wall view
with POM enabled held 59.97 FPS, p99 16.675 ms, and 0/300 frames below 55 FPS.
CDP compositor screenshots confirmed that base and POM both retain real VT
texture pixels and differ across 78% of frame pixels at the validation pose.
There were zero WebGPU errors and zero post-seal pipeline violations. See
`docs/benchmarks/dungeon-pom-2026-07-16.md` for the exact pose, method, and
pixel-correctness gates.
