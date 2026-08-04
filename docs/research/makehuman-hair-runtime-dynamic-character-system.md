# MakeHuman Hair in a Runtime Dynamic Character System

Date: 2026-08-02

## Research question

Can MakeHuman hair fit arbitrary runtime body shapes, and can it have runtime
physics?

## Result

**Yes, but an exported hair mesh is not sufficient.** MakeHuman hair is a
separate `MHCLO` surface-wrap asset. MakeHuman recalculates its vertices from
the deformed `hm08` basemesh. This is why one hairstyle can fit different head
and body shapes inside MakeHuman.

The standard export process bakes the current fit. It does not export the
MakeHuman fitting operation as a standard glTF operation. Thus, an exported
hair mesh does not continue to refit when Afterglow changes body morphs.

Most standard hair has no independent hair bones. MakeHuman can interpolate
weights from the body rig, so the hair follows the head, neck, shoulders, and
other body bones. These weights give attachment, not secondary-motion physics.

MPFB supports an optional asset sub-rig. The upstream `ponytail01` asset has a
six-bone sub-rig and custom weights. The sub-rig is an animation structure. It
does not contain a spring simulation, physics parameters, or body colliders.
Afterglow must supply those runtime functions.

The recommended Afterglow design has two generic engine mechanisms:

1. **SurfaceWrap:** fit a child mesh to a runtime-deformed parent surface.
2. **SpringChain:** apply bounded secondary motion to an authored bone chain.

Do not bake all body morph targets into every hairstyle. Keep one compact
MakeHuman driver surface per character and evaluate the original wrap map when
the structural body shape changes.

## Direct answers

### Are the hair assets generic?

They are generic **inside the `hm08` MakeHuman body family**. They are not
universal meshes.

A compatible hair asset depends on:

- The exact `hm08` vertex order.
- The `hm08` body and helper geometry.
- Six body vertices used for X, Y, and Z scale measurements.
- A mapping from each hair vertex to three `hm08` vertices.

The mapping does not directly work with:

- An unrelated character mesh.
- A reordered or remeshed `hm08` mesh.
- A PunkElvs proxy without a conversion step.
- A body LOD with different vertex identifiers.

The hair metadata can include Male, Female, Short, Long, or style tags. These
tags are library filters. The common `hm08` topology makes cross-sex fitting
technically possible, but an artist did not approve each result.

### Does hair fit head-size morphs?

Yes, in MakeHuman and MPFB when the asset refit operation runs. The mapping is
evaluated against the body after the active shape targets are combined.

The local audit applied `head-scale-horiz-incr` to all ten standard hair assets.
Almost every hair vertex moved. Maximum movement was from 0.0152 to 0.0255
Blender units, depending on the style.

This result does not mean that all extreme shapes look good. Surface wrapping
is geometric attachment. It is not an artist-made corrective shape, collision
solve, or new hair style.

### Do the assets have bones for physics?

The local CC0 system-asset archive has no hair `.mhw`, `.jsonw`, or
`.mpfbskel` files. Its hair assets receive interpolated weights from the main
body rig. They have no independent hair-bone chains.

The upstream `makehuman-assets` repository is different. It adds these files to
`ponytail01`:

- `ponytail01.mpfbskel`.
- `ponytail01.mhw`.

The sub-rig has one root and five ponytail bones. The root has a Child Of
constraint to the head. The weight file has the five deform groups plus an
`mhmask-subrig` group.

This gives an authored deformation chain. It does not make the chain physical.
A runtime spring solver must rotate the bones and resolve collisions.

# How MakeHuman hair works

## Hair is a clothes-format asset

MakeHuman uses the same fitting technology for:

- Clothes.
- Hair.
- Eyes.
- Teeth.
- Body parts.
- Complete replacement proxies.

A hair asset normally contains:

- An OBJ mesh.
- An `MHCLO` fitting map.
- An `MHMAT` material.
- One or more textures.
- A thumbnail.
- Optional custom body-rig weights.
- An optional MPFB sub-rig and sub-rig weights.

The directory name tells the application that the asset is hair. The geometric
fitting format is the same format that clothes use.

## Vertex mapping

For each hair vertex, `MHCLO` stores:

- Three parent `hm08` vertex identifiers.
- Three interpolation weights.
- A three-component offset.

It also stores three scale records. Each record identifies two parent vertices
and one reference distance.

The fitted hair position is:

```text
P = w0 * B[i0] + w1 * B[i1] + w2 * B[i2] + S(B) * d
```

Where:

- `B` is the deformed, unposed `hm08` surface.
- `i0`, `i1`, and `i2` are parent vertex identifiers.
- `w0`, `w1`, and `w2` are mapping weights.
- `d` is the authored offset.
- `S(B)` is an X/Y/Z scale matrix from the six scale-reference vertices.

The three weights sum to approximately one. They are not limited to the
zero-to-one range. The local files contain weights from -3.0678 to 5.1426.
Afterglow must not clamp these values.

The offset is necessary because much of the hair is not on the skin. A long
strand can use a parent triangle and keep a large distance from it.

## Body and helper anchors

The standard styles use two anchor types:

