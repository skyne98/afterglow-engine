# Surface detail and POM

The Dungeon combines streamed 8K stone materials with a bounded close-range
parallax occlusion mapping tier.

The engine's `POM_UV_WGSL` performs an adaptive 8–32-layer march with linear
intersection refinement. Its effect fades after 65% of the configured range
and is completely disabled beyond 3.25 m. It does not include silhouettes,
self-shadowing, or a second relief pass—the measured low-core tier is the only
variant enabled for normal gameplay on the Radeon 680M.

Height comes from compact resident 1K ambient-occlusion maps belonging to the
same ambientCG materials. Exposed stone is light and cracks/mortar are dark,
which provides a conservative pseudo-height field. Keeping this small field
resident prevents page residency from changing during the non-uniform march;
the 8K color, normal, and packed mask channels remain virtual.

Displaced albedo uses `VT_SAMPLE_FROM_LEVEL_WGSL`: it begins at the material's
resolved mip, walks to a coarser resident page when the displaced coordinate
crosses a missing page, and finally uses the pinned mip tail. Gradients come
from the undisplaced UV, avoiding physical-atlas seams.

Press **P** in the Dungeon to switch between two prewarmed materials. The swap
never compiles a gameplay pipeline. Three r185 cannot safely share one
fragment-local marched UV among independent NodeMaterial output flows, so POM
is applied to albedo while the existing high-frequency VT normal/mask flow is
retained. This avoids triple marching and fallback corruption.

On the 680M at 2880×1800 physical pixels, the close angled-wall validation held
59.97 FPS, p99 16.675 ms, and 0/300 frames below 55 FPS with POM enabled.
