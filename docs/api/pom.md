# Surface-detail / POM API

`crates/afterglow-web/www/engine/surface-detail.ts` exports the reusable
low-core POM shader used by the Dungeon demo.

## `POM_UV_WGSL`

Pass the string to Three.js `wgslFn()`. `pomMarchUV` accepts a resident height
texture/sampler, base UV, tangent-space view direction, height scale, minimum
and maximum layers, maximum distance, and fragment view distance. It returns a
displaced UV.

The bounded tier provides:

- view-angle-adaptive layers (Dungeon: 8–32),
- linear intersection refinement,
- smooth distance fade (Dungeon: disabled beyond 3.25 m),
- fixed maximum work and explicit-LOD height samples,
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

Three r185 builds NodeMaterial outputs as independent control-flow stacks and
cannot safely carry a fragment-local marched UV between them. The Dungeon
therefore applies the POM UV to albedo while retaining its established
high-frequency VT normal/mask flow. Both complete materials are prewarmed; `P`
swaps fixed material references without runtime pipeline creation. This is a
known composition boundary, not hidden duplicated marching.

## Dungeon controls and automation

- `P`: toggle the prewarmed POM/base material variants.
- `window.__afterglowDungeon.setPomEnabled(boolean)`
- `window.__afterglowDungeon.pomStatus()` reports layer bounds, scale, distance,
  height source, and enabled state.

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
