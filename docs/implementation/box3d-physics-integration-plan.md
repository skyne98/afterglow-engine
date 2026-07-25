# Box3D physics integration plan

**Status:** queued behind Steam Audio integration; design recorded, implementation not started  
**Date:** 2026-07-18  
**Direction:** Box3D is the selected physics candidate, subject to the alpha and cross-target gates below

Related documents:

- [`spatial-audio-integration-plan.md`](spatial-audio-integration-plan.md) — shared structural geometry and worker constraints
- [`editor-plan.md`](editor-plan.md) — Blender/editor authoring and cooked-shape visualization
- [`no-runtime-allocation-constant-time-budget-plan.md`](no-runtime-allocation-constant-time-budget-plan.md) — sealed-runtime rules
- [`../api/ring-buffer.md`](../api/ring-buffer.md) — only page/worker payload mechanism

This plan records the physics direction now so the project can return to Steam
Audio without losing the decisions. It does not authorize starting physics work
before the audio integration gates complete.

---

## 1. Decision

Use glTF/GLB as the durable geometry authoring format and Box3D as the candidate
runtime physics engine. Do not invent a custom model format and do not duplicate
Box3D's mesh, hull, BVH, or compound cookers.

```text
Blender model.glb / level.structural.glb
  explicit render, physics, audio usage metadata
                    |
                    v
afterglow-pipeline structural cook
  |                 |                    |
  v                 v                    v
render chunks   Box3D derived chunks   acoustic blob
                     |
                     v
fixed-memory physics Web Worker
                     |
             RingBuffer pose/events
                     |
                     v
page ECS and renderer
```

Public web builds and the page hosted by CEF use the same Box3D Web Worker. The
CEF shell remains thin and gains no native physics IPC. A native Box3D service
may be built for tests or a future non-browser host, but it is not inserted
between CEF and the website.

Box3D was announced in June 2026 and its author still describes it as alpha.
Pin one audited commit, record it in the asset/toolchain fingerprint, and do not
call the integration production-ready until native and wasm acceptance, soak,
and determinism gates pass.

Resolved shared-shape policy:

- runtime instantiates pipeline-cooked primitive/convex/compound templates; it
  does not upload arbitrary new point clouds or triangle meshes in version one;
- moving compounds use fixed prebuilt child slots; runtime may enable/disable a
  child and change child/whole transforms and primitive dimensions;
- movement is a normal per-tick update; resize applies only at tick boundaries
  as a bounded structural update that refreshes mass, contacts and bounds;
- static triangle meshes, height fields and baked compounds remain static;
  movable complex bodies use convex/primitive compounds;
- there is no loading-screen phase; initial warm-up is followed by continuous
  bounded world/asset streaming, with as much work as possible cooked offline.

---

## 2. KISS boundaries

1. **One physics engine.** Do not retain Rapier or a second gameplay physics
   backend after Box3D promotion. Existing demo-only Rapier dependencies are
   removed when their replacement is verified.
2. **One authored geometry format.** Modular collider nodes may live beside
   render nodes in `model.glb`; large worlds may use a companion
   `level.structural.glb`. Both use the same metadata schema.
3. **One payload mechanism.** Bootstrap capability descriptors aside, all
   authored page↔physics-worker payload uses fixed SPSC RingBuffers and
   payload-free wake-ups.
4. **Mechanism versus policy.** Box3D owns collision cooking and simulation.
   Afterglow owns source selection, capacities, handles, scheduling, transport,
   ECS projection, and telemetry.
5. **Derived data is disposable.** Box3D mesh/compound bytes are recookable from
   glTF and never become the source model format.
6. **No sealed allocation.** Worker memory, worlds, pools, templates, bodies,
   shapes, joints, contacts, commands, events, and output snapshots are bounded
   before `GameplaySealed`.
7. **No runtime render-mesh collision fallback.** Missing or invalid physics
   metadata is a cook/load error, not permission to collide against the render
   mesh.
