# Runtime character bake and compact runtime plan

**Status:** proposed for user decisions
**Date:** 2026-08-02
**Scope:** built-in character baking, live structural edits, compact finished
characters, fitted equipment, hair, rig rest data, and bounded publication.

Related documents:

- [`character-editor-prototype.md`](character-editor-prototype.md)
- [`unified-paged-resources-completion-plan.md`](unified-paged-resources-completion-plan.md)
- [`no-runtime-allocation-constant-time-budget-plan.md`](no-runtime-allocation-constant-time-budget-plan.md)
- [`comms-unification-plan.md`](comms-unification-plan.md)
- [`../research/makehuman-hair-runtime-dynamic-character-system.md`](../research/makehuman-hair-runtime-dynamic-character-system.md)
- [`../api/static-lod.md`](../api/static-lod.md)
- [`../api/engine-memory.md`](../api/engine-memory.md)
- [`../api/persistent-blob-store.md`](../api/persistent-blob-store.md)

## TLDR

Use two representations.

1. A shared `CharacterSourcePack` contains the complete editable data.
2. A small `CharacterRecipe` contains one character's structural selections.
3. A fixed `CharacterBakeWorker` evaluates the recipe into an immutable
   `CharacterBakeRecord`.
4. The finished runtime character contains only rendered geometry, selected
   facial channels, fitted rig data, materials, LODs, colliders, and spring
   bones.
5. It does not contain the complete structural target library or `MHCLO` maps.
6. Live edits build into an inactive fixed slot. Publication changes all parts
   at one frame boundary.
7. A recipe hash lets equal characters share one bake and use a persistent
   derived-data cache.

The existing engine already has most ownership primitives. It has fixed model
handles, atomic model revisions, a `GeometryArena`, generated RPC workers,
native OS-worker composition, and a bounded blob store.

The important missing parts are a generic complete model record, a character
baker, a multi-primitive atomic model publication, and fixed character pools.

### Prototype algorithm evidence

`prototype/character-editor/bake-core-rs` contains an isolated Rust prototype
for the difficult bake algorithms. It is not an engine crate, workspace member,
worker, public API, or accepted runtime design.

The prototype tests signed SurfaceWrap fitting, sparse-target evaluation,
macro products, corrected skin transfer, and normal rebuilding. It also has
no-allocation checks and MPFB parity for 26 sampled CC0 `short04` vertices.

All 14 Rust tests pass. The real-hair fit error limit is `3e-6` Blender units.
The character editor also has a TypeScript CPU-fit slice for all ten CC0 system
hairstyles. It keeps authored helper-cage bindings and composes body-bound MHCLO
records with PunkElvs. It transfers rig weights from the composed proxy
triangles. It does not mask the head or add a clearance. This remains prototype
code.

Source parsing, complete macro tables, tangents, rig-rest fitting, SpringChain,
and worker integration remain open.

## 1. Locked direction

The following direction is already accepted:

- Runtime character editing is necessary.
- Male and female bodies remain separate topology assets.
- Structural changes can cause a character rebake.
- Expressions, visemes, gaze, and animation do not change structural rest fit.
- Humentity and `bevy_make_human` are co-primary N1 permissive references.
- `MHCLO` geometric weights remain signed and are not clamped.
- Runtime work uses fixed capacities after `GameplaySealed`.
- Finished characters must be smaller than the editable source.
- The old complete character stays visible until a new bake is valid.

## 2. Current engine audit

### 2.1 Current character prototype

The prototype produces these source models:

| Source | Vertices | Triangles | Structural and face targets | File size |
|---|---:|---:|---:|---:|
| Male | 18,104 | 33,392 | 691 | 25 MiB |
| Female | 17,432 | 32,424 | 689 | 19 MiB |

The glTF files use sparse accessors for most targets. This keeps the files
small, but Three.js can expand sparse morph data into runtime arrays.

A dense position-only expansion has these lower bounds:

| Source | All targets | All 87 face targets on the complete body |
|---|---:|---:|
| Male | 143.2 MiB | 18.0 MiB |
| Female | 137.5 MiB | 17.4 MiB |

These values exclude base geometry, normals, skin data, indices, GPU padding,
and other LODs. Thus, the current glTF files are good source fixtures, but they
are not lightweight finished runtime models.

