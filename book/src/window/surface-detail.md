# Surface detail and POM

The Dungeon combines streamed 8K stone materials with a bounded close-range
parallax occlusion mapping tier.

The engine's `POM_UV_WGSL` performs an adaptive 8–32-layer march with linear
intersection refinement. Physical height (`1` exposed, `0` recessed) is
converted to ray depth with `1 - height`; a ratio-2 offset limit prevents
exploding UVs at grazing angles. Its effect fades after 65% of the configured
range and is completely disabled beyond 3.25 m. It does not include silhouettes,
self-shadowing, or a second relief pass—the measured low-core tier is the only
variant enabled for normal gameplay on the Radeon 680M.

Height comes from compact resident 1K ambient-occlusion maps belonging to the
same ambientCG materials. Exposed stone is light and cracks/mortar are dark,
which provides a conservative pseudo-height field. Keeping this small field
resident prevents page residency from changing during the non-uniform march;
the 8K color, normal, and packed mask channels remain virtual.

Displaced PBR channels use `VT_SAMPLE_FROM_LEVEL_WGSL`: each begins at the
material's resolved mip, walks to a coarser resident page when the coordinate
crosses a missing page, and finally uses the pinned mip tail. Gradients come
from the undisplaced UV, avoiding physical-atlas seams.

Press **P** in the Dungeon to switch between two prewarmed materials. The swap
never compiles a gameplay pipeline. Three r185 builds diffuse color before AO
and lazy lighting normals, so the Dungeon marches once there, publishes one
shader-local UV, and consumes it for albedo, normal, roughness, and AO. This
keeps moss/stone color registered with its normal and mask detail without
tripling the march.

On the 680M at 2880×1800 physical pixels, the close angled-wall validation held
59.97 FPS, p99 16.675 ms, and 0/300 frames below 55 FPS with POM enabled.
