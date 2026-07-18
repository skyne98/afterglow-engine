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

## Static mesh LOD

The offline pipeline command `static-lod <model.gltf|glb> <output.big>` cooks a
single static triangle primitive into compressed 100%, 50%, 25%, and 10% mesh
records. Skins and morph targets are rejected rather than simplified with the
wrong semantics.

At bootstrap, `loadStaticMesh()` validates and decodes the fixed chain, closes
its decoder, and returns a disposable `StaticMeshAsset` containing only fixed
geometry levels. `LodSet` selects one precreated mesh from normalized projected coverage, with explicit
capacity and hysteresis. Selection performs no allocation and leaves exactly
one level visible. The canonical LOD demo uses one CC0 Avocado model and its GPU
regression verifies transitions `0,1,2,3,2,1,0` in both camera directions.

Games still choose presentation height, active clip, whether to ground, camera,
lighting, and shadow policy. Capacity failure occurs during bootstrap instead of
silently growing model bookkeeping during gameplay.