The 87 face targets affect a smaller vertex union:

| Source | Affected face vertices | Dense face-target positions after a face split |
|---|---:|---:|
| Male | 3,820 | 3.8 MiB |
| Female | 7,823 | 7.8 MiB |

A face-support split can remove approximately 10-14 MiB of face morph storage
for one LOD. This is a measured source fact, not yet a selected production
policy.

The prototype also has production gaps:

- Macro combinations use endpoint approximations.
- The skeleton does not refit for every structural control change.
- Hair and clothes do not use the built-in runtime path.
- The Blender/MPFB generator is an offline fixture tool.
- MPFB code cannot become an engine runtime dependency.

### 2.2 Model runtime

`ModelSystem` already supplies useful mechanisms:

- Fixed generational `ModelHandle` values.
- Bounded pending work and resident CPU bytes.
- Stale revision tokens.
- Old-revision retention after failure.
- Atomic publication into `GeometryArena` slots.
- Shared geometry between model bindings.
- Rigged and morphed runtime LOD processing.

`GeometryArena` already supplies:

- Fixed prewarmed `BufferGeometry` slots.
- Exact vertex, index, group, attribute, and morph capacities.
- Fixed CPU arrays behind the Three.js geometry.
- Complete-publication preflight and rollback.
- GPU-byte and upload telemetry.

The model path still has blockers:

- The current cooked BIG model is rigid and has one primitive.
- The complete rig/morph glTF extension remains open.
- `ModelSystem` records one geometry chain, not a complete primitive set.
- `GeometryArena.publish()` still allocates publication scratch.
- Runtime meshoptimizer work allocates and is too large for each character edit.
- Real-GPU skinned/morphed arena evidence remains open.

The character system must complete these generic model parts. It must not add a
second character-only geometry owner.

### 2.3 Worker and storage runtime

The worker system already supplies:

- Generated typed TypeScript clients.
- The shared RingBuffer protocol.
- Web Workers for the public web target.
- Real OS workers for the native target.
- Named native service manifests.
- Generational native arenas and `HandleQueue`.
- Fixed task and completion capacities.

`PersistentBlobStore` already supplies:

- Bounded item and byte admission.
- Atomic two-generation publication.
- Native and OPFS worker backends.
- 512 KiB bounded chunks.
- Stable failure states and telemetry.

A character derived-data cache must use this store. It must not add a new file
or OPFS mechanism.

## 3. Target ownership model

```text
shared CharacterSourcePack
        +
small CharacterRecipe
        |
        v
fixed CharacterBakeWorker workspace
        |
        v
unpublished CharacterBakeRecord
        |
        v
ModelSystem batch preflight + CharacterSystem preflight
        |
        v
one atomic CharacterPublication
        |
        v
many lightweight CharacterInstance records
```

### 3.1 Shared source ownership

One loaded source pack can serve many characters. It contains immutable source
geometry, targets, fitting maps, rig rules, and LOD templates.

The source pack is not copied into each character. The worker source table owns
fixed source slots with generational handles.

### 3.2 Recipe ownership

A recipe contains only choices:

- Source-pack content ID.
- Male or female body source ID.
- Structural control values.
- Persistent asymmetry values.
- Equipment, hair, and body-part IDs.
- Material and texture parameter IDs.
- Runtime facial-channel profile.
- Geometry and spring LOD profile.

Animation pose, expression weights, viseme weights, gaze, spring positions, and
world transforms are not recipe data.

### 3.3 Finished bake ownership

A finished bake contains immutable data:

- Base positions after structural fitting.
- Normals, tangents, UVs, colors, indices, groups, and bounds.
- Skin indices and weights.
- Selected face and speech morph targets only.
- Complete cooked LOD records.
- Fitted skeleton rest transforms and inverse binds.
- Fitted collider records.
- Hair spring-chain rest records.
- Material and texture bindings by stable asset ID.
- Source, recipe, profile, and baker hashes.

It does not contain:

- The complete structural target library.
- `MHCLO` text or packed SurfaceWrap maps.
- Editor zones or control descriptions.
- Undo history.
- Temporary fit positions.
- MPFB or Blender data structures.

### 3.4 Instance ownership

Equal recipe hashes share one immutable bake. Each visible instance owns only:

