# Afterglow editor plan

**Status:** architecture recorded; implementation deferred until Steam Audio integration, then Box3D foundations  
**Date:** 2026-07-18  
**Scope:** web editor, Blender boundary, project/scene sources, play-in-editor, and later collaboration

Related documents:

- [`spatial-audio-integration-plan.md`](spatial-audio-integration-plan.md) — sole audio pass and acoustic authoring
- [`box3d-physics-integration-plan.md`](box3d-physics-integration-plan.md) — collider schema, cooking and worker
- [`no-runtime-allocation-constant-time-budget-plan.md`](no-runtime-allocation-constant-time-budget-plan.md) — gameplay/runtime discipline
- [`../api/engine-memory.md`](../api/engine-memory.md) — current phases and fixed storage

This plan preserves the editor decisions while implementation focus returns to
Steam Audio. The Yjs-versus-Loro prototype is intentionally deferred and must
select one CRDT before collaboration ships.

---

## 1. Product boundary

The Afterglow editor is a **world assembler, component inspector, cook frontend,
and engine debugger**. It is not a second Blender.

Blender remains responsible for:

- render mesh modeling, UVs, materials and rigging;
- collision/proxy mesh editing;
- acoustic structural mesh editing;
- convex point geometry and primitive placement;
- animation authoring;
- per-model Afterglow metadata through a small Blender add-on.

The Afterglow editor is responsible for:

- placing assets and entities into worlds;
- hierarchy, names, transforms and components;
- cameras, lights and gameplay configuration;
- physics body/material/filter configuration at instance level;
- audio source/listener configuration;
- authored and cooked physics/acoustic visualization;
- deterministic source validation, cooking and diagnostics;
- play-in-editor from an immutable revision;
- later, multi-user source collaboration.

Do not implement mesh/UV/sculpt/texture/animation editing, an alternate renderer,
an alternate physics engine, or an alternate audio mixer.

---

## 2. One engine, editor phase

The editor uses the real engine:

- Three.js WebGPU-only renderer;
- the canonical ECS and hierarchy;
- the canonical asset loader and `.big` parser;
- the Box3D physics Worker once integrated;
- the sole `EngineAudio` AudioWorklet pass;
- the same diagnostics, frame budget and worker transports.

Add an explicit `EnginePhase.Editor`. Editor source manipulation may allocate,
use promises, create UI objects and mutate structures. This does not relax
`GameplaySealed`: play-in-editor creates a distinct gameplay world/runtime from
a captured source revision, warms it, seals it, and runs the normal allocation
and capacity gates.

Incoming editor changes never mutate an active sealed play world. Stop/restart
play or explicitly apply a supported bounded live-tuning subset. Version one
simply restarts.

The editor is authored TypeScript and runs in the native-shell page (or browser
page for public web) like the engine. `afterglow-shell` is a thin host and
gains no editor IPC.

---

## 3. Source and generated formats

Use three layers with one owner each:

```text
model.glb / level.structural.glb
  geometry, transforms, model-level physics/audio metadata

world.afterglow.scene.json
  entities, hierarchy, components, asset references, instance overrides

*.big
  generated deployment container: render, Box3D, acoustic and other cooked data
```

### glTF/GLB

GLB remains the durable model format. Modular assets may carry explicit collider
and acoustic nodes beside render nodes. Large worlds may use independently
streamable `*.structural.glb`. The Blender add-on writes validated glTF `extras`;
the pipeline routes nodes by consumer usage.

### Scene JSON

The editor needs an engine scene format because arbitrary ECS/game components,
asset references, capacities and world settings are not model geometry. Keep it
small, versioned, deterministic and human-reviewable. It references assets and
stable authored IDs; it never embeds GLB, Box3D, acoustic, texture or mesh bytes.

Initial top-level shape:

```json
{
  "format": "afterglow-scene",
  "version": 1,
  "project": "sample-game",
  "worldId": "uuid",
  "entities": [],
  "settings": {},
  "capacities": {}
}
```

Entities use stable UUIDs in source. Runtime cooking maps them to bounded numeric
IDs/handles. Components are registered by versioned type IDs and validated by
the owning engine/game schema. Serialization sorts entities, component types and
map keys deterministically.

### Generated `.big`

`.big` holds generic blob/mesh/texture/VT records and specialized consumers
interpret their blobs. All generated records include source hashes and cook
fingerprints. Generated output is never edited by hand and is reproducible from
glTF plus scene JSON.

---

## 4. Minimal editor interface

The first useful editor has only:

1. **Viewport** — canonical WebGPU render, grid, selection outline and gizmos.
2. **Outliner** — fixed world/entity hierarchy, search outside hot paths.
3. **Inspector** — transform and registered component fields.
4. **Asset browser** — project-scoped GLB/scene/texture/audio references.
5. **Diagnostics/cook panel** — source errors, capacities, hashes and task state.
6. **Play/stop** — immutable revision into a separately sealed runtime.
7. **Save revision** — validate and deterministically write scene source.

