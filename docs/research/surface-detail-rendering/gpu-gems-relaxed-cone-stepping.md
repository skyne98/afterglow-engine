# Relaxed Cone Stepping for Relief Mapping (GPU Gems 3, Ch.18)
Fabio Policarpo & Manuel M. Oliveira.
https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-18-relaxed-cone-stepping-relief-mapping

## Family
Relief mapping = ray-march a heightfield per fragment to find the true surface
intersection (correct self-occlusion; real-looking depth unlike parallax).
Naive relief marching takes many fixed steps.

## Cone Step Mapping (CSM) — Donnelly & Wloka 2005 / Dummer 2006
- Precompute a **cone map**: at each texel store the cone ratio (width/height)
  of the largest cone that fits entirely *below* the heightfield at that texel.
  A marching ray can **leap forward** by the cone radius conservatively
  without passing through the surface.
- Far fewer steps than fixed-step POM.
- Drawback: conservative cones are large on flat areas; rays can never *enter*
  an overhang (heightfield-only).

## Relaxed Cone Stepping (RCS) — the GPU Gems 3 contribution
- **Relaxes** the cone definition: rays may *enter* a relief surface but never
  *leave* it. Renders **non-height-field structures** (overhangs, multiple
  layers, impostors of 3D objects) — POM cannot.
- Binary-search refinement of the final hit. One cone channel + height map.

## Anisotropic Cone Step Mapping — Chen & Chuang 2009
- Cones stored in **two directions** (u, v) to handle anisotropic surfaces,
  leaping farther along the flatter axis.

## ARCSM — Axiom VFX, 2025 (current raster SOTA claim)
"Anisotropic Relaxed Cone Step Mapping." Precomputed accel structure + custom
HLSL. Claims the **fidelity of thousands of POM steps at almost the cost of a
normal map.** https://www.axiomvfx.com/arcsm

## Robust Cone Step Mapping — 2024
Fixes two correctness bugs:
1. At bilinearly-interpolated cone values the unbounding cones can violate the
   heightfield → artifacts. New generation keeps cones disjoint.
2. Exact method to generate relaxed cones guaranteeing any in-ray intersects
   the heightfield **at most once** (original was costlier & wrong).