- A character publication handle.
- One fixed skeleton-pose slot.
- One facial-weight slot for the selected profile.
- One animation-state slot.
- One spring-state slot when necessary.
- Render binding and world-transform handles.

The editable recipe can remain game-owned. A character that is not editable
does not need a recipe slot.

## 4. Source and output formats

### 4.1 Authored input

Keep normal authoring files:

- Body source glTF.
- Hair, clothes, and body-part glTF files.
- Small TOML files for fitting, spring, collider, fit-range, and LOD semantics.
- MakeHuman target, rig, and proxy files as licensed source inputs.

TOML is an authoring format only. Runtime workers do not parse TOML.

### 4.2 `CharacterSourcePack`

Add an offline `afterglow-pipeline character-source` command. It validates and
converts authored input into one versioned binary source record.

The source record contains:

1. A section directory with checked lengths and checksums.
2. The neutral `hm08` driver positions.
3. Sparse structural targets and exact macro composition data.
4. Male and female body-proxy geometry and SurfaceWrap records.
5. Eye, tooth, tongue, hair, clothing, and accessory records.
6. Precomputed top-four rig weights.
7. Rig landmark and rest-fit records.
8. Precomputed normal and tangent adjacency.
9. Face-target support sets.
10. Per-part LOD remaps and index templates.
11. Body hide groups and fit limits.
12. Collider and SpringChain templates.
13. Content IDs and a license ledger.

The cook must preserve signed `MHCLO` weights. It must not repeat Humentity's
rig-weight padding error.

### 4.3 `CharacterBakeRecord`

Use a small character envelope around the generic complete model record.

```text
CharacterBakeRecord
  header and section directory
  source/recipe/baker/profile hashes
  CompleteModelRecord
  SkeletonRestRecord
  ColliderRestRecord
  SpringRestRecord
  MaterialBindingRecord
```

`CompleteModelRecord` must be the same binary model payload used by the planned
`EXT_afterglow_mesh_lods` path. Character baking must not create a second mesh
format.

The output record contains no JSON. It is deterministic and safe for direct
storage in `PersistentBlobStore`.

### 4.4 Cache key

Use this logical key input:

```text
source content hash
+ canonical recipe bytes
+ baker format version
+ facial-channel profile
+ LOD profile
+ target capability profile
```

The final ASCII store key can use a fixed digest. A stale baker or source hash
must cause a clean cache miss.

## 5. Bake algorithm

The offline pipeline and runtime worker must call one shared Rust core. The
core accepts caller-owned input, output, and scratch slices.

### Stage 1: validate and resolve

- Validate source and recipe handles.
- Validate all selected asset combinations and fit ranges.
- Resolve exact macro weights.
- Reject non-finite or unsupported values.
- Reserve all output sections before geometry work starts.

### Stage 2: evaluate the body driver

- Copy the neutral driver into the inactive workspace.
- Apply sparse direct structural deltas.
- Apply exact macro-stack weights.
- Apply persistent asymmetry.
- Keep expressions and visemes at zero.

The evaluator can maintain a fixed dirty-target list for live edits. A full
recipe evaluation remains bounded by the source target capacity.

### Stage 3: fit the skeleton

- Evaluate rig landmarks from the fitted body driver.
- Calculate bone heads, tails, rolls, and parent-local rest transforms.
- Recalculate inverse bind matrices.
- Keep one stable bone ID table across all bakes.
- Reject a collapsed or non-finite bone before publication.

### Stage 4: fit body parts

For each selected body proxy, eye, tooth, tongue, garment, hair, or accessory:

- Evaluate exact and three-parent SurfaceWrap mappings.
- Apply all three offset scale values.
- Preserve source map weights without clamping.
- Copy precomputed skin influences.
- Apply authored correction targets only when specified.
- Validate fit limits and bounds.

### Stage 5: build retained facial channels

For each selected facial channel:

1. Apply the channel to the structurally fitted driver.
2. Refit only its precomputed support parts.
3. Subtract the fitted structural base.
4. Write the resulting runtime morph delta.

This gives shape-correct expressions and speech without retaining structural
morphs in the finished model.

### Stage 6: build shading data

