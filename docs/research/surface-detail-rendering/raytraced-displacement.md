# Ray-traced displacement — Thonat et al. (2021 D-BVH / 2023 RMIP) + Ogaki 2023 + Haydel 2023

## Tessellation-Free Displacement Mapping for Ray Tracing (2021, ACM TOG)
https://dl.acm.org/doi/10.1145/3478513.3480535
- Constructs a **D-BVH**: a min-max mipmap over **affine intervals** of the
  base geometry. Displaced bounds generated dynamically from quad-tree nodes.
- Real-time, **artifacts-free inverse displacement mapping** via per-pixel ray
  tracing through an **image pyramid** on the GPU.
- No tessellation → no vertex amplification/memory blowup. Each pixel makes a
  ray and traces it through the displacement pyramid.
- Cost: D-BVH traversal per ray (significant compute).

## RMIP (SIGGRAPH Asia 2023)
https://dl.acm.org/doi/10.1145/3610548.3618182
- Displacement ray tracing via **inversion and oblong bounding**.
- **Bidirectional mapping** between 2D depth-texture space and 3D object space;
  rectangular regions allow **anisotropic** ray acceleration.
- Hierarchical data structure over the displacement map. Faster than D-BVH.
- Needs a BLAS rebuild when the displacement is edited (the problem PDM 2025
  solves by dropping the BLAS).

## Nonlinear Ray Tracing (Ogaki, SIGGRAPH Asia 2023)
https://dl.acm.org/doi/10.1145/3610548.3618199
- Maps world rays into **canonical prism volumes** where they become
  **nonlinear**, solved via cubic equations per ray step.
- Broad-phase BVH + min-max mipmaps for local displacement, or BVHs for
  instanced meso-geometry. Rays transformed at leaf prism nodes.
- Pre-computation + cubic solving → not interactive-editing friendly.

## Locally-adaptive LOD ray tracing (Haydel, Yuksel, Seiler 2023)
- Does not split the mesh; refines **each triangle** through explicit one-to-
  four subdivision. Precomputed tessellation tree selects per-triangle LOD.
- Benthin & Peters 2023: Nanite-style triangle clusters inserted into a dynamic
  GPU BVH per frame for real-time RT of micro-poly geometry.