8. **Static and dynamic geometry stay distinct.** Static concave meshes,
   height fields and baked compounds never attach to dynamic/kinematic bodies.
   Moving compound templates contain fixed primitive/convex child slots.
9. **No arbitrary sealed geometry cook.** Runtime changes pose, analytic
   dimensions, prebuilt hull scale and fixed-child activation; new point/triangle
   topology is pipeline work.
10. **No loading screen.** Static tiles and instances admit continuously through
   fixed arenas, queues and atomic publication after warm-up.
11. **The editor visualizes truth.** It can compare authored proxies with cooked
   Box3D debug geometry and reports version/capacity errors before play.

---

## 3. Authored structural schema

A small Afterglow Blender panel writes validated glTF `extras`. Metadata, not
object naming, is canonical. Names and collections remain human-readable aids.

Minimum node fields:

```json
{
  "afterglow_schema": 1,
  "afterglow_usage": "physics,audio",
  "afterglow_id": "west_gate",
  "afterglow_body": "static",
  "afterglow_collider": "box",
  "afterglow_physics_material": "stone",
  "afterglow_collision_layer": "world",
  "afterglow_collision_mask": "dynamic,character",
  "afterglow_audio_material": "concrete"
}
```

Supported physics roles:

- `static`;
- `kinematic`;
- `dynamic`;
- `trigger`.

Authored collider kinds and Box3D mapping:

| Authored kind | Box3D cook/runtime representation |
|---|---|
| `box` | `b3MakeBoxHull`; authored size becomes half-extents |
| `sphere` | native `b3Sphere` |
| `capsule` | native `b3Capsule` |
| `convex` | bounded `b3CreateHull` from authored points |
| `trimesh` | static `b3CreateMesh` |
| `heightfield` | `b3CreateHeightField` plus referenced height samples |
| `compound` | static baked compound, or fixed prebuilt child slots attached as normal shapes for moving bodies |

The schema aligns shape meaning and meter units with the proposed
`KHR_implicit_shapes`/`KHR_physics_rigid_bodies` work where practical, but does
not depend on an unratified extension. Add a cooker adapter later if a stable
Khronos extension is published.

Validation rejects:

- absent usage/body/collider metadata;
- duplicate stable IDs;
- non-finite transforms or material values;
- zero dimensions;
- nonuniform sphere/capsule radial scale;
- dynamic or kinematic triangle meshes/height fields/baked compounds;
- runtime-mutable point/triangle topology or unbounded compound child counts;
- convex input that is coplanar, degenerate, or over the configured point/face
  limits;
- unsupported skinning, morphing, or deforming collision geometry;
- unknown material/layer/filter IDs;
- count or byte capacity excess.

---

## 4. Offline Box3D cooking

Add a structural cook mode to `afterglow-pipeline`. It selects `physics` nodes,
applies static transforms, preserves moving local-space shapes and stable IDs,
resolves material/filter tables, and invokes Box3D rather than implementing a
parallel collision cooker.

### Convex and primitive shapes

- Boxes map to Box3D's efficient inline box hull.
- Convex point sets call `b3CreateHull(points, count, maxVertexCount)`.
- The configured vertex ceiling is explicit and cooker diagnostics record the
  resulting vertices/faces, volume, and byte count.
- Spheres/capsules remain analytic and are not triangulated.

### Static triangle meshes

Build `b3MeshDef` directly from glTF positions/indices/material IDs:

- enable vertex welding with an explicit meter tolerance;
- enable edge identification for shared-edge internal-collision suppression;
- choose SAH by default and median split only for measured grid/voxel-like
  inputs;
- reject/report degenerates, invalid winding, invalid indices, and material
  excess;
- record vertices, triangles, BVH nodes/height, bytes, degenerates, cook time,
  and source hash.

Do not automatically simplify production geometry in Box3D integration. Artists
provide reviewed collision proxies. An optional offline simplification/convex-
decomposition experiment must emit error/debug reports and cannot silently
replace authored shapes.