- Recalculate positions, normals, and tangents with cooked adjacency.
- Preserve smooth normals across any face/body partition.
- Validate finite values and unit normal tolerances.
- Use authored card normals for hair only after a visual gate accepts them.

### Stage 7: compose topology and LODs

- Apply body hide groups from selected equipment.
- Apply cooked per-part LOD remaps and index templates.
- Do not run meshoptimizer during a live character bake.
- Preserve material groups and skin seams.
- Create the selected face-support partition when its profile requests it.
- Keep spring-rigged hair as a separate primitive when necessary.

### Stage 8: finalize runtime records

- Calculate bounds and exact byte counts.
- Write model, skeleton, collider, spring, and material sections.
- Calculate section and whole-record checksums.
- Return one unpublished bake handle.

### Stage 9: publish atomically

- Preflight every model arena slot and character-state slot.
- Copy every primitive and LOD into inactive slots.
- Create fitted skeleton and spring rest state in inactive slots.
- Swap one character publication generation at a frame boundary.
- Release old slots only after the new publication is complete.

Any failure keeps the old publication visible.

## 6. Fast live rebaking

Live editing uses two job classes.

### 6.1 Preview bake

A preview bake updates only data needed for the current close view:

- High-detail base positions.
- Normals and tangents.
- Skeleton rest data.
- Current fitted parts.
- Current colliders.

It can reuse the last face targets while the editor displays a neutral face.
Secondary hair motion stops during the drag.

The coordinator keeps only the newest requested recipe revision. The worker
checks stale revision tokens between fixed stages.

A new preview can start at most once per frame. Intermediate slider events are
coalesced into the newest fixed recipe slot.

### 6.2 Final bake

A final bake adds:

- Shape-correct facial channels.
- All requested LODs.
- Final body hide topology.
- Spring rest data.
- Cache encoding and checksum data.

It starts after control release, explicit commit, or an accepted idle interval.
The final result replaces the preview in one transaction.

### 6.3 Incremental work

For a control-only change:

- Subtract the prior sparse target contribution.
- Add the new sparse target contribution.
- Refit selected parts from the updated driver.
- Refit only bones and colliders related to dirty landmarks when possible.

For an equipment or hairstyle change:

- Keep unchanged part outputs.
- Fit only the new part and changed body hide groups.
- Preflight the replacement before the old part is released.

For a sex change:

- Start a complete new body-source transaction.
- Keep the prior character visible until the replacement is complete.

## 7. Runtime components

### 7.1 Shared Rust core

Add `afterglow-character` with:

- Versioned source and bake records.
- Public-document and permissive-reference parsers.
- SurfaceWrap evaluation.
- Macro-stack evaluation.
- Rig fitting and weight transfer.
- Normal and tangent rebuild.
- Caller-owned `CharacterBakeWorkspace` operations.
- Deterministic hashing and validation.

The crate must record Humentity and `bevy_make_human` notices. It must not
depend on Bevy, Blender, MPFB, or Three.js.

### 7.2 Worker crate

Add `afterglow-character-worker` with one generated async RPC service.

The worker owns fixed tables for:

- Source packs.
- Pending recipes.
- Bake workspaces.
- Output records.
- Completion records.
- Input and output chunk transfers.

The native shell starts it as a real OS worker. Public web starts the matching
Wasm service in a Web Worker.

Large source and output bytes use bounded chunks. Native source-backed loading
can keep source bytes out of V8. The first implementation must keep identical
record semantics on both targets.

### 7.3 TypeScript coordinator

Add `engine/character/character-system.ts`.

Proposed public operations:

```ts
registerSource(source): CharacterSourceHandle | 0
createCharacter(recipe, profile): CharacterHandle | 0
requestPreview(handle, recipe, revision): CharacterBakeStatus
requestCommit(handle, recipe, revision): CharacterBakeStatus
poll(budget): void
getView(handle): Readonly<CharacterView> | null
createInstance(handle, options): CharacterInstanceHandle | 0
destroyInstance(handle): boolean
destroyCharacter(handle): boolean
```

Hot calls return typed states. They do not create promises, strings, arrays, or
result objects.

### 7.4 Generic model changes

Complete the existing UPR model work before character publication:

- Replace the rigid one-primitive cooked record.
- Add a complete multi-primitive `CompleteModelRecord`.
- Add cooked revision replacement without runtime meshoptimizer work.
- Add atomic batch publication across all primitives and LODs.
- Make `GeometryArena` publication scratch persistent.
- Support fixed morph-count bucket profiles.
- Keep one skeleton and animation graph across primitive LODs.

The character system consumes these APIs. It does not access Three.js private
renderer fields.

### 7.5 Persistent cache adapter

Add a thin `CharacterBakeCache` consumer over `PersistentBlobStore`.

- A configured store enables persistent derived bakes.
- No store gives a memory-only session cache.
- Cache load validates the complete record before publication.
- Cache save uses atomic replacement.
- Cache failure never blocks a successful in-memory bake.

## 8. Lightweight finished-character profiles

The engine should support fixed profile descriptors instead of one hard-coded
character size.

Recommended initial profiles:

| Profile | Structural targets | Facial channels | Spring hair | Use |
|---|---:|---:|---:|---|
| `Hero` | 0 | ARKit 52 plus one speech set | Full near LOD | Player and close NPC |
| `Social` | 0 | One game-selected expression/speech subset | Reduced | Nearby NPC |
| `Crowd` | 0 | 0 or a small fixed set | None | Distant NPC |

The complete source always remains editable. A profile changes only the
finished bake.

A later rebake can promote or demote a character profile. The old profile stays
visible until the replacement is ready.

## 9. Hair and secondary motion

Character baking uses the generic SurfaceWrap record from the hair research.

The first implementation supports:

- Main-rig short hair.
- Main-rig long hair without secondary motion.
- Authored SpringChain hair.
- Fixed sphere and capsule colliders.

During a structural preview:

- Disable secondary motion for the edited character.
- Refit hair rest vertices, bones, and colliders.
- Reset spring history after publication.
- Fade spring motion in after the final commit.

Do not use strand simulation or full body-mesh collision.

## 10. Materials and texture edits

Geometry baking and texture composition remain separate mechanisms.

A character recipe can refer to:

- Stable material descriptors.
- Virtual-texture handles.
- Mutable texture snapshot keys.
- Color and scalar material parameters.

Tattoos, makeup, dirt, and user paint should use the existing mutable texture
and persistence systems. They must not force a geometry bake unless their
material layout changes.

## 11. Capacity and failure policy

Add explicit capacities to `EngineMemoryConfig` and the character worker:

- Maximum loaded character source packs.
- Maximum baked character publications.
- Maximum character instances.
- Maximum concurrent preview and final jobs.
- Maximum selected parts per recipe.
- Maximum structural controls.
- Maximum facial channels per profile.
- Maximum bones, spring joints, and colliders.
- Maximum source, workspace, output, and cache bytes.
- Maximum upload bytes and publications per frame.

Deterministic failure states include:

- `CapacityExceeded`.
- `SourceUnavailable`.
- `RecipeInvalid`.
- `CombinationUnsupported`.
- `FitOutOfRange`.
- `StaleRevision`.
- `OutputTooLarge`.
- `ModelArenaRejected`.
- `CacheMiss`.
- `CacheCorrupt`.
- `WorkerFault`.

A failed preview or final bake does not damage the current character.

## 12. Telemetry

Append fixed telemetry records for:

- Preview and final jobs queued, completed, stale, failed, and rejected.
- Stage time for target evaluation, wrap, rig, shading, LOD, encoding, upload,
  and publication.
- Source, workspace, output, CPU geometry, and GPU geometry bytes.
- Dirty targets and fitted vertices.
- Upload bytes and publication latency.
- Cache hit, miss, corrupt, read bytes, and write bytes.
- Active character, instance, face-channel, spring, and collider counts.
- Pool high-water and overflow values.

Do not create dynamic labels from asset or control names.

## 13. Work sequence

### RCB-000 — Freeze fixtures and measurements

- Preserve the current male and female source GLBs as input fixtures.
- Record sparse accessor counts and dense runtime lower bounds.
- Export exact MPFB golden positions for direct, macro, asymmetry, and face
  combinations.
- Record current load time, heap use, GPU bytes, and slider latency.

**Done when:** every optimization has a numeric and visual baseline.

### RCB-010 — Complete the generic model record

