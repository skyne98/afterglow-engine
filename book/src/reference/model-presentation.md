# Model Presentation

Afterglow provides small bootstrap utilities rather than a monolithic character
system:

- `ModelPrimitives(capacity)` collects meshes into fixed storage and reports
  overflow explicitly.
- `computeDeformedBoundsInto()` evaluates static, morphed, or skinned vertex
  positions into caller-owned box/vector scratch.
- `normalizeModelPivot()` applies target-height scaling, X/Z centering, and Y
  grounding to an engine-owned presentation parent.
- `groundDeformedModel()` grounds the currently evaluated animated pose.
- `AnimationSet` precreates a bounded action set and updates only an enabled,
  active clip.
- `SkeletonDebugAdapter` owns helper visibility, scene attachment, and disposal.

## Unified model LOD

`ModelSystem` gives cooked disk models and runtime RAM models the same bounded,
generational handles and stable revision views. Cooked records enter through
`loadCookedModel()` and `adoptCookedModel()`; runtime geometry enters through
`createRuntimeModel()` and is processed asynchronously by the engine-owned
meshoptimizer service. Complete revisions publish atomically while previous
levels remain renderable.

Rigid, skinned, and morphed primitives all receive LODs. Meshoptimizer evaluates
UV, normal, skin-weight, tangent, color, and morph-envelope error while locking
incompatible joint seams. Every base attribute, joint/weight stream, and morph
target follows the same compact remap. `ModelLodBinding` keeps the complete
skeleton and animation graph shared by all levels and switches one visible mesh
using projected coverage plus hysteresis; bone reduction is not performed.

The compact `static-lod` cook currently remains a rigid one-primitive disk
adapter. Full parsed glTF rigs enter the same `ModelSystem` through RAM until the
compact cooked format carries complete rig/morph metadata. The canonical
Avocado demo uses `loadCookedModel() -> ModelSystem.adoptCookedModel() ->
ModelSystem.createBinding()` and never constructs a geometry arena itself.

Games still choose presentation height, active clip, grounding, camera,
lighting, and shadow policy. Active model count, pending optimizer work, CPU
geometry bytes, completion publication, and LOD count all have explicit
capacities. CPU geometry bytes are enforced. Asset composition uses
`EngineAssets.createModelSystem()`; standalone cooked presentation uses
`await ModelSystem.open()` and registers that closeable owner with the runtime.
Every `ModelSystem` requires a
`geometryArena` bucket configuration and owns those fixed, prewarmed
Three-compatible slots; there is no unbounded engine-model mode. Complete LOD
publications swap atomically, while Three still creates the physical WebGPU
buffers. Every rigid,
skinned, and morph bucket must be rendered during warm-up, and current GPU/soak
evidence remains a release gate.