### Static baked compounds

Box3D's intended large-static-world path is:

1. assemble child spheres, capsules, hulls, and meshes in `b3CompoundDef`;
2. call `b3CreateCompound` to build one immutable internal AABB tree;
3. call `b3ConvertCompoundToBytes`;
4. store the flat buffer as a generic `.big` blob;
5. load it into writable stable worker memory;
6. call `b3ConvertBytesToCompound` for in-place pointer fixup;
7. attach it to one static body with `b3CreateBakedCompoundShape`.

A baked compound enters the world broad phase as one shape while querying only
relevant children. Use it for world tiles, building shells, and large kitbashed
static assemblies. Do not use it for moving bodies.

### Moving templates and resize

The pipeline also emits moving body templates containing a fixed list of normal
Box3D primitive/convex shapes and stable child IDs. Worker bootstrap reserves
body/shape slots for declared templates. Runtime may move/rotate the body every
tick and, at tick boundaries, enable/disable fixed children, change child local
poses, resize boxes/spheres/capsules, or apply permitted scale to prebuilt hulls.
A resize is structural: update the shape, recompute body mass as required,
refresh contacts/bounds and report its bounded cost. It is not treated as cheap
pose motion.

No version-one command supplies new convex points, triangles or compound child
topology. Procedural content composes and parameterizes the prebuilt set. Static
triangle meshes/height fields/baked compounds remain immovable.

### Target-specific derived chunks

The current compound serialization is direct C memory layout, not a documented
portable interchange format. It contains compiler/ABI-sensitive structures and
pointer fixups. `B3_COMPOUND_VERSION` is necessary but not sufficient proof that
native 64-bit output can load under wasm32.

Wrap every derived chunk with:

- Afterglow physics chunk version;
- exact Box3D commit and `B3_COMPOUND_VERSION`;
- target triple/ABI and pointer width;
- endianness;
- `sizeof`/alignment fingerprint for every serialized public structure;
- source glTF SHA-256 and cook settings hash;
- payload byte count and checksum.

First test whether native and wasm layouts are byte compatible. Unless proven,
emit separate `wasm32` and native chunks using the exact target toolchains.
Loading a mismatch fails before pointer fixup. A Box3D upgrade invalidates and
recooks all affected derived chunks.

Current upstream compound packing/alignment issues must be resolved or locally
regression-tested before this format is accepted.

---

## 5. Physics worker

Create workspace crates only after the alpha gate:

```text
crates/afterglow-box3d-sys/       pinned minimal C FFI/build
crates/afterglow-physics/         public handles, commands, events, asset schema
crates/afterglow-physics-worker/  Box3D ownership, stepping, queries, telemetry
```

Browser orchestration remains authored TypeScript under:

```text
crates/afterglow-web/web/src/engine/physics/
```

The Web Worker owns:

- fixed Box3D WASM memory;
- one Box3D world;
- fixed static-compound tile slots that stream continuously after warm-up;
- fixed body/shape/joint/template/compound-child handle tables;
- fixed command, query, event, and pose rings;
- fixed simulation scratch and telemetry;
- optional persistent worker threads only if the benchmark selects them.

The page owns ECS bindings and generational public handles, not Box3D pointers or
IDs. Worker tables map public handles to validated current Box3D IDs.

---

## 6. Fixed timestep and ring protocol

Use an explicit fixed simulation step, initially 60 Hz, independent of render
refresh. The page publishes bounded commands and an intended tick horizon; the
worker advances at most a configured number of steps per wake. It never performs
an unbounded catch-up loop.

Initial rings:

| Producer | Consumer | Purpose |
|---|---|---|
| Page | Physics Worker | lifecycle and structural commands |
| Page | Physics Worker | kinematic targets, forces and query requests |
| Physics Worker | Page | latest pose batches |
| Physics Worker | Page | contacts, triggers, query results and failures |
| Physics Worker | Page | bounded telemetry |