- Finish UPR-040 and UPR-050.
- Add complete multi-primitive rig/morph cooked records.
- Add cooked model revision replacement.
- Add persistent arena publication scratch.
- Prove skinned and morphed arena rendering on WebGPU.

**Done when:** one cooked multi-part rigged fixture can atomically replace all
of its geometry without runtime mesh optimization.

### RCB-020 — Add the shared character core

- Add `afterglow-character`.
- Import no Bevy code or data structures.
- Adapt the two N1 implementations under their notices.
- Implement caller-owned fitting functions.
- Correct rig influence selection and normalization.
- Add malformed-input and no-allocation tests.

**Done when:** the core reproduces neutral and single-target fixtures.

### RCB-030 — Add the source cooker

- Add the pipeline command.
- Convert body, proxy, rig, face, hair, clothes, TOML, and LOD data.
- Build support sets, adjacency, remaps, hide groups, and hashes.
- Write deterministic source packs.

**Done when:** a clean checkout can rebuild both body source packs and all ten
hair fixtures without MPFB runtime code.

### RCB-040 — Complete golden parity

- Test all ten CC0 hair assets.
- Test all body controls and macro endpoints.
- Test representative multi-control combinations.
- Test both body proxies, genitals, eyes, teeth, and tongue.
- Test all 87 face channels on multiple structural faces.
- Test rig rest transforms and skin deformation.

**Done when:** all accepted position, normal, tangent, and bone tolerances pass.

### RCB-050 — Add the fixed worker

- Add generated RPC methods and chunk transfer.
- Add fixed source, job, workspace, and output slots.
- Add latest-revision cancellation checkpoints.
- Compose one native OS worker and one public-web Worker path.
- Add tracked allocator tests after worker seal.

**Done when:** repeated bake calls plateau in memory and reject overflow.

### RCB-060 — Add `CharacterSystem`

- Add fixed handles, records, views, and typed states.
- Integrate worker polling and frame budgets.
- Integrate atomic complete-model publication.
- Add instance sharing by bake hash.
- Integrate skeleton, facial, and material bindings.

**Done when:** two instances share geometry and animate independently.

### RCB-070 — Add live preview and final commit

- Add fixed recipe slots and coalesced revisions.
- Add incremental control updates.
- Add preview publication.
- Add final face/LOD publication.
- Keep neutral facial state and disabled springs during drag.

**Done when:** rapid controls never publish stale or partial output.

### RCB-080 — Add persistent derived bakes

- Add the thin blob-store cache adapter.
- Validate cache keys and complete records.
- Test interrupted writes, corrupt generations, and source version changes.
- Keep memory publication independent from cache-save success.

**Done when:** a cache hit skips fitting and publishes the same model bytes.

### RCB-090 — Add hair spring integration

- Convert `ponytail01` from its source sidecars.
- Add fixed SpringChain and collider pools.
- Add shape-change, teleport, and LOD resets.
- Add visual and fixed-step parity tests.

**Done when:** the ponytail passes shape, motion, collision, and reset gates.

### RCB-100 — Migrate the editor

- Move the visual editor to public `CharacterSystem` APIs.
- Remove production use of direct GLTF structural morph weights.
- Keep Blender/MPFB only for source generation and golden fixtures.
- Keep game code free of RPC, source-pack parsing, and worker ownership.

**Done when:** the editor can edit, commit, reload, and swap hair through public
engine APIs only.

### RCB-110 — Release evidence

Run both public-web Chromium and `afterglow-shell` tests:

- Cold source load.
- Cache hit and cache miss.
- Continuous slider drag.
- Equipment and hair replacement.
- Male/female body replacement.
- Concurrent edited and animated characters.
- 30-minute mixed edit soak.
- 60-minute finished-character crowd soak.
- Device loss and worker fault.

Require stable heap floors, bounded queues, no stale publication, no post-seal
pipeline creation, and no unexpected WebGL fallback.

## 14. Technical acceptance gates

These are measured engineering gates, not product decisions.

### RCB-TG-001 — Preview backend

Compare one Rust CPU worker, a worker pool, and a GPU compute fit only if the
single worker misses the accepted latency. Select the simplest passing path.

### RCB-TG-002 — Face partition

Compare:

- One complete body primitive.
- One static body plus one expression-support face primitive.
- A custom sparse facial deformation path.

