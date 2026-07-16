# Projective Displacement Mapping for Ray Traced Editable Surfaces
Rama Carl Hoetzlein, Feb 2025. arXiv:2502.02011.

Most recent (2025) SOTA reference. Its Related Work (§2) is a complete
taxonomy of every displacement/parallax/relief technique with citations.

## Method
- **No BLAS.** Top-level hardware BVH (OptiX) over base-mesh prisms only.
  Editing the displacement texture needs zero data-structure rebuild.
- **Parallel offset prisms**: offset triangles all parallel to the base
  triangle (scale normal by 1/(N·Ng)) → linear world→texture projection, so
  ray samples map to displacement texels without per-sample tangent transforms.
- **Projective Displacement Mapping (PDM)**: ray-march the heightfield inside
  each prism via an incrementally-advancing "scanning triangle"; bilinear fetch
  + boundary interpolation for sub-sample accuracy.
- **Ray/bilinear-patch prism sides** (Reshetov "cool patches") → C¹-ish
  continuity, no tetrahedra, no buckling.
- **Shading normal correction**: Ns = Ng + ∇D ⊗ ∇P N′; subtract Ng, add
  interpolated N′ → Phong-like smoothing without a C¹ intermediate surface.
- **Thin features**: stochastic jitter of first sample along t (no extra cost).
- Tight per-prism bounds via precomputed max(D).

## Performance
- Detailed scene (9 objects, 4 lights, path tracing, soft shadows, refl/refr):
  15k tris, 1.8 MB GPU, 30 s @ 4096×1280, RTX 4090.
- 40–60% faster than RMIP (primary rays); 2×–13× faster (beauty), same HW.
  Faster than Micro-Meshes on most low-poly + displacement-map models.
- Interactive sculpting: 256² region @ 70 fps while ray tracing.

## Caveat
Ray-tracing technique — needs a hardware BVH + custom intersection program
(OptiX). Not portable to a pure-raster/WebGPU pipeline.
