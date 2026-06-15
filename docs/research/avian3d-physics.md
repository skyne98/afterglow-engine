# Avian 3D Physics — Deep Dive

## Overview

**Avian** (v0.6.1, Bevy 0.18) is an ECS-driven 2D/3D physics engine built *for* Bevy *with* Bevy.
Created by Joona Aalto (jondolf). Repository: [github.com/avianphysics/avian](https://github.com/avianphysics/avian)

Unlike `bevy_rapier3d` which wraps a separate physics world, **Avian stores all physics state as
ECS components** — `RigidBody`, `Position`, `Rotation`, `LinearVelocity`, `Collider`, etc. are
all native Bevy Components. There is no separate physics World to sync.

---

## Architecture Comparison: Avian vs Rapier

| Aspect | Rapier / bevy_rapier3d | Avian |
|---|---|---|
| **World** | Separate physics world + sync glue | Pure ECS, everything is Components |
| **Rigid bodies** | Handle-based, sync each frame | Entity with `RigidBody` + `Position` + `Rotation` |
| **Colliders** | Handle-based | Entity with `Collider` component |
| **Joints** | Handle-based | Entity with joint component (e.g. `FixedJoint`) |
| **Solver** | Impulse-based (contacts + joints) | Hybrid: impulse contacts + XPBD joints |
| **Parallelism** | Island-based (1 island = 1 thread) | Graph coloring (same-color constraints parallel) |
| **Broad phase** | Sweep-and-prune (SAP) | BVH (via `obvhs` crate) |
| **Collision detection** | Built-in to Rapier | Parry (pluggable via `AnyCollider` trait) |
| **Customizability** | Hard to replace internals | High — broad phase, colliders, solver all pluggable |
| **Source complexity** | Large glue/sync layer | Clean, modular, approachable |
| **Documentation** | Poor on docs.rs, rapier.rs outdated | Excellent on docs.rs, well-organized |
| **Maturity** | Very mature, battle-tested | v0.6, growing fast, slightly less stable |

---

## Plugin Architecture

`PhysicsPlugins` is the top-level plugin group. Its `build()` assembles the engine:

```
PhysicsPlugins
├── PhysicsSchedulePlugin      — schedules, system sets, Time<Physics>
├── MassPropertyPlugin         — mass/inertia from collider density
├── ForcePlugin                 — ExternalForce, ConstantTorque, impulses
├── ColliderHierarchyPlugin     — ColliderOf parent tracking
├── ColliderTransformPlugin     — sync rigid body → child collider transforms
│
├── ColliderBackendPlugin       — initialize collider shapes (Parry)
├── ColliderTreePlugin          — BVH tree management
├── NarrowPhasePlugin           — contact manifold computation
│
├── SolverPlugins (group)
│   ├── IntegratorPlugin        — semi-implicit Euler
│   ├── SolverPlugin            — constraint solving (Gauss-Seidel)
│   ├── CcdPlugin               — swept CCD
│   ├── IslandPlugin            — persistent sleeping islands
│   ├── IslandSleepingPlugin    — sleep/wake management
│   ├── XpbdSolverPlugin        — XPBD joint solving
│   └── JointGraphPlugins       — per-joint-type graph management
│
├── BroadPhaseCorePlugin        — broad phase resources
├── BvhBroadPhasePlugin         — BVH broad phase
├── JointPlugin                 — joint lifecycle (not solving)
├── SpatialQueryPlugin          — ray/shape casting
├── PhysicsTransformPlugin      — Position/Rotation ↔ Transform sync
└── PhysicsInterpolationPlugin  — smooth rendering interpolation
```

### Optional Plugins
| Plugin | Feature | Purpose |
|---|---|---|
| `PhysicsDebugPlugin` | `debug-plugin` | Gizmo debug render (colliders, AABBs, contacts, joints, islands) |
| `PhysicsDiagnosticsPlugin` | `bevy_diagnostic` | Performance counters |
| `PhysicsDiagnosticsUiPlugin` | `diagnostic_ui` | On-screen debug UI |
| `PhysicsPickingPlugin` | `bevy_picking` | Physics-backed picking backend |

---

## Physics Pipeline

### Outer Schedule (`PhysicsSystems`)
```
First → Prepare → StepSimulation → Writeback → Last
```

### Inner Schedule (`PhysicsStepSystems`, inside `StepSimulation`)
```
First → BroadPhase → NarrowPhase → Solver → Sleeping → SpatialQuery → Finalize → Last
```

The **Solver** schedule runs the substep loop:
```
Substep × N:
  Velocity Integration (semi-implicit Euler)
  Warm Start (accumulated impulses)
  Solve Constraints (Gauss-Seidel contacts + XPBD joints)
  Position Integration
  Relax (solving again without bias)
  Damping
```

---

## Parallelism Model: Graph Coloring

Avian uses **greedy edge coloring** for constraint parallelism — fundamentally different from
Rapier's island-based approach.

**How it works:**
1. Build a constraint graph where bodies are nodes and constraints are edges
2. Color edges so no body appears in two edges of the same color
3. Solve all constraints of each color in parallel (24 colors max)
4. Overflow color (color 23) handles high-degree bodies (solved serially)

**Benefits over islands:**
- A single dense pile of bodies uses ALL cores (not 1 thread)
- Scales better with constraint count
- No need to split/merge islands

**Tradeoffs:**
- More solver iterations needed (colors are solved sequentially, so convergence is slower per-pass)
- 24 colors = 24 sequential passes, but each pass uses all threads
- Rapier's fully-coupled island solve converges faster per-iteration

---

## XPBD Joints vs Impulse-Based Contacts

Avian uses a **hybrid solver**:

| Mechanism | Method | Why |
|---|---|---|
| **Contacts** | Impulse-based (Gauss-Seidel) | More accurate for collision response |
| **Joints** | XPBD (position-based) | More stable for stiff constraints |

### XPBD (Extended Position-Based Dynamics)
- Constraints are solved at the **position level**, not velocity level
- Position corrections are converted to velocity changes
- Naturally handles complementarity (limits, inequalities)
- Easy to add custom constraints via the `XpbdConstraint` trait

### Built-in Joint Types
| Joint | DOF Allowed | XPBD Constraint |
|---|---|---|
| `FixedJoint` | 0 (locked) | `FixedConstraint` |
| `DistanceJoint` | 2T + 3R | `DistanceConstraint` |
| `PrismaticJoint` | 1T | `PrismaticConstraint` |
| `RevoluteJoint` | 1R | `RevoluteConstraint` |
| `SphericalJoint` | 3R (3D only) | `SphericalConstraint` |

---

## Feature Flags

| Feature | Description | Default |
|---|---|---|
| `3d` | 3D physics | Yes (avian3d) |
| `f32` | Single precision | Yes |
| `f64` | Double precision | No |
| `parry-f32` | Parry collision (f32) | Yes |
| `xpbd_joints` | XPBD joint solver | Yes |
| `parallel` | Extra multithreading | Yes |
| `simd` | SIMD optimizations (via Parry simd-stable) | No |
| `collider-from-mesh` | Generate colliders from meshes (3D) | Yes |
| `debug-plugin` | Debug rendering via gizmos | Yes |
| `enhanced-determinism` | Cross-platform deterministic math | No |
| `serialize` | Serde for physics components | No |
| `validate` | Extra correctness checks | No |

---

## Performance Characteristics

Based on the single-stack benchmark on Ryzen 7 6800U (8 cores, integrated GPU):

| Bodies | Rapier (pure physics) | Avian (with rendering) | Avian Advantage |
|---|---|---|---|
| 100 | 1021 FPS / 1.0ms | 877 FPS / 1.1ms | similar |
| 400 | 661 / 1.5ms | 1170 / 0.9ms | **1.8×** |
| 900 | 427 / 2.3ms | 992 / 1.0ms | **2.3×** |
| 1600 | 182 / 5.5ms | 677 / 1.5ms | **3.7×** |
| 2500 | 132 / 7.6ms | 654 / 1.5ms | **5.0×** |
| 3600 | 92 / 10.9ms | 330 / 3.0ms | **3.6×** |
| 4900 | 58 / 17.2ms | 167 / 6.0ms | **2.9×** |

Avian is **2-5× faster** due to:
- Graph coloring parallelism (all 8 cores used even in a single pile)
- ECS-native data layout (no sync overhead, better cache behavior)
- BVH broad phase (better than SAP for dense scenes)

---

## Sleeping & CCD

**Sleeping:**
- Persistent islands via union-find (retained across frames)
- Islands merge when constraints are created, split via DFS when removed
- An island sleeps when ALL bodies' velocities stay below threshold
- Waking any body wakes the entire island
- Component-based: `Sleeping` (marker), `SleepThreshold`, `TimeToSleep`

**CCD (Continuous Collision Detection):**
- **Speculative collision** (default, always on): AABBs expanded by velocity × timestep + margin
- **Swept CCD** (opt-in via `SweptCcd` component): Sweep-based, more expensive but accurate
- `SpeculativeMargin` component controls margin per entity

---

## Known Limitations

1. **Young** — v0.6, fewer community resources than Rapier
2. **Missing features**: Articulations, soft bodies, fluids, cloth, character controller
3. **Joint motors**: Not fully supported yet ("coming in future release")
4. **Ghost collisions**: Speculative CCD can create phantom contact planes
5. **Energy absorption**: Speculative contacts absorb KE over multiple bounces
6. **Joint coloring**: Joints not yet in graph coloring (solved separately)
7. **Block solver**: Missing simultaneous two-point contact solving (TODO in source)
8. **Single-threaded physics schedule**: `PhysicsSchedule` uses `ExecutorKind::SingleThreaded` — all parallelism is manual (graph coloring, parallel loops)

---

## Key Takeaway

Avian is the strongest physics option for Bevy 0.18. Its ECS-native design eliminates the
sync tax, its graph coloring parallelism scales better on multi-core CPUs, and its modular
plugin system makes it far more hackable than Rapier. The performance advantage is
substantial (2-5× in our benchmarks), and the API is cleaner because you're just working
with Bevy components.

The main risk is maturity — at v0.6, there are more rough edges than Rapier. But the
architecture is sound, development is active, and the design is clearly the right direction
for Bevy-native physics.

## Current Engine Integration

Afterglow uses `avian3d = 0.6.1` with explicit minimal deterministic-friendly
features: `3d`, `f32`, `parry-f32`, `xpbd_joints`, and
`enhanced-determinism`. The engine deliberately does **not** enable Avian's
`parallel` feature in the core dependency, so prediction-sensitive physics is not
subject to extra threaded solver nondeterminism. It also does not enable Avian's
default debug-render, picking, scene, or mesh-collider features, because those
pull in extra Bevy resources that make minimal/headless runtime tests less clean.
Mesh-derived colliders can be added later as an explicit import/tooling path.

The public engine layer lives in
[physics.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/physics.rs:1):
`AfterglowPhysicsPlugin` adds Avian, mirrors `AfterglowPhysicsConfig.gravity`
into Avian `Gravity`, and provides small authoring components for body kind,
primitive colliders, and authored velocity.

## References
- Repository: https://github.com/avianphysics/avian
- Docs (3D): https://docs.rs/avian3d
- crates.io: https://crates.io/crates/avian3d
- Bevy migration guide available in the repository