- Short styles and some ponytail vertices use body and scalp vertices.
- `bob01`, `braid01`, and `long01` use the `helper-hair` surface.

The helper surface is hidden geometry in the MakeHuman basemesh. It exists to
help clothes and hair fitting. A runtime implementation that keeps only the
visible PunkElvs mesh loses these source anchors.

Afterglow has two possible solutions:

1. Keep a compact hidden `hm08` driver surface.
2. Re-author each wrap map against the visible runtime proxy.

The recommended default is the compact `hm08` driver. It preserves the source
asset meaning and does not depend on the visible body LOD.

## Fit update in MakeHuman

The MakeHuman source applies all active targets to its seed mesh, updates its
rest-pose coordinate copy, and then updates proxies. The proxy implementation
evaluates all mapped coordinates and recalculates normals.

MPFB performs the same logical operation in Blender:

1. Create a temporary shape key from the current target mix.
2. Read the deformed parent vertices.
3. Evaluate every mapped child vertex.
4. Update the child mesh basis.
5. Remove the temporary shape key.

MPFB can do this after a manual command or through automatic refit. It is an
authoring operation, not a web runtime.

## Rig-weight interpolation

The geometric wrap weights and skeleton weights are different data.

When no custom skeleton weights exist, MakeHuman or MPFB derives them:

1. Read body-bone weights at the three mapped parent vertices.
2. Multiply each body-bone weight by the corresponding wrap weight.
3. Add values by bone name.
4. Remove very small results.
5. Normalize or prepare the final skin weights.

As a result, short hair usually follows the head bone strongly. Long hair can
also receive neck, spine, shoulder, arm, pelvis, or thigh weights because its
anchors extend down the body.

This behavior keeps static long hair near the body during animation. It does
not produce inertia, swing, wind, or collision response.

# Local asset audit

## Source

The repository contains:

`assets/character-rig/downloads/makehuman_system_assets_cc0.zip`

The archive labels the standard assets CC0. It contains ten card-mesh hair
styles.

## Geometry and mappings

| Style | Vertices | Faces | Unique parent anchors | Main anchor type | Local sub-rig |
|---|---:|---:|---:|---|---|
| `afro01` | 2,196 | 1,096 | 528 | Body/scalp | No |
| `bob01` | 5,203 | 4,237 | 261 | Hair helper | No |
| `bob02` | 1,653 | 1,124 | 907 | Body/scalp | No |
| `braid01` | 4,493 | 2,759 | 289 | Hair helper | No |
| `long01` | 3,239 | 2,054 | 337 | Hair helper | No |
| `ponytail01` | 3,718 | 2,676 | 513 | Body/scalp and helper | No |
| `short01` | 2,984 | 1,839 | 504 | Body/scalp | No |
| `short02` | 1,755 | 1,672 | 706 | Body/scalp | No |
| `short03` | 1,011 | 961 | 673 | Body/scalp | No |
| `short04` | 865 | 525 | 613 | Body/scalp | No |
| **Total** | **27,117** | **18,943** | — | — | — |

All 27,117 hair vertices use a three-parent weighted mapping. None uses the
single-parent exact form.

Across all ten styles, the maps reference only 1,601 unique `hm08` vertices.
The six scale-reference vertices increase the compact driver set to 1,602
because five are already in the map union.

A full 19,000-vertex hidden body is therefore not necessary for hair fitting.
A cooked compact driver needs approximately 19.2 KiB of float32 positions per
character, before alignment and state data.

## Textures and materials

Each style has one 2048 by 2048 RGBA diffuse texture. `short02` also has a
normal texture in the archive.

The MakeHuman materials use:

- Transparency.
- Alpha-to-coverage.
- No back-face culling.
- Hair-specific lit-sphere shaders.
- Non-PBR color and specular settings.

These materials are not production glTF PBR hair materials. The cook must
convert them to the Afterglow material model. The engine must not depend on the
MakeHuman lit-sphere shader.

## Empirical fit test

Test environment:

- Blender 5.2.0 LTS.
- MPFB 2.0.17.
- Standard game-engine body rig.
- Neutral body, then `head-scale-horiz-incr = 1`.
- The ten local CC0 hair assets.

| Style | Moved vertices | Maximum movement | MPFB neutral fit | MPFB changed fit |
|---|---:|---:|---:|---:|
| `afro01` | 2,152 / 2,196 | 0.019440 | 5.993 ms | 6.427 ms |
| `bob01` | 5,199 / 5,203 | 0.025502 | 13.357 ms | 14.083 ms |
| `bob02` | 1,653 / 1,653 | 0.019059 | 4.647 ms | 5.066 ms |
| `braid01` | 4,463 / 4,493 | 0.022496 | 11.787 ms | 12.213 ms |
| `long01` | 3,231 / 3,239 | 0.023773 | 8.570 ms | 9.100 ms |
| `ponytail01` | 3,690 / 3,718 | 0.015361 | 9.670 ms | 10.391 ms |
| `short01` | 2,973 / 2,984 | 0.015243 | 7.784 ms | 8.445 ms |
| `short02` | 1,755 / 1,755 | 0.017887 | 5.058 ms | 5.074 ms |
| `short03` | 1,011 / 1,011 | 0.018843 | 3.093 ms | 3.144 ms |
| `short04` | 862 / 865 | 0.015162 | 2.736 ms | 2.747 ms |