Hot records are fixed binary layouts with protocol version, byte length,
sequence, world generation, tick, and generational handles. Lifecycle RPC may
use generated postcard clients only outside sealed hot paths.

Policy:

- commands are applied only at tick boundaries;
- pose output is latest-state and old unread snapshots may be skipped without
  running catch-up work;
- contact/trigger event overflow is visible and deterministic;
- query IDs are fixed-pool generational handles;
- stale world/body/query generations are ignored and counted;
- no promises or per-body RPC calls occur in frame hot paths.

Frame integration follows the engine's worker poll and pose-batch stages. The
page applies one bounded pose batch before hierarchy/render/audio publication,
so rendering and audio observe the same Box3D tick.

---

## 7. Sealed allocation strategy

Box3D exposes expected initial world capacities, but they are not documented as
hard no-growth guarantees. Prove behavior with a tracked allocator.

During warm-up:

1. create the world with declared expected static/dynamic body and contact
   counts;
2. reserve fixed static-compound tile slots and admit the initial neighborhood;
3. create every moving collider/compound template and fixed child slot;
4. exercise configured body/shape/joint/contact/resize high-water scenarios;
5. reserve fixed Afterglow handle/ring/snapshot storage;
6. create any selected persistent Box3D workers;
7. reset to the starting world and seal the allocator.

Gameplay spawning uses fixed template pools and Box3D's internal free lists. If
create/destroy still allocates after warm-up, precreate disabled bodies/shapes
per declared template/capacity and activate/deactivate them. Shape resize and
fixed-child enable/disable must also pass the sealed allocator test. Static tiles
stream into reserved writable buffers and publish atomically; there is no phase
that restores general allocation. Do not add an untracked allocator exception.

Every queue and pool reports capacity, use, high-water, overflow, and current
world generation. A full body/joint/event/query pool returns a typed failure and
does not resize.

---

## 8. Threading policy

Start evaluation at one Box3D simulation thread inside its service Worker. The
page, audio simulation Worker and its two pthreads, asset/codec workers, and
renderer already compete for CPU. Do not copy Box3D's maximum thread count.

Benchmark one versus two persistent Box3D workers under simultaneous Dungeon and
Steam Audio load. Select the smallest configuration meeting the tick budget.
All threads are created before seal. After seal, task dispatch must use shared
memory synchronization without authored postMessage payloads.

WASM builds retain Box3D SIMD and fixed memory. Native and web use identical
scene/tick/command fixtures for differential determinism testing.

---

## 9. Public engine surface

The intended public mechanism is capacity-explicit and handle-based:

```ts
interface PhysicsConfig {
  maxBodies: number;
  maxShapes: number;
  maxJoints: number;
  maxContacts: number;
  maxEvents: number;
  maxQueries: number;
  maxPoseBatch: number;
  fixedHz: number;
  maxCatchUpSteps: number;
}

interface EnginePhysicsSystem extends RenderWorkerInput {
  warm(): Promise<void>;
  seal(): void;
  trySpawn(template: PhysicsTemplateHandle, entity: number): PhysicsBodyHandle;
  tryDespawn(body: PhysicsBodyHandle): PhysicsStatus;
  trySetKinematicTarget(body: PhysicsBodyHandle, pose: RigidPose): PhysicsStatus;
  trySetShapeDimensions(shape: PhysicsShapeHandle, value: ShapeDimensions): PhysicsStatus;
  trySetCompoundChild(body: PhysicsBodyHandle, child: number, state: CompoundChildState): PhysicsStatus;
  tryApplyForce(body: PhysicsBodyHandle, force: Vec3): PhysicsStatus;
  tryQuery(request: PhysicsQuery): PhysicsQueryHandle;
  readStats(out: PhysicsStats): void;
  dispose(): void;
}
```

Exact names wait for implementation. The invariant is one worker-owned Box3D
world, fixed page handles, fixed command/result records, and no exposed FFI
pointer or Box3D ID.

