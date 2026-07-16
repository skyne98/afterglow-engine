# Surface detail and POM

The Dungeon combines streamed 8K stone materials with a bounded close-range
parallax occlusion mapping tier.

The engine's `POM_UV_WGSL` performs an adaptive 8–32-layer march with linear
intersection refinement. Physical height (`1` exposed, `0` recessed) is
converted to ray depth with `1 - height`; a ratio-2 offset limit prevents
exploding UVs at grazing angles. Dungeon uses scale 0.05 and intentionally has
no radial per-fragment fade—the moving contour was visible as a flattening wave.
An 8-sample ray toward the point light self-shadows direct diffuse/specular at
82% strength. Hemisphere fill remains unshadowed. There is no silhouette or
secondary relief/depth pass.

Height comes from the official resident 1K, 16-bit ambientCG displacement maps.
The offline pipeline converts the source PNGs into versioned, single-channel
R16 payloads. Runtime expands their exact normalized `u16` levels into a
single-channel filterable WebGPU `r32float` texture; every source level remains
distinct and the data never passes through the browser's 8-bit image path. The
Dungeon fails closed without `float32-filterable`. WebGPU exposes `r16unorm` as
unfilterable, while manual bilinear sampling would quadruple every POM height
read and patching vendored Three would be brittle.

AO describes occluded lighting, not geometry; using it as pseudo-height produced
weak, incorrect relief. Keeping displacement resident prevents page residency
from changing during the non-uniform march; the 8K color, normal, and packed
mask channels remain virtual. Resident displacement uses `flipY=false` to match
VT storage, and the virtual `NormalGL` green channel is inverted at sampling to
correct the custom uploader's top-left row orientation.

Displaced PBR channels use `VT_SAMPLE_FROM_LEVEL_WGSL`: each begins at the
material's resolved mip, walks to a coarser resident page when the coordinate
crosses a missing page, and finally uses the pinned mip tail. Gradients come
from the undisplaced UV, avoiding physical-atlas seams.

Press **P** in the Dungeon to switch between two prewarmed materials. The swap
never compiles a gameplay pipeline. The tangent-space ray uses the geometric
normal rather than Three's normal-map-dependent `parallaxDirection`, avoiding a
cycle where the normal needs the displaced UV that the normal itself influences.
The first material flow publishes one marched UV for albedo, normal, roughness,
and AO. Generated WGSL is checked during warm-up for one correctly ordered
march and three linked PBR samples.

The march equations and sign convention match LearnOpenGL's established POM
implementation; the direct-light ray comes from the known-working prototype.
At 2880×1800, the corrected shader previously measured 59.87 FPS and p99
16.68 ms with 1/600 below 55; base measured 59.77 FPS with 2/600. Moving POM
measured 59.97 FPS with 0/300 below 55. Those measurements used a 16-bit source
PNG but Three's old `rgba8unorm` runtime upload, so they are historical shader
baselines—not validation of the new precision-preserving path. VT feedback runs every 8
frames; a Radeon 680M correctness/performance rerun is required.
