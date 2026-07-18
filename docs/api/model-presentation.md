# Model presentation utilities

The public browser entrypoint `web/src/engine/presentation/index.ts` exports focused bootstrap ownership
primitives for static, morphed, and skinned glTF scenes. It deliberately does
not define character, camera, clip-selection, grounding, or lighting policy.

## `ModelPrimitives`

`ModelPrimitives(capacity)` owns a fixed `(Mesh | null)[]` and one retained
traversal callback. `collect(root)` returns `Complete` or `CapacityExceeded`; it
never silently grows. The first `capacity` meshes remain available through
`items` and `count`.

`computeDeformedBoundsInto(primitives, outBox, vertexScratch)` calls
`Mesh.getVertexPosition()` for every source vertex, so skinning and morph targets
participate, transforms into world space, and writes only caller-owned scratch.

`normalizeModelPivot(pivot, targetHeight, box, size, center)` scales an
engine-owned presentation pivot, centers X/Z, and grounds Y. It returns explicit
invalid-height or empty-bounds statuses. `groundDeformedModel()` performs the Y
adjustment against the current evaluated deformed pose.

## `AnimationSet`

`AnimationSet(root, clips, capacity)` rejects clip counts over capacity and
creates every `AnimationAction` during construction. `play(index)` returns
`false` for invalid/unavailable clips. Disabled or inactive sets do not update
the mixer. `dispose()` stops actions, uncaches the root, clears fixed slots, and
is idempotent.

## Skeleton diagnostics

`SkeletonDebugAdapter(scene, root)` owns one hidden `SkeletonHelper`, visibility,
scene attachment, and idempotent disposal. It is diagnostic presentation, not a
shadow-casting game primitive.