Panels, docking, theming and plugin APIs beyond these are YAGNI until the
vertical slice works. Use accessible HTML/CSS controls around the WebGPU canvas;
do not put authored JavaScript inline in HTML.

Transform gizmos operate on engine numeric transforms, support local/world
translation and rotation first, and group one drag as one undo transaction.
Scale follows only after collider/nonuniform-scale validation is clear.

---

## 5. Physics and audio authoring views

The viewport can switch overlays without changing source geometry:

- render geometry;
- authored physics proxies colored by shape/body/material/layer;
- cooked Box3D geometry and compound child/AABB diagnostics;
- authored acoustic surfaces colored by material/usage;
- cooked acoustic triangles/dynamic instances;
- audio source/listener ranges and priority/reflection assignment;
- validation differences between authored and cooked bounds/IDs/materials.

The editor invokes cookers against a captured source revision hash and displays
the resulting immutable diagnostics. It does not call Box3D or Steam Audio cook
APIs ad hoc on every mouse move. Debounce preview cooking with cancellation and
explicit operation/time/byte budgets.

Hard errors block Save Revision/Play:

- missing/duplicate stable IDs;
- stale asset references;
- hierarchy cycles;
- component schema failure;
- missing collider/audio material metadata;
- dynamic concave Box3D geometry;
- unsupported scale/deformation;
- cooker version/ABI mismatch;
- configured runtime capacity excess.

---

## 6. Blender add-on

Build a small versioned Afterglow Blender add-on, not a geometry exporter. It
uses Blender's official glTF exporter and provides validated UI for:

- usage mask: render/physics/audio/navigation as supported;
- stable authored ID;
- static/kinematic/dynamic/trigger role;
- Box3D box/sphere/capsule/convex/trimesh/heightfield/compound metadata;
- physics material, collision layer and mask;
- acoustic material coefficients/category;
- export selected model or structural collections;
- local validation matching the Rust cooker where practical.

Canonical data is explicit metadata, not `UCX_`-style names. Names may be shown
or imported as migration hints. Export fixtures prove every Blender field lands
in the expected glTF node `extras` and round-trips source IDs/transforms.

Do not make Blender invoke runtime workers. It exports source; the Afterglow
pipeline/editor owns deterministic cooking.

---

## 7. Project tooling boundary

Add an editor development command only when implementation starts:

```sh
cargo xtask editor <project-root>
```

It starts the existing bounded development serving mechanism plus a project-
scoped editor tooling endpoint for:

- enumerating allowed source assets;
- reading/writing deterministic scene revisions;
- starting/canceling bounded pipeline cooks;
- reporting cook output and diagnostics;
- watching source hashes for external Blender changes.

The service binds loopback by default, receives a random session token, confines
all paths beneath the declared project root, uses bounded workers/queues, and
never exposes arbitrary command execution. Remote collaboration is a separate
authenticated service and does not gain filesystem or cook permissions.

This tooling endpoint is editor/server I/O, not a second page↔engine-worker
transport. Page↔physics/audio/asset worker payloads remain RingBuffer-only.

---

## 8. Editor state and undo before collaboration

Represent source state independently from runtime ECS objects. The editor
projects validated source records into its edit ECS; UI widgets modify source
through versioned editor commands, then update the projection.

Initial commands include:

- create/delete entity;
- reparent/reorder;
- set name/transform;
- add/remove component;
- set component field;
- set asset reference;
- set project capacity/setting.

Each command has validate/apply/invert behavior and one transaction origin.
Undo/redo is local and bounded by configured command count/bytes. Large asset
bytes never enter history.

Do not freeze a bespoke collaborative operation log around this local stack.
Before collaboration starts, the Yjs/Loro prototype determines the canonical
shared document and undo model; any superseded local history implementation is
removed rather than retained alongside the CRDT.

---

## 9. Collaboration decision gate

Collaboration is deferred until the single-user editor vertical slice and Steam
Audio/Box3D source schemas are stable. Then prototype exactly two candidates:

- Yjs with a central WebSocket provider/Hocuspocus;
- Loro 1.x with its movable tree and version-control model.

Use the same deterministic 1,000-entity workload:

- 10 users concurrently create/delete/reparent/reorder entities;
- transform drags, component edits and undo/redo;
- deliberate hierarchy conflicts and offline reconnect;
- 100,000 operations followed by snapshot/reload;
- presence for selection, cursor/ray, camera and drag preview;
- server persistence, reconnect and corruption recovery;
- memory/update/snapshot bytes, merge latency and load time;
- deterministic export to identical scene JSON;
- schema migration and unknown component preservation;
- permission/rate/size rejection.

Selection criteria:

| Criterion | Weight |
|---|---:|
| Correct hierarchy move/concurrent reparent behavior | required |
| Deterministic scene export and cycle handling | required |
| Offline merge/reconnect correctness | required |
| Server auth/persistence/backups | required |
| Document/update/snapshot memory and latency | high |
| Per-user undo and transaction grouping | high |
| Presence ecosystem | medium |
| Rust/JS cross-language support | medium |
| Operational complexity and maintenance | high |

Choose one and delete the other prototype and artifacts. Do not abstract both
behind a speculative CRDT interface and do not ship both.

Current hypothesis, not a decision: Yjs+Hocuspocus is the lower-risk operational
default; Loro's movable tree/version control may be the better scene-document
model if the prototype proves its smaller server/presence ecosystem manageable.

---

## 10. Collaboration data model requirements

Whichever CRDT wins, synchronize editor **source**, never runtime state.

Persisted:

- entities/components/hierarchy/transforms/names;
- asset references and shared settings;
- optional comments only when product scope adds them.

Ephemeral presence:

- user identity/color;
- selection/hover;
- viewport camera and pointer ray;
- active gizmo/soft lock;
- throttled transform preview.

A drag broadcasts lossy preview and commits one final grouped source
transaction. Assets are immutable content-addressed references, not CRDT blobs.
Cooked physics/audio/render output is generated from an explicit captured CRDT
revision hash.

The collaboration server persists updates plus compact snapshots and enforces
room-level authentication, byte/rate limits, awareness limits and backups. A
**Save Revision** validates one CRDT revision and emits deterministic scene JSON
for Git/builds. Reconstructing a live document repeatedly from JSON is forbidden
because it loses operation identities and offline merge history.

Collaboration runs only in `EnginePhase.Editor`. It is disconnected from the
separate sealed play runtime.

---

## 11. Implementation order

This order is deliberately behind current engine priorities.

1. **Finish Steam Audio integration** through real AudioWorklet and integrated
   render/audio gates.
2. **Complete Box3D B3-0/B3-2 foundations** so authored/cooked physics data is
   real rather than mocked.
3. **Blender add-on/schema fixtures** shared by physics and audio.
4. **Scene source schema and deterministic serializer/validator**.
5. **Single-user vertical slice:** viewport, outliner, inspector, gizmo,
   asset placement, save/load.
6. **Cook/debug overlays:** authored versus Box3D/acoustic output.
7. **Play-in-editor:** captured revision into separate warmed/sealed runtime.
8. **Tooling confinement/cancellation/telemetry**.
9. **Yjs versus Loro 1,000-entity prototype**.
10. **Select one collaboration stack; delete the loser**.
11. **Multi-user hardening:** auth, persistence, backups, rate limits and soak.

---

## 12. Acceptance gates

### ED-0 — source round-trip

- Blender fixtures export every supported metadata field;
- model GLB and companion structural GLB routes are equivalent;
- scene JSON serializes deterministically and rejects malformed/cyclic sources;
- load→save without edits is byte stable;
- source hash deterministically selects all cooked artifacts.

### ED-1 — single-user vertical slice

- create/place/reparent/transform/delete/save/reload works through public editor
  commands;
- no panel mutates renderer/physics/audio internals directly;
- undo transactions restore exact source and projection;
- external Blender updates revalidate/cook without unbounded tasks;
- all tooling paths are project-confined and queues bounded.

### ED-2 — authored/cooked truth

- Box3D and acoustic overlays match source transforms/IDs/materials;
- cook cancellation/stale-output rejection works;
- capacity/ABI/version failures are visible before play;
- no generated artifact can be edited as source.

### ED-3 — play isolation

- play captures one source revision and creates a separate runtime;
- gameplay warms/seals through canonical APIs;
- collaborative/editor mutations cannot enter the active play world;
- stop disposes all play workers/audio/render resources and restores edit state.

### ED-4 — collaboration selection

- both bounded prototypes run the identical checked-in workload;
- one passes all required correctness criteria and wins by recorded evidence;
- the other dependency, code, artifacts and docs are deleted;
- selected server persistence/backup/recovery passes destructive tests.

### ED-5 — long collaborative soak

- multi-user hierarchy and transform editing converges;
- memory, update log, snapshots, connections and pending tasks stay bounded or
  compact according to policy;
- reconnect/offline edits preserve data;
- deterministic saved revisions and derived cooks match on every client;
- no collaboration code or connection survives transition into packaged game
  builds.

---

## 13. Definition of done

The editor is complete when artists author geometry in Blender/glTF, designers
assemble deterministic scene sources through the real engine viewport,
physics/audio cooks and diagnostics expose authored-versus-runtime truth,
play-in-editor runs a separate canonical sealed world, and one—not two—measured
CRDT stack supports durable multi-user editing. It remains a focused world and
engine editor rather than a replacement DCC.
