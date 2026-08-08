# Unified model and LOD system

## Public runtime API

`presentation/index.ts` exports:

- `ModelSystem(optimizer, options)` — fixed model handles, pending optimizer
  capacity, resident CPU-geometry byte limit, and bounded completion publication;
- `ModelSystem.createRuntimeModel(geometry)` — registers canonical RAM geometry
  and starts deformation-aware meshoptimizer processing;
- `ModelSystem.reviseRuntimeModel(handle, geometry)` — starts a complete new
  immutable revision while retaining published LODs;
- `ModelSystem.adoptCookedModel(asset)` — transfers offline meshopt LODs into
  the same handle/view namespace without another optimization pass;
- `ModelSystem.getView(handle)` — stable status/revision/LOD/byte view;
- `ModelSystem.createBinding(...)` — creates one `ModelLodBinding` after the
  model is ready;
- `buildModelGeometryLods(geometry, optimizer, options)` — the policy-free
  geometry processor used by RAM-backed models;
- `loadCookedModel(options)` and `CookedModelAsset` — the disk/BIG adapter for
  already optimized rigid LOD records;
- `projectedCoverage(...)` — normalized projected diameter for LOD selection.

Texture and model streaming share `FixedResourceRegistry`: opaque numeric
handles, fixed slots, generations, stale-handle rejection, and deterministic
capacity failure. Their source decoding and GPU publication remain separate:
textures publish atlas/page-table records while models publish geometry levels.
`EngineAssets.createModelSystem(options)` and `createAssetStore()` share the one
engine-owned meshoptimizer service and close it only after both consumers. A
standalone cooked-model presentation can use
`await ModelSystem.open(options, telemetry)`; it owns its generated platform mesh
worker and must be registered with `EngineRuntime.ownCloseable()`.

## Rigged and morphed simplification

Rigged and morphed primitives use the same LOD processor as rigid geometry.
`buildModelGeometryLods()` sends one bounded continuous error stream to
meshoptimizer containing available UVs, normals, skin weights, tangents, colors,
and a morph-position envelope. Coincident vertices with incompatible discrete
joint sets are locked against collapse. After simplification, every retained
base attribute, skin index/weight stream, and every morph target follows the
same compact vertex remap; material groups are simplified independently and
retained.

`ModelLodBinding` clones the source mesh per level and keeps one complete shared
skeleton, bind matrix, animation graph, morph dictionary, and morph-influence
array. Bone reduction is intentionally not part of geometry LOD. Selection uses
strictly descending projected-coverage thresholds and allocation-free
hysteresis; exactly one level is visible.

Runtime revisions are atomic at the model level. If processing or CPU-byte
admission fails, existing published LODs remain usable. Destroying a handle
invalidates late worker completions and disposes all owned geometries.

`maxResidentCpuBytes` is enforced. `GeometryArena` is mandatory for every
engine-owned model. `ModelSystemOptions.geometryArena` supplies bootstrap-defined
layout buckets; `ModelSystem` creates, owns, reports, and disposes the arena.
Buckets own fixed `BufferGeometry` slots, persistent typed attributes, group
records, generations, exact logical GPU bytes, and atomic complete-LOD
publication. There is no unbounded publication branch. Old slots remain
published until every replacement slot is admitted and copied.

Three still creates the physical WebGPU buffers for those persistent attributes.
Every arena slot must therefore be rendered with its matching rigid/skinned/
morph material variant during warm-up. The Avocado LOD demo now exercises all
four arena slots before seal. Current rigid/skinned/morph real-GPU prototype and
long-soak evidence remain required before the arena is a completed release gate;
there is no private renderer patch or claimed one-buffer suballocation path.

## Offline cooked adapter

```sh
cargo run -p afterglow-pipeline -- \
  static-lod assets/lod-demo/Avocado.gltf \
  crates/afterglow-web/web/assets/lod-demo.big
```

The current compact BIG mesh command remains a rigid, one-primitive adapter
containing position/UV records at 100%, 50%, 25%, and 10%. It is consumed by
`loadCookedModel()` and adopted into `ModelSystem`. Full glTF rigs and morphs
currently enter `ModelSystem` after rig-preserving GLTF parsing and are processed
through the runtime mesh worker; extending the compact offline record to carry
complete rig/morph metadata remains a source-format gate, not a separate LOD
policy.

## Validation

Unit tests enforce:

- deformation-aware worker RPC and vertex locks;
- identical compaction of base, skin, and morph attributes;
- material-group retention;
- shared skeleton and morph state across levels;
- atomic RAM-model revision publication;
- cooked and RAM models sharing one bounded handle namespace.

The LOD demo uses `loadCookedModel()` plus `ModelLodBinding`; game code never
constructs mesh workers or RPC transports.