---

## 10. Editor contract

The editor does not implement collision algorithms. It:

- renders authored glTF collider nodes by kind and usage;
- offers Box3D-specific shape/material/filter fields through the Blender add-on
  and scene inspector;
- invokes the pipeline cook against an immutable source revision;
- displays cooked Box3D debug geometry, BVH/compound bounds and diagnostics;
- compares authored and cooked bounds/IDs/materials;
- blocks play/save revision on hard validation failures;
- never edits generated Box3D bytes.

Static baked chunks, dynamic templates, and acoustic outputs use the same stable
authored IDs so the world manifest resolves them consistently.

---

## 11. Acceptance gates

### B3-0 — alpha viability

Before engine API work:

- pin an exact Box3D commit and license;
- compile native and wasm32 with deterministic fixtures;
- validate boxes, spheres, capsules, hulls, static meshes, height fields,
  triggers, joints, CCD, character movement, queries and baked compounds;
- resolve/regress current compound packing/alignment issues;
- test malformed and misaligned compound buffers under sanitizers;
- measure memory, tick p50/p95/p99 and output determinism;
- decide one versus two simulation workers under audio/render contention.

### B3-1 — cross-target derived format

- prove or reject native↔wasm compound byte compatibility;
- reject wrong version/ABI/alignment/checksum before pointer fixup;
- recook deterministically from the same glTF hash;
- load writable buffers zero-copy within the target worker;
- stream/unload repeated static tiles without leaks or stale handles.

### B3-2 — cook correctness

- Blender fixture exports every supported shape and usage combination;
- Box3D cooked debug geometry matches authored bounds/transforms;
- mesh weld/edge/material results are checked;
- dynamic concave and malformed shapes fail;
- deterministic output and source/cook hashes are tested;
- physics and audio select the intended overlapping/different node subsets.

### B3-3 — sealed worker

- tracked allocator records zero allocations in sealed step, command, query,
  event and pose paths;
- all fixed capacities reach and deterministically reject overflow;
- no recurring payload-bearing postMessage after bootstrap;
- long spawn/despawn/contact/query churn plateaus memory and queues.

### B3-4 — integrated runtime

On the Ryzen 7 6800U with Dungeon and the final Steam Audio pass active:

- physics sustains 60 fixed ticks or an explicitly selected lower gameplay tier;
- render remains hardware WebGPU at its accepted presentation target;
- AudioWorklet records zero deadline misses;
- worker count and CPU contention stay within measured policy;
- 10/30/60-minute soaks show plateaued memory, queues, body/contact counts and
  no stale IDs, event loss, allocator violations or nondeterministic divergence.

---

## 12. Deferred implementation sequence

Do not begin these phases until the Steam Audio integration plan's production
AudioWorklet and integrated audio gates pass.

1. **B3-0 spike:** pin/evaluate Box3D native+wasm and current compound bugs.
2. **FFI/toolchain:** add sys crate, Nix fixed sources and target builds.
3. **Blender/schema/cook:** implement structural metadata and Box3D derived
   chunks.
4. **Worker vertical slice:** one static compound, dynamic boxes/capsules,
   fixed-step poses/events.
5. **Sealed pools:** body/template/joint/query/event capacities and allocator
   proof.
6. **Engine integration:** ECS bindings, frame order, public handles and docs.
7. **Editor visualization:** authored/cooked overlays and diagnostics.
8. **Contention/soak:** run physics with final rendering and audio workloads.
9. **Promotion:** remove Rapier/demo paths, update API/book/capacity docs, make
   Box3D canonical.

---

## 13. Definition of done

Box3D integration is complete only when glTF-authored explicit colliders cook
reproducibly into versioned target-safe derived data, a fixed-memory worker runs
them without sealed allocation, page ECS receives bounded generational pose and
event records through RingBuffers, authored and cooked shapes are visible in the
editor, and long render+audio+physics soaks meet all target budgets with no
second physics backend.