These times measure Blender Python and Blender mesh updates. They are not an
Afterglow runtime benchmark. They only show that the standard reference
operation changes hair when head size changes.

## Driver-morph storage audit

The 592 canonical MPFB direct targets touch the compact hair driver as follows:

- 319 targets change at least one driver vertex.
- Total sparse driver-delta entries: 82,584.
- Maximum entries in one target: 1,602.

The CC0 face-unit and viseme files add 13,937 driver entries. Many of these
entries come from jaw and mouth movement near lower hair anchors.

At 16 bytes per sparse entry, the direct structural data is approximately
1.3 MiB before the face targets. This immutable data is shared by all
characters. It is much smaller than a complete morph copy for each hairstyle.

Do not include expression and speech targets in hair rest fitting. Expressions
are transient animation state. Hair rest shape must follow structural body
state only.

## Upstream ponytail sub-rig

The upstream `makehumancommunity/makehuman-assets` repository added a
`ponytail01` MPFB sub-rig on 2022-12-17.

It contains:

- `root`.
- `ponytail`.
- `ponytail1`.
- `ponytail2`.
- `ponytail3`.
- `ponytail4`.
- 10,748 nonzero entries across the sub-rig weight groups and mask.

The root follows the main head bone. The five-bone chain uses fitted head and
tail strategies based on hair vertices. This is useful source data for a
runtime rig.

The repository's local system-asset ZIP does not contain these two sidecars.
The current generation path must fetch or vendor the upstream CC0 sidecars if
it uses this rig.

A local MPFB 2.0.17 import test of the upstream 2022 sub-rig failed in the
Rigify-layer upgrade path. Thus, the production cook must not assume that a
current Blender session can import this old sidecar without a conversion or
fix. Direct JSON conversion is a valid alternative.

# Why a normal export is not runtime-dynamic

## glTF keeps the result, not the MakeHuman relation

A normal glTF export can contain:

- Hair positions for the current fitted body.
- Hair normals, tangents, UVs, and material data.
- Main skeleton skin weights.
- An optional hair sub-rig.

Standard glTF has no `MHCLO` surface-wrap operation. Unless the exporter bakes
hair morph targets, the parent-triangle references and scale references are
lost.

If Afterglow then changes only body morph weights, the body changes but the
hair rest mesh does not. The result can clip, float, or expose the scalp.

## Duplicating body morphs is the wrong default

One possible cook is to refit each hairstyle for every body target and export
all results as hair morph targets. This is the same method that the current
prototype uses for its two genital body proxies.

It is not a good hair-library default:

- Every hairstyle duplicates hundreds of target streams.
- Hair selection loads many unrelated morph accessors.
- Storage grows with `hair styles * targets * affected vertices`.
- Runtime GPU morph state becomes larger.
- Every new body target needs every hairstyle to be recooked.
- Expression targets can cause unwanted hair deformation.
- Hair-bone rest pivots and colliders still need shape updates.

The current male body GLB is approximately 25.4 MB and the female body GLB is
approximately 18.9 MB. Repeating a similar target set across each hair asset is
not KISS.

## Morph and skin order

The glTF 2.0 specification states that morph position, normal, and tangent
deltas are applied before skinning and node transforms.

A runtime wrap has the same logical place:

1. Evaluate structural body shape.
2. Evaluate hair rest fit.
3. Apply body-rig and hair-rig skinning.
4. Apply node transforms.

Spring simulation changes the hair-rig transforms used in step 3. It does not
replace the rest-fit operation in step 2.

# Runtime architecture options

## Option A: bake all body targets into each hair asset

**Advantages**

- Uses standard glTF morph targets.
- Uses the current Three.js morph path.
- Needs no custom wrap evaluator.

**Problems**

- Large duplicated data.
- Slow asset cook and load.
- Poor scaling with many hairstyles.
- Hair rig and collision data still need dynamic shape support.

**Decision:** Reject as the general system. It is acceptable only for one
small, fixed asset with a very small target set.

## Option B: CPU SurfaceWrap

Keep the compact driver positions in a preallocated typed array. When a
structural control changes:

1. Update affected driver vertices from sparse deltas.
2. Recalculate the three scale values.
3. Evaluate each hair vertex from its three anchors.
4. Recalculate or update normals.
5. Upload the changed position data.

**Advantages**

- Simple and faithful to MakeHuman.
- Small immutable data.
- Easy validation against MPFB.
- Hair fit changes only when structural shape changes.
- Can update fitted hair-bone rest positions in the same task.

**Problems**

- Position and possibly normal data need an upload after each shape change.
- Each fitted character needs mutable output storage.
- Main-thread cost is unknown until a browser benchmark exists.

**Decision:** Recommended first measured prototype.

## Option C: vertex-shader SurfaceWrap

Store driver positions in a GPU buffer. Give each hair vertex its three compact
anchor indexes, weights, and offset. Evaluate the wrap in the hair vertex
shader before skinning.

