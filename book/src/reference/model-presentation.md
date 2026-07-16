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

Games still choose presentation height, active clip, whether to ground, camera,
lighting, and shadow policy. Capacity failure occurs during bootstrap instead of
silently growing model bookkeeping during gameplay.
