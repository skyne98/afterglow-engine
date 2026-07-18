# Offline static-mesh LOD

## Cook

```sh
cargo run -p afterglow-pipeline -- \
  static-lod assets/lod-demo/Avocado.gltf \
  crates/afterglow-web/web/assets/lod-demo.big
```

`static-lod` accepts one glTF/GLB triangle primitive, reads POSITION, indices,
and optional TEXCOORD_0, and writes four independently compressed `.big` mesh
chunks at 100%, 50%, 25%, and 10% targets. It rejects skins, morph targets,
non-triangle topology, multiple primitives, noncontiguous records, and unsafe
sizes. Missing UVs become a fixed zero-filled channel. Simplification is an
offline operation; no runtime worker generates levels.

## Runtime API

`loadStaticMesh()` validates the bounded `.big` header, finds one named mesh
chain, checks contiguous stable LOD indices, and decodes each meshopt chunk
during bootstrap. The engine creates and closes the decoder before the promise
resolves. It returns a `StaticMeshAsset` that owns only its fixed
`BufferGeometry` levels; `dispose()` is idempotent. There is no static-LOD
"session" and game code never configures RPC or worker lifetime.

`LodSet(meshes, thresholds, hysteresis, capacity)` owns fixed selection state.
Levels are ordered finest to coarsest. Thresholds are normalized projected
coverage boundaries in strictly descending order. `select(coverage)` changes
visibility without allocation and applies symmetric hysteresis around the
current boundary. Exactly one mesh remains visible.

`projectedCoverage(radius, distance, verticalFovRadians)` computes a bounded
normalized projected diameter suitable for `LodSet.select()`.

Skinned and morphed meshes are deliberately outside this API. They require a
separate deformation-aware strategy and are rejected by the static cook.

## Demo and validation

The canonical `lod-demo.ts` presents one CC0 Microsoft Avocado model rather than
four synthetic spheres. Its offline chain contains 682, 341, 170, and 104
triangles. A deterministic camera trajectory validates transitions
`0,1,2,3,2,1,0`, exactly one visible mesh, no diagnostics, and prewarmed solid
and wireframe pipelines. Run:

```sh
DISPLAY=:0 ./scripts/test-lod-gpu.sh
```