**Advantages**

- No fitted-position upload for each style.
- The immutable hairstyle mesh remains shared.
- Per-character driver data is small.
- Live character editing can update only the driver buffer.

**Problems**

- Needs a custom Three.js WebGPU node path.
- Normal and tangent fitting is not directly defined by `MHCLO`.
- Hair-bone rest positions and inverse bind matrices still need CPU updates.
- Per-instance driver binding can limit batching.
- The interaction with standard skinning must be validated on WebGPU.

**Decision:** Measure after Option B. Select it only if the CPU path misses its
frame or upload budget.

## Option D: GPU compute refit

A compute pass writes fitted positions and normals to a dynamic vertex buffer.

**Advantages**

- Can recalculate triangle normals.
- Keeps large fit work off the CPU.
- Can process many changing characters.

**Problems**

- More synchronization and buffer management.
- More difficult Three.js integration.
- Unnecessary until a measured CPU or vertex path fails.

**Decision:** Do not implement without a measured need.

## Option E: fit in a worker

A native or web worker can evaluate a fixed wrap task and return fitted data
through the existing RingBuffer transport.

**Advantages**

- Removes fit work from the page thread.
- Uses the same deterministic formula on both targets.

**Problems**

- The web target copies result data from worker memory.
- The page still uploads the result to the GPU.
- A worker adds scheduling delay for a pointer-driven edit.

**Decision:** Use only if the page-thread Option B benchmark fails and the
delay remains acceptable.

# Recommended generic engine model

## SurfaceWrap

`SurfaceWrap` must be a policy-free child-surface mechanism. Hair, clothes,
armor liners, eyes, and body parts can use it.

Cooked immutable data:

- Parent surface identifier.
- Compact parent-vertex table.
- Child geometry identifier.
- Three compact parent indexes per child vertex.
- Three signed float weights per child vertex.
- Three-component offset per child vertex.
- X/Y/Z scale-reference records.
- Coordinate-system and format version.
- Bounds for validation.

Runtime instance data:

- Parent shape revision.
- Driver positions.
- Fitted output positions when the CPU backend is used.
- Current fit revision.
- Pending/publication state.
- Fixed error and overflow state.

The runtime must never parse `MHCLO` text. The offline pipeline must convert it
to a validated binary record.

## CharacterShape

The character must have one structural shape state that is independent of
expression state.

Structural state includes:

- Sex and ethnicity composition.
- Age, height, weight, and muscle.
- Head, face, torso, limb, hand, foot, and genital structure.
- Persistent asymmetry.

Animation state includes:

- Expressions.
- Speech visemes.
- Temporary gaze and pose controls.

Only a structural revision updates SurfaceWrap rest geometry, skeleton rest
data, and collision shapes.

## Main rig weights

At cook time, transfer main skeleton weights through the wrap map. Keep a fixed
maximum number of bone influences per hair vertex. Normalize after removal of
small weights.

Short hair can use only these main-rig weights. It needs no secondary-motion
rig.

## Hair rig

Long moving sections need an authored hair rig. The rig record must include:

- Root attachment to a main skeleton bone.
- Acyclic bone-chain topology.
- Rest head and tail fit rules.
- Hair vertex weights.
- Optional terminal nodes.
- Spring parameters.
- Collider groups.
- Simulation LOD class.

Do not automatically infer a production rig from mesh length. Card topology,
style flow, tied sections, and desired motion are artist decisions.

## SpringChain

Use the VRM 1.0 `VRMC_springBone` model as the source model for the first
implementation.

It supplies:

- A chain of bone nodes.
- Verlet-style tail state.
- Stiffness.
- Gravity direction and strength.
- Drag.
- Joint hit radius.
- Sphere and capsule colliders.
- Collider groups.
- Optional center space.

This model is sufficient for ponytails, braids, loose card groups, tails, and
small cloth parts. It is not strand-level hair simulation.

The VRM specification is complete and written against glTF 2.0. The MIT
`pixiv/three-vrm` project supplies a Three.js reference implementation.
Afterglow can use it for prototype comparison.

The production engine should use a fixed-capacity data layout. The current
`three-vrm` manager uses Sets, Maps, arrays, and dynamic object graphs during
setup and topology changes. Its normal joint update reuses vector and matrix
objects, but runtime hairstyle replacement can cause allocation and sorting.
This conflicts with sealed-mode Afterglow rules unless it runs behind a tracked
slow-path permit.

## Collision shapes

Spring chains need morph-aware body colliders. Use a small authored set:

- Head sphere or capsule.
- Neck capsule.
- Left and right shoulder capsules.
- Upper torso capsule.
- Optional back or chest capsule for long hair.

Collider transforms follow main skeleton bones. Collider radii and local
offsets must also follow structural shape metrics. A fixed neutral collider set
is not sufficient for large head, neck, shoulder, or chest changes.

Do not collide spring bones with the complete skinned body mesh. A small fixed
collider set is bounded and predictable.

# Runtime update order

For a normal animation frame:

1. Read the current published structural shape.
2. Update the main animation pose.
3. Update main skeleton world transforms.
4. Update attached collider transforms.
5. Run fixed-step SpringChain updates from root to tip.
6. Compose final hair-bone matrices.
7. Render wrapped and skinned hair.

When structural shape changes:

1. Apply sparse deltas to the compact parent driver.
2. Refit hair rest vertices or publish the new GPU driver.
3. Refit hair-bone rest heads and tails.
4. Update inverse bind data if applicable.
5. Update shape-dependent collider offsets and radii.
6. Reset spring previous and current tails to the new rest chain.
7. Publish all new data in one revision.

Do not keep old spring velocity after a large body-shape change. It can cause an
explosive chain movement. Reset is the safe default.

During a continuous editor drag, one of these policies is necessary:

- Disable secondary motion and refit live, then restart it on release.
- Refit and reset at a fixed low rate during drag.
- Keep the last committed fit during drag and publish on release.

The recommended default is live rest fitting with secondary motion disabled for
the selected character. Restart and fade in secondary motion after release.

# Runtime selection and sharing

## Hair change

A hairstyle change must be a bounded transaction:

1. Request the cooked hair asset.
2. Reserve a fixed hair instance slot.
3. Load immutable mesh, map, rig, and material data.
4. Fit against the current character shape.
5. Initialize rig, colliders, and spring state.
6. Publish the new hair instance.
7. Release the old slot.

If any stage fails, keep the old hairstyle. Do not publish a partially fitted
mesh.

## Geometry sharing

Static index, UV, mapping, weight, and material data can be shared by all
instances of one hairstyle.

CPU-fitted position output is shape-specific. Each different character shape
needs its own output slot. A GPU wrap path can share the base hair data and bind
a different small driver buffer per character.

## Simulation LOD

Recommended behavior:

- **Near or edited character:** complete fixed-step spring simulation.
- **Middle distance:** lower update rate or fewer active chains.
- **Far distance:** main-rig skinning only.
- **Offscreen:** no spring update, with reset or bounded catch-up on return.

The transition must be deterministic. Do not perform unbounded catch-up steps.

# Rendering requirements

The standard assets are alpha-card hair. Their source material asks for
alpha-to-coverage and two-sided rendering.

Recommended first material:

- Alpha test or alpha hash for the main card body.
- Alpha-to-coverage when MSAA is active and validated.
- Two-sided card shading only where necessary.
- An anisotropic hair response in the production material.
- No general sorted alpha blend for the complete hairstyle.
- Cooked mipmaps with alpha-coverage preservation.

The source lit-sphere material is not sufficient for Afterglow PBR lighting.
Hair material conversion needs its own visual acceptance gate.

Normals need special care. The reference CPU fit recalculates them. A pure
vertex-shader wrap cannot read adjacent fitted vertices to produce geometric
normals. The prototype must compare:

1. Static authored card normals.
2. CPU-recalculated normals after a shape commit.
3. A compute-generated normal buffer.

Use the simplest option that has no visible failure across the accepted shape
range.

# Failure modes

## Geometric fit failures

- Hair clips into a large or asymmetric head.
- Hair floats above a small head.
- Hair-helper offsets stretch long cards.
- Ears pass through short hair.
- Long hair passes through shoulders or breasts.
- Extreme body changes distort card spacing.
- A helper-based asset fails when helper vertices are absent.

Mitigation:

- Validate every asset against an offline body-shape sweep.
- Add asset-specific structural correction targets only after a measured
  failure.
- Add fit-range metadata.
- Reject unsupported combinations deterministically.

## Rig failures

- Main-rig weights pull long hair with arms or thighs.
- A sub-rig root does not follow the fitted head.
- A bone pivot remains at the neutral shape.
- Hair weights refer to missing bones.
- A spring chain has no terminal node.

Mitigation:

- Require an authored rig for moving sections.
- Validate every bone and weight at cook time.
- Refit rest pivots after structural changes.
- Use a rigid main-rig fallback when a sub-rig is invalid.

## Physics failures

- Variable frame time changes spring behavior.
- A teleport creates excessive inertia.
- A shape change retains old velocity.
- Large time debt causes unbounded catch-up.
- Colliders remain at neutral body dimensions.

Mitigation:

- Use a fixed simulation step and a hard substep limit.
- Reset on teleport, publication, or large time debt.
- Keep center space near the head for hair chains.
- Recalculate morph-aware collider metrics.

## Rendering failures

- Sorted transparent cards reveal wrong layer order.
- Alpha mips make hair disappear.
- Two-sided lighting is too bright.
- Static normals look incorrect after a large fit.
- Hair casts a dense rectangular shadow.

Mitigation:

- Prefer tested masked or hashed transparency.
- Preserve alpha coverage in mips.
- Use a hair-specific shadow alpha threshold.
- Validate normals and two-sided response on WebGPU.

# Capacity and allocation rules

Before `GameplaySealed`, reserve fixed pools for:

- Hair instances.
- Compact driver positions.
- Fitted CPU output buffers.
- Hair bones and terminal nodes.
- Spring state.
- Sphere and capsule colliders.
- Pending hair-change transactions.
- Undo or editor shape revisions.