Measure GPU bytes, seams, shader variants, upload time, and close-face quality.
Use public Three.js APIs only.

### RCB-TG-003 — Native output transfer

Start with bounded ring chunks and the generic model publication path. Measure
native direct GPU upload only after the common path works.

Do not create a native-only character representation.

### RCB-TG-004 — LOD template quality

Compare cooked topology templates against per-character simplification on
extreme shapes. Add shape-specific templates only after a measured failure.

### RCB-TG-005 — Incremental fit value

Compare full fixed-array refit with dirty-target and dirty-landmark updates. Keep
incremental complexity only when it gives a material latency reduction.

## 15. Tests

Required unit and regression coverage:

- Source and bake record round trips.
- Section overflow, overlap, truncation, and checksum failure.
- Direct and three-parent SurfaceWrap mapping.
- Signed weights below zero and above one.
- X/Y/Z scales and coordinate conversion.
- Exact macro composition.
- Top-four rig influence selection.
- Skeleton rest and inverse bind generation.
- Normal and tangent reconstruction.
- Face support and seam continuity.
- LOD remap and material-group retention.
- Body hide groups.
- Recipe canonicalization and hashing.
- Cache generation and stale-version rejection.
- Job cancellation and stale completion.
- Complete publication rollback at every stage.
- Source, job, workspace, output, model, skeleton, and spring pool overflow.
- Native and web output equivalence.
- Sealed worker no-allocation stages.
- Long repeated edits with stable memory.

## 16. Rejected approaches

Do not use these paths:

- Keep all 689/691 structural morph targets on every finished character.
- Parse `MHCLO` or TOML in a gameplay frame.
- Run Blender or MPFB in the shipped engine.
- Run meshoptimizer for every slider movement.
- Allocate one worker per character.
- Publish body, hair, skeleton, or colliders separately.
- Keep old spring velocity after a shape publication.
- Copy the unlicensed Retro Engine fitter.
- Translate GPL or AGPL implementation structure.
- Add a character-specific GPU allocator or persistent store.
- Use a native Wasm worker instead of a real OS worker.

## 17. User decisions before implementation

### RCB-DEC-001 — Finished facial profiles

**Recommended:** `Hero` keeps ARKit 52 plus Meta 14. Other profiles keep a
smaller game-selected set or no face targets.

Alternative: keep all 87 face targets on every close character. This increases
GPU memory and arena bucket sizes.

### RCB-DEC-002 — Persistent derived cache

**Recommended:** enable automatic character-bake cache reads and writes when the
game supplies a `PersistentBlobStore`. Use memory-only behavior otherwise.

Alternative: make every save and load explicit in game code.

### RCB-DEC-003 — Recipe precision

**Recommended:** store canonical signed 16-bit control values. This gives stable
hashes and more precision than the UI needs.

Alternative: preserve raw float32 bits. Equivalent visible recipes can then use
different cache keys.

### RCB-DEC-004 — Live preview policy

**Recommended:** accept at most one newest preview per frame. Start the final
bake on control release or after 100 ms without a new edit.

Alternatives: preview at a lower fixed rate, or update only after release.

### RCB-DEC-005 — Runtime sharing

**Recommended:** equal recipe and profile hashes share immutable baked geometry.
Each instance keeps independent pose, face, and spring state.

Alternative: give every character private geometry. This is simpler but uses
much more memory.

### RCB-DEC-006 — Product capacities

The user must specify or accept defaults for:

- Maximum nearby character instances.
- Maximum simultaneous Hero-profile characters.
- Maximum concurrent edited characters.
- Maximum loaded source packs.
- Maximum selected equipment parts.

No fixed pool values can be approved before this decision.

## 18. Recommended first measured slice

Build the smallest complete vertical slice:

1. One female source body.
2. Ten structural controls, one macro combination, and one asymmetry control.
3. One 52-target ARKit face profile.
4. One rigid short hairstyle.
5. One four-level cooked LOD set.
6. One fixed worker workspace.
7. One inactive model publication.
8. One recipe-hash memory cache.
9. Native and public-web parity.

This slice must measure preview latency, final latency, output bytes, GPU bytes,
main-thread upload time, and memory plateau. Expand to all controls and parts
only after it passes.
