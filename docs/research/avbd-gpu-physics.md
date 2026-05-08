# AVBD (Augmented Vertex Block Descent) — GPU Physics

## Overview

AVBD is a GPU-accelerated physics simulation method from **Giles, Diaz, and Yuksel (2025)**, SIGGRAPH 2025, University of Utah / Roblox. It extends Vertex Block Descent (VBD) with an augmented Lagrangian formulation to handle **hard constraints** (infinite stiffness), **high stiffness ratios**, and **complex contact** scenarios (stacking, friction, articulated bodies, soft bodies).

**Key result**: 3.5ms per frame (9.8ms including collision detection) for millions of objects on an RTX 4090.

## Papers

| Title | Venue | Link |
|---|---|---|
| Augmented Vertex Block Descent | SIGGRAPH 2025 (TOG) | [Paper](https://graphics.cs.utah.edu/research/projects/avbd/Augmented_VBD-SIGGRAPH25.pdf) |
| Crazy Fast Physics! AVBD in Action! | SIGGRAPH 2025 Real-Time Live! | [RTL Abstract](https://graphics.cs.utah.edu/research/projects/avbd/Augmented_VBD-SIGGRAPH25_RTL.pdf) |

## Core Algorithm (Algorithm 1)

```
for each time step:
  1. Collision detection from current state x^t
     a. Broad phase: LBVH (GPU) → candidate body pairs
     b. Narrow phase: discrete contact manifold generation
     c. Warm-start: persist contact state across frames
  2. Coloring (greedy graph coloring for parallel safety)
  3. Inertial target y and primal init, warm-start dual/stiffness
  4. For each iteration (low count, ~5-10):
     For each color:
       a. Accumulate inertial + constraint contributions
       b. Assemble local rigid-body system
       c. AVBD primal update (approximate Hessian, Section 3.5)
     d. Dual variable update (augmented Lagrangian)
     e. Stiffness ramp update
  5. Finalize velocities from updated positions
```

## Key Technical Contributions

### 1. Augmented Lagrangian for VBD
- Standard VBD handles soft constraints via stiffness — can't model infinitely stiff joints/contacts
- AVBD introduces **dual variables** per constraint, updated after each primal sweep
- Allows **hard constraints** (joints, rigid-body contacts) with no penetration
- Combined with **stiffness ramping**: start soft, increase over iterations

### 2. Penetration-Free Contact
- Novel GPU collision detection guarantees no penetrations during integration
- Compatible with VBD-like solvers (also works with Newton-based, less optimal)
- Builds on "Offset Geometric Contact" (same SIGGRAPH 2025) for reliable GPU contact

### 3. GPU Parallelism Strategy
- **LBVH** (Linear Bounding Volume Hierarchy) built and traversed entirely on GPU
- **Greedy coloring** ensures all bodies of same color can be solved in parallel
- **Per-body constraint gather**: builds constraint lists for each body on GPU
- In-place colored solve (paper uses double-buffered; current implementations use in-place)

### 4. Constraints Supported
- **Collisions**: rigid-body stacking, friction (Coulomb model)
- **Joints**: hard limits, limited degrees of freedom, articulated bodies
- **Springs**: various stiffness levels
- **Soft bodies**: deformable + rigid coupling

## Implementations

### Reference (WebGPU)
- **Repository**: [github.com/jure/webphysics](https://github.com/jure/webphysics)
- **Language**: TypeScript, WebGPU compute shaders
- **Status**: Experimental proof of concept
- **Pipeline**: `collision detection → coloring → inertial/primal init → colored primal solves → dual updates → finalize velocities`
- **Files of interest**:
  - `src/physics/PhysicsEngine.ts` — main orchestration
  - `src/physics/gpu/broadPhase.ts` — LBVH broad phase
  - `src/physics/gpu/contactGeneration.ts` — narrow phase + manifolds
  - `src/physics/gpu/contactRecord.ts` — warm-start contact state
  - `src/physics/gpu/avbdState.ts` — coloring, primal solve, dual update, velocity finalize

### 2D Demo (C++)
- [github.com/savant117/avbd-demo2d](https://github.com/savant117/avbd-demo2d)
- Demonstrates AVBD core in 2D with interactive sandbox

### 3D Demo (C++)
- [github.com/savant117/avbd-demo3d](https://github.com/savant117/avbd-demo3d)
- 3D rigid-body stacking, joints, soft bodies

## Comparison to Alternatives

| Method | Penetration-free | GPU | Hard constraints | Stiffness ratios | Speed |
|---|---|---|---|---|---|
| IPC | Yes | Slow | Yes | Yes | Not real-time |
| PBD/XPBD | No | Yes | Approx | Poor | Fast |
| VBD | No | Yes | Approx | Poor | Fast |
| **AVBD** | **Yes** | **Yes** | **Yes** | **Excellent** | **Real-time (3.5ms)** |

## Relevance to Afterglow-Engine

AVBD is ideal for a Bevy-based GPU physics plugin because:

1. **Full GPU pipeline**: Broad phase, narrow phase, solver — all compute shaders
2. **No CPU bottleneck**: Unlike PhysX or Jolt, the entire simulation stays on GPU
3. **Suitable for mass simulations**: Chainmail, sand, cloth, rigid-body piles
4. **Well-specified algorithm**: Algorithm 1 in the paper maps directly to compute shader dispatch
5. **WebGPU reference exists**: Can be ported to WGSL + wgpu native (Bevy's backend)

### Integration Path
1. Port `avbdState.ts` (colored primal solve + dual update) to WGSL compute shader
2. Port LBVH broad phase → compute shader
3. Port contact generation → compute shader
4. Integrate as a Bevy plugin with `PhysicsEngine` resource
5. Sync transforms back to Bevy ECS after simulation step

### Challenges
- Bevy's ECS owns transforms — need efficient read/copy pattern
- WebGPU reference is single-threaded dispatch per color — could pipeline
- Soft-body requires mesh data access (vertex positions)
- Friction model needs warm-start persistence across frames

## References

- Giles, Diaz, Yuksel. "Augmented Vertex Block Descent." SIGGRAPH 2025.
  [PDF](https://graphics.cs.utah.edu/research/projects/avbd/Augmented_VBD-SIGGRAPH25.pdf)
- Project page: [graphics.cs.utah.edu/research/projects/avbd/](https://graphics.cs.utah.edu/research/projects/avbd/)
- WebGPU implementation: [github.com/jure/webphysics](https://github.com/jure/webphysics)
- Two-Minute Papers: [youtube.com/watch?v=TzIKbjuSy2A](https://www.youtube.com/watch?v=TzIKbjuSy2A)
- HN Discussion: [news.ycombinator.com/item?id=44334403](https://news.ycombinator.com/item?id=44334403)