After sealing:

- A structural control update must not create arrays or objects.
- A hair swap must use a reserved transaction and instance slot.
- A failed reservation must keep the current hair.
- Spring updates must use fixed arrays and fixed iteration limits.
- No stage can scan all prior character revisions.
- Telemetry must report pool use, fit time, upload bytes, active joints,
  collider tests, resets, and rejected changes.

Exact capacities are product policy. Do not lock them before the target
character count and hair library are selected.

# Permissive implementation audit

## Result

Permissive implementations exist. **Decision locked 2026-08-02:** Humentity and
`bevy_make_human` are the co-primary **N1 implementation references**. Both are
available under the user's choice of MIT or Apache-2.0.

Afterglow does not need to derive runtime code from GPL or AGPL source. It can
adapt these permissive fitting implementations, keep the required license
notices, and validate the result against the public `MHCLO` format and MPFB
output.

No permissive implementation found in this audit is a correct drop-in
Afterglow component. The fitting loop is small, but each project has different
coordinate, allocation, mesh, and rig assumptions.

## Humentity

Repository:

<https://github.com/emberlightstudios/Humentity>

Pinned audit revision:

`1cd7b005d73ec07c9fa1127fdee6059564dcb8d1`

License:

- MIT or Apache-2.0, at the user's choice.
- The same dual license existed when its current `MHCLO` parser first appeared
  in revision `7785791bfd76b5906058dcf06fae5d260fc75da2`.

Relevant functions:

- `src/loaders/mhclo.rs` parses exact and three-parent mappings, X/Y/Z scale
  records, tags, deletion ranges, and the `material` line inside the vertex
  section.
- `src/assets.rs::shape_mesh_from_helpers_mhclo` evaluates the weighted parent
  positions and scaled offset.
- The same function recalculates normals and tangents and corrects duplicated
  OBJ vertices at UV seams.
- `src/rigs.rs::set_asset_rig_arrays` transfers body-rig weights through the
  `MHCLO` map.
- The project builds body parts, clothes, and hair at runtime in Bevy.

The geometric fit follows the necessary operation:

```text
P = sum(parent_position[i] * map_weight[i]) + axis_scale * map_offset
```

Humentity supplies the more complete end-to-end path, but its current code
still needs correction for Afterglow:

- It allocates Bevy meshes, vectors, maps, and sets during mesh construction.
- It normalizes the three source map weights. Official files already sum to
  approximately one, and Afterglow can preserve the source values.
- Its rig-weight code pads fewer than four influences by repeating the first
  influence before normalization. With two or three different bones, this can
  change their effective relative weights.
- It has synthetic and application use, but this audit found no complete
  MPFB-parity suite for the ten standard hair assets.
- Its runtime template model bakes a small selected morph set. It is not the
  fixed-array live refit system that Afterglow needs.

Thus, use Humentity for the complete path and `bevy_make_human` for an
independent implementation comparison. Do not import either Bevy runtime, and
do not copy Humentity's rig-weight code without a correction.

## bevy_make_human

Repository:

<https://github.com/slyedoc/bevy_make_human>

Pinned audit revision:

`6d6652051db9795f4bb0f7b3ef5f27cbe217316c`

License:

- MIT or Apache-2.0, at the user's choice.
- Included MakeHuman assets are identified separately as CC0.

Its `src/loaders/mhclo.rs` contains both a parser and
`MhcloAsset::apply_to_base`. The function evaluates three-parent interpolation,
axis scale, offsets, and an optional triangle-normal push.

This is the second co-primary N1 implementation reference. It is also an early
WIP, and its README says that customization is not yet ideal. Its coordinate
signs and 0.1 unit conversion are specific to its Bevy OBJ path. The optional
normal push is not part of the source `MHCLO` fit and must not become an
implicit Afterglow policy.

## OxiHuman

Repository:

<https://github.com/cool-japan/oxihuman>

Pinned audit revision:

`603b446854c3d5a9ca478214e7b85008d54786b9`

License:

- Apache-2.0.

OxiHuman explicitly labels its clothing fit as an independent implementation
of documented `MHCLO` semantics. This is useful provenance evidence, but its
current implementation is incomplete:

- It interprets `verts 0` as an expected vertex count. In standard `MHCLO`, the
  zero is the start index, not the number of mappings.
- It accepts only nine-field mappings and not exact one-field mappings.
- It does not parse or apply X/Y/Z offset scales.
- It keeps source normals instead of recalculating fitted normals.

A standard hair file has `verts 0` and then thousands of mappings. The current
OxiHuman parser will reject that file because it expected zero mappings.
Therefore, do not use this implementation as the correctness base.

## Other implementations

