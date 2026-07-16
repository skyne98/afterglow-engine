# NVIDIA Micro-Mesh / Displacement Micro-Map (DMM)
https://developer.nvidia.com/rtx/ray-tracing/micro-mesh
SDK: https://github.com/NVIDIAGameWorks/Displacement-MicroMap-SDK
Vulkan ext: `VK_NV_displacement_micromap` (RTX 30/40+, beta drivers)

## What it is
Hardware-accelerated graphics primitive: render highly detailed objects as
**locally adaptive subdivided triangles** (micro-triangles) with **barycentric-
map compression**. Displacement applied by remapping a depth/displacement
texture to the micro-triangle barycentric coordinates.

## Key properties
- **No UV mapping needed** — sampling is implicit via barycentrics.
- Primarily a **ray-tracing compression** mechanism: encode a detailed source
  model compactly so RT traversal hits micro-triangles directly (vs a fat
  triangle BLAS). Superior RT performance + memory compression.
- Supports displacement maps as input (remesh from depth texture to bary map).
- **Not** interactive-editing friendly (precompute/remesh required).
- RTX 40 hardware path; Vulkan via the NV extension and OptiX. Two SDKs:
  low-level Displacement Micro-Map SDK + Python Toolkit for DCC integration.

## vs PDM (Hoetzlein 2025)
Micro-Meshes tessellates/compresses detailed source geometry. PDM is faster on
most low-poly + displacement-map models (no BLAS to build/traverse) but is not
aimed at compressing massive static scenes (Micro-Meshes / Nanite are).