| Project | License result | Technical result | Decision |
|---|---|---|---|
| `makehuman.js` by Mark-André Hopf | AGPL-3.0-or-later | Complete TypeScript proxy system | Do not copy into Afterglow |
| `makehuman-js/makehuman-js` | Package states AGPLv3 | Browser character library | Do not copy into Afterglow |
| `NitroxNova/humanizer` | AGPL-3.0 | Runtime Godot character system | Do not copy into Afterglow |
| `pdcamargo/retro-engine` | No license file or package license | Good allocation-free TypeScript fitter and tests | No permission to copy |
| `slyedoc/bevy_make_human` | MIT or Apache-2.0 | Complete basic fitter, early WIP | Co-primary N1 reference |
| `emberlightstudios/Humentity` | MIT or Apache-2.0 | Most complete permissive fitter | Co-primary N1 reference |
| `cool-japan/oxihuman` | Apache-2.0 | Incomplete standard-file support | Provenance only |

A public repository without a license does not give reuse permission. The
Retro Engine code cannot be used unless its owner adds a suitable license or
gives explicit permission.

## Recommended license path

1. Use Humentity and `bevy_make_human` under MIT or Apache-2.0 as the licensed
   N1 implementation references.
2. Keep both copyright and license notices in Afterglow's third-party notices.
3. Compare both implementations before adapting the geometric semantics to
   fixed TypeScript arrays.
4. Use the public MakeHuman format documents to define accepted input.
5. Use MPFB only as an offline golden-output oracle.
6. Write Afterglow-specific parsing, validation, packing, update, and telemetry
   code.
7. Do not inspect or translate the AGPL TypeScript implementations during that
   work.

This path is legally clearer than making pseudocode from GPL code. MIT and
Apache-2.0 expressly permit modification and redistribution when their terms
are obeyed. A legal review is still recommended for the final notice and
provenance record.

## Correctness gates for the permissive port

The port is not accepted until golden tests cover:

- All ten standard CC0 hair assets.
- Exact one-parent and weighted three-parent mappings.
- Signed weights below zero and above one without clamping.
- The `material` line that occurs after `verts 0` in standard hair files.
- All three scale records and coordinate conversion.
- Neutral, head-width, head-height, head-depth, age, sex, weight, and
  asymmetry shapes.
- OBJ vertices split at UV seams.
- Recalculated positions, normals, and tangents.
- Correct top-four skeleton-weight selection and normalization without
  duplicated influence padding.
- Bitwise-stable packed records and bounded runtime errors.

For each test shape, compare Afterglow positions with a Blender/MPFB fixture.
Use an explicit maximum position error and normal-angle error. Do not approve
the port from code review alone.

# Recommended implementation path

## Gate 1: offline conversion

Add a cook-only `MHCLO` converter that:

- Accepts only approved `hm08` assets.
- Converts MakeHuman coordinates to engine coordinates.
- Remaps global parent indexes to a compact driver table.
- Preserves signed weights without clamping.
- Validates finite values and approximately unit weight sums.
- Converts source materials and textures.
- Transfers main-rig weights.
- Writes a versioned generic SurfaceWrap record.

Golden tests must compare cooked output with the MPFB fit for all ten standard
styles and a set of structural targets.

## Gate 2: CPU runtime prototype

Implement a fixed-array reference evaluator in the prototype. Test:

- Neutral fit.
- Head width, height, depth, translation, and angle.
- Age, sex, ethnicity, height, weight, and muscle.
- Ear and jaw structural extremes.
- Persistent asymmetry.
- Hair selection during gameplay.
- Repeated edits without heap growth.

Measure page-thread time, upload bytes, p99 frame time, and fit latency on the
Radeon 680M.

## Gate 3: backend decision

If the CPU path meets the budget, keep it. If it fails, measure the vertex
shader path. Use compute or a worker only after both simpler paths have an
identified failure.

This is a technical acceptance gate, not a product decision.

## Gate 4: spring-bone prototype

Use upstream `ponytail01` as the first rigged test asset. Convert its root and
five-bone chain without depending on Blender's old Rigify upgrade path.

Compare:

- A small fixed-array VRM-compatible solver.
- The MIT `three-vrm-springbone` reference result.

Test fixed-step stability, head-local center space, teleport reset, shape-change
reset, and sphere/capsule collision.

## Gate 5: asset acceptance

Classify each style:

- Rigid short hair.
- Main-rig weighted long hair.
- Spring-rigged hair.
- Rejected until corrected.

The upstream ponytail can enter the spring-rigged class after conversion and
visual validation. `long01` and `braid01` need authored motion rigs before they
can use physics. An automatic chain is not sufficient evidence.

## Gate 6: production integration

Only after the prior gates pass:

- Add generic SurfaceWrap and SpringChain API records.
- Add fixed runtime pools and telemetry.
- Add native and web target tests.
- Add the character editor hair library.
- Add release soaks with repeated body edits and hair swaps.

# Prototype result (2026-08-05)

The isolated character editor now has a CPU SurfaceWrap slice for CC0 `short04`
and `ponytail01`. It keeps a shared 733-vertex compact driver and 201 sparse
structural target streams. Expressions and visemes do not change hair rest fit.

Both styles use interpolated main-rig weights. The ponytail has no secondary
motion yet. A fitted 376-vertex `scalp` cap covers transparent card gaps. The
first cap test incorrectly used the large `helper-hair` fitting cage and looked
like a second hairstyle. That path was removed.

MPFB parity validation accepts a neutral maximum error below `3e-6` Blender
units and checks live movement for `head-scale-horiz-incr`. A 100-update browser
diagnostic measured 0.202 ms mean for `short04` and 0.630 ms for `ponytail01`,
including normal and bound rebuilds. These numbers are prototype evidence only.

This does not complete the production gates. The code remains under
`prototype/character-editor/` and is not an engine crate or public API.

# Decisions before implementation

The following product decisions remain open:

1. **Hair quality target.** Recommended: use the ten CC0 assets as system tests
   and starter content, not final production-quality hair.
2. **Physics scope.** Recommended: spring bones for authored long sections,
   rigid main-rig skinning for short hair, and no strand simulation.
3. **Runtime edit scope.** Recommended: permit live structural editing and hair
   replacement during gameplay, with secondary motion disabled during a drag.
4. **Asset policy.** Recommended: require authored rig and collider metadata for
   each physically moving hairstyle.
5. **Character count.** The user must give the maximum nearby simulated
   characters and maximum concurrent character edits before pool capacities are
   selected.

The CPU-versus-GPU fit backend is not a user decision. Select it with the
measured gates above.

# Sources

## MakeHuman and MPFB

- MakeHuman file formats and `MHCLO` mapping:
  <https://static.makehumancommunity.org/oldsite/documentation/file_formats_and_extensions.html>
- MPFB `MHCLO` file-format reference:
  <https://github.com/makehumancommunity/mpfb2/blob/47deded84aba2c43238cc90efdcf0421ba7c5f46/docs/fileformats/mhclo.md>
- MPFB ClothesService source:
  <https://github.com/makehumancommunity/mpfb2/blob/47deded84aba2c43238cc90efdcf0421ba7c5f46/src/mpfb/services/clothesservice.py>
- MakeHuman proxy evaluator source:
  <https://github.com/makehumancommunity/makehuman/blob/a8bc2d54ff0ac92e78ff71431b1023eda42bf482/makehuman/shared/proxy.py>
- MakeHuman target application and proxy update source:
  <https://github.com/makehumancommunity/makehuman/blob/a8bc2d54ff0ac92e78ff71431b1023eda42bf482/makehuman/apps/human.py>
- MakeHuman hair creation FAQ:
  <https://static.makehumancommunity.org/assets/creatingassets/faq/how_can_i_create_hair.html>
- MakeHuman clothes, hair, and body-part concept:
  <https://static.makehumancommunity.org/mpfb/docs/assets/concept_clothes_hair_bodyparts.html>
- MPFB mesh-asset sub-rig tutorial:
  <https://static.makehumancommunity.org/mpfb/docs/rigging_mesh_assets.html>
- CC0 MakeHuman system assets:
  <https://github.com/makehumancommunity/makehuman-assets/tree/8cf9645b975a98eea056b140df11a1d278da0d10/base/hair>
- Upstream ponytail sub-rig commit:
  <https://github.com/makehumancommunity/makehuman-assets/commit/6349afc4e656fe2c591778464f9304d2527ec613>

## Permissive implementations

- Humentity dual MIT/Apache-2.0 license:
  <https://github.com/emberlightstudios/Humentity/blob/1cd7b005d73ec07c9fa1127fdee6059564dcb8d1/LICENSE>
- Humentity `MHCLO` parser:
  <https://github.com/emberlightstudios/Humentity/blob/1cd7b005d73ec07c9fa1127fdee6059564dcb8d1/src/loaders/mhclo.rs>
- Humentity fitting function:
  <https://github.com/emberlightstudios/Humentity/blob/1cd7b005d73ec07c9fa1127fdee6059564dcb8d1/src/assets.rs>
- Humentity rig-weight transfer:
  <https://github.com/emberlightstudios/Humentity/blob/1cd7b005d73ec07c9fa1127fdee6059564dcb8d1/src/rigs.rs>
- `bevy_make_human` dual MIT/Apache-2.0 license and fitter:
  <https://github.com/slyedoc/bevy_make_human/blob/6d6652051db9795f4bb0f7b3ef5f27cbe217316c/LICENSE-MIT>
  <https://github.com/slyedoc/bevy_make_human/blob/6d6652051db9795f4bb0f7b3ef5f27cbe217316c/src/loaders/mhclo.rs>
- OxiHuman Apache-2.0 independent implementation:
  <https://github.com/cool-japan/oxihuman/blob/603b446854c3d5a9ca478214e7b85008d54786b9/LICENSE>
  <https://github.com/cool-japan/oxihuman/blob/603b446854c3d5a9ca478214e7b85008d54786b9/crates/oxihuman-mesh/src/clothing.rs>

## Runtime standards and references

- glTF 2.0 specification, morph targets and pre-skin application order:
  <https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#morph-targets>
- VRM 1.0 `VRMC_springBone` specification:
  <https://github.com/vrm-c/vrm-specification/blob/00baaa64a2d9cd7742862949ef9fd54be72cb712/specification/VRMC_springBone-1.0/README.md>
- MIT Three.js spring-bone reference:
  <https://github.com/pixiv/three-vrm/tree/cbd9a77a0d17f0099fdac5dcc2b4c7ee30342869/packages/three-vrm-springbone>
