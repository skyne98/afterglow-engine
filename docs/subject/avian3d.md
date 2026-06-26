# Avian3d — Engine Reference

> Engine version: 0.6.1 · Bevy compat: 0.18 · License: MIT/Apache-2.0
> Author: Joona Aalto · Repository: https://github.com/avianphysics/avian
> Re-exports Parry as `avian3d::parry` when the `parry-f32` or `parry-f64` feature is enabled.

---

> **Scope:** This reference was originally written against Avian 0.5.0 source
> and updated for the engine's Avian 0.6.1 baseline. The legacy 0.5-only
> prototype (`prototype-physics-lightyear`) and upstream `lightyear_avian3d`
> crate have been retired; the engine now uses its own
> `afterglow-lightyear-avian3d` bridge on Avian 0.6.1. 0.5-only details below
> are kept as historical notes only. For the engine's current 0.6.1
> integration, see
> [`docs/research/avian3d-physics.md`](../research/avian3d-physics.md).

---

## 1. Overview

Avian is an **ECS-driven physics engine** built _for_ Bevy _with_ Bevy. Unlike `bevy_rapier` which maintains a separate physics world, Avian stores all physics state as ECS components (`Position`, `Rotation`, `LinearVelocity`, etc.). This removes synchronization overhead and makes the source approachable.

### Architecture Philosophy

- **No separate physics world** — physics data lives in Bevy's ECS
- **Modular plugins** — every major subsystem is its own plugin, replaceable at the app level
- **XPBD joints** — Extended Position-Based Dynamics for constraint solving
- **Impulse-based contacts** — contacts use a different solver than joints
- **`f32`/`f64` precision** — pick at compile time via feature flags
- **Determinism-supported** — `enhanced-determinism` feature for cross-platform bit-identical results

### Key Differences from bevy_rapier

| Avian | bevy_rapier |
|---|---|
| Physics state = ECS components | Separate physics world + sync glue |
| `Position`/`Rotation` separate from `Transform` | Uses `Transform` directly |
| `Collisions` system param for contact data | `Query` + Rapier types |
| XPBD joints | Impulse-based joints |
| `CollisionStart`/`CollisionEnd` as `Message` + `Event` | Rapier events |

---

## 2. Feature Catalog

From `Cargo.toml` and `src/lib.rs`:

| Feature | Description | Default | When to use |
|---|---|---|---|
| `3d` | 3D physics (incompatible with `2d`) | Yes | Any 3D project |
| `f32` | `f32` precision | Yes | Most projects |
| `f64` | `f64` precision | No | Large worlds, determinism needs |
| `default-collider` | Enables `Collider` with Parry backend | Yes | Almost always needed |
| `parry-f32` | f32 Parry collision detection (implies `default-collider` & `f32`) | Yes | Default choice |
| `parry-f64` | f64 Parry collision detection (implies `default-collider` & `f64`) | No | When using `f64` |
| `xpbd_joints` | XPBD joint support | Yes | When using joints |
| `collider-from-mesh` | Generate colliders from `Mesh` assets | Yes | Importing meshes as colliders |
| `bevy_scene` | `ColliderConstructorHierarchy` for scene loading | Yes | Loading glTF scenes |
| `bevy_picking` | Physics picking backend for `bevy_picking` | Yes | Click-to-select physics objects |
| `debug-plugin` | Physics debug rendering (gizmos) | Yes | During development |
| `bevy_diagnostic` | Write physics diagnostics to `DiagnosticsStore` | No | Performance profiling |
| `diagnostic_ui` | Physics diagnostics UI overlay (implies `bevy_diagnostic`) | No | In-engine performance HUD |
| `enhanced-determinism` | Cross-platform deterministic math via `libm` | No | Multiplayer/netcode determinism |
| `parallel` | Extra multithreading for large simulations | Yes | Simulations with 1000s of bodies |
| `simd` | SIMD optimizations in Parry | No | Maximum CPU throughput |
| `serialize` | Serde derives on physics components | No | Save/load, networking, replay |
| `validate` | Extra correctness checks (performance cost) | No | Debugging elusive bugs |

### Feature Interactions

- `f32` + `f64` are **mutually exclusive**
- `default-collider` requires `parry-f32` (with `f32`) or `parry-f64` (with `f64`)
- `diagnostic_ui` requires `bevy_diagnostic` + `bevy/bevy_ui`
- `debug-plugin` requires `bevy/bevy_gizmos` + `bevy/bevy_render`
- `collider-from-mesh` requires `bevy/bevy_mesh` + `bevy/bevy_mikktspace` + `3d`

---

## 3. Module Map

| Module | Contents |
|---|---|
| `src/lib.rs` | `PhysicsPlugins`, `PhysicsPluginsWithHooks`, top-level plugin group |
| `prelude` | Re-exports all common types (the one-stop `use avian3d::prelude::*;`) |
| `collision/` | Broad phase, narrow phase, collider types, contact graph, collision events, hooks |
| `collision/collider/` | `Collider` enum, constructors, hierarchy, mesh conversion |
| `collision/contact_types/` | `ContactGraph`, `ContactPair`, `ContactManifold`, `ContactPoint`, `Collisions` system param |
| `collision/collision_events.rs` | `CollisionStart`, `CollisionEnd`, `CollisionEventsEnabled` |
| `collision/hooks.rs` | `CollisionHooks` trait, `ActiveCollisionHooks` |
| `dynamics/` | Rigid body components, forces, mass properties, sleeping, CCD |
| `dynamics/rigid_body/` | `RigidBody` enum, `LinearVelocity`, `AngularVelocity`, `GravityScale`, `LockedAxes`, `Dominance`, damping |
| `dynamics/rigid_body/forces/` | `ConstantForce`, `ConstantTorque`, `ConstantAcceleration`, etc. |
| `dynamics/rigid_body/mass_properties/` | `Mass`, `AngularInertia`, `CenterOfMass`, `MassPropertiesBundle`, `ColliderDensity` |
| `dynamics/rigid_body/sleeping.rs` | `Sleeping`, `SleepingDisabled`, `DeactivationTime`, etc. |
| `dynamics/joints/` | `FixedJoint`, `DistanceJoint`, `PrismaticJoint`, `RevoluteJoint`, `SphericalJoint` (3D), `JointFrame`, `JointForces`, `JointDamping`, `JointDisabled`, `EntityConstraint` |
| `dynamics/solver/` | Solver plugins, schedule, contact solving, island sleeping, joint graph |
| `dynamics/solver/xpbd/` | `XpbdConstraint` trait, `XpbdConstraintSolverData`, `prepare_xpbd_joint`, `solve_xpbd_joint` |
| `dynamics/solver/islands/` | Sleeping islands: `IslandPlugin`, `IslandSleepingPlugin`, `WakeIslands`, `SleepIslands` |
| `dynamics/solver/joint_graph/` | `JointGraph`, `JointGraphPlugin` |
| `dynamics/ccd/` | `SpeculativeMargin`, `SweptCcd`, `SweepMode` |
| `dynamics/integrator/` | `Gravity`, `IntegratorPlugin`, integration systems |
| `spatial_query/` | `SpatialQuery` system param, `RayCaster`, `ShapeCaster`, filters |
| `schedule/` | `PhysicsSchedule`, `PhysicsSystems`, `PhysicsStepSystems`, `SubstepCount`, `PhysicsTime` |
| `interpolation.rs` | `PhysicsInterpolationPlugin`, transform interpolation/extrapolation |
| `physics_transform/` | `Position`, `Rotation`, `PhysicsTransformPlugin` |
| `debug_render/` | `PhysicsDebugPlugin`, gizmo configuration |
| `diagnostics/` | `PhysicsDiagnosticsPlugin`, `PhysicsDiagnosticsUiPlugin` |
| `picking/` | `PhysicsPickingPlugin` (requires `bevy_picking` feature) |
| `math/` | Re-exported type aliases: `Vector`, `Scalar`, `Dir`, `Rot`, `AngularVector`, etc. |

---

## 4. Core Concepts

### 4.1 Plugin Setup

```rust
use avian3d::prelude::*;

// Standard setup — runs physics on FixedPostUpdate
App::new()
    .add_plugins((DefaultPlugins, PhysicsPlugins::default()))
    .run();

// Headless/server — runs on FixedUpdate instead
App::new()
    .add_plugins(MinimalPlugins)
    .add_plugins(PhysicsPlugins::new(FixedUpdate).build())
    .run();

// With collision hooks (set at plugin construction time — cannot change at runtime)
App::new()
    .add_plugins(PhysicsPlugins::default().with_collision_hooks::<MyHooks>())
    .run();

// Custom length unit (pixels-per-meter for 2D, or world scaling)
App::new()
    .add_plugins(PhysicsPlugins::default().with_length_unit(100.0))
    .run();
```

`PhysicsPlugins::default()` adds (from `lib.rs:800-830`):
- `PhysicsSchedulePlugin` — schedules, system sets, time resources
- `MassPropertyPlugin` — auto-compute mass properties from colliders
- `ForcePlugin` — external forces, torques, acceleration
- `ColliderHierarchyPlugin` — `ColliderOf` relationships
- `ColliderTransformPlugin` — propagate collider transforms
- `ColliderCachePlugin` — (with `collider-from-mesh`+`default-collider`) cache mesh colliders
- `ColliderBackendPlugin<Collider>` — (with `default-collider`) initialize colliders and AABBs
- `NarrowPhasePlugin<Collider>` — (with `default-collider`) contact management
- `SolverPlugins` — solver plugin group
- `BroadPhasePlugin<()>` — AABB pair finding
- `JointPlugin` — joint initialization (not the solver)
- `SpatialQueryPlugin` — ray casting, shape casting
- `PhysicsTransformPlugin` — sync `Position`/`Rotation` ↔ `Transform`
- `PhysicsInterpolationPlugin` — transform interpolation/extrapolation

Optional additional plugins (must be added separately):
- `PhysicsDebugPlugin` (requires `debug-plugin` feature — on by default)
- `PhysicsDiagnosticsPlugin` (requires `bevy_diagnostic` feature)
- `PhysicsDiagnosticsUiPlugin` (requires `diagnostic_ui` feature)
- `PhysicsPickingPlugin` (requires `bevy_picking` feature)

### 4.2 Schedule Architecture

```
Main schedule (default: FixedPostUpdate)
├── PhysicsSystems::First           (user init before physics)
├── PhysicsSystems::Prepare         (mass props, transforms)
├── PhysicsSystems::StepSimulation  ⟶ runs PhysicsSchedule
│   └── PhysicsSchedule
│       ├── PhysicsStepSystems::First
│       ├── BroadPhase              ⟶ AABB overlap pair finding
│       ├── NarrowPhase             ⟶ contact computation, collision events
│       ├── Solver                  ⟶ runs SolverSystems
│       │   ├── PrepareSolverBodies
│       │   ├── PrepareJoints
│       │   ├── PrepareContactConstraints
│       │   ├── PreSubstep          (user: prepare custom joints)
│       │   ├── Substep             ⟶ runs SubstepSchedule × SubstepCount (default: 6)
│       │   │   └── SubstepSchedule
│       │   │       ├── IntegrationSystems::Velocity
│       │   │       ├── WarmStart
│       │   │       ├── SolveConstraints (incl. XpbdSolverSystems::SolveUserConstraints)
│       │   │       ├── IntegrationSystems::Position
│       │   │       ├── Relax
│       │   │       └── Damping
│       │   ├── PostSubstep
│       │   ├── Restitution
│       │   ├── Finalize            (write back solver body data)
│       │   └── StoreContactImpulses
│       ├── Sleeping                ⟶ body deactivation
│       ├── SpatialQuery            ⟶ update RayCaster/ShapeCaster hits
│       ├── Finalize
│       └── Last
├── PhysicsSystems::Writeback       (Transform ← Position/Rotation sync)
└── PhysicsSystems::Last            (user cleanup after physics)
```

Key types:
- `PhysicsSchedule` — the internal schedule that runs the physics step
- `SubstepSchedule` — runs `SubstepCount` (default 6) times per physics step
- `SubstepCount` resource — `app.insert_resource(SubstepCount(12))`
- `Time<Physics>` — physics clock, `Time<Substeps>` — substep clock
- `LastPhysicsTick` — `Tick` from the end of the previous physics run

### 4.3 Rigid Body Types

```rust
RigidBody::Dynamic    // Full simulation, affected by forces and collisions
RigidBody::Static     // Infinite mass, cannot move. Ground, walls.
RigidBody::Kinematic  // Programmatic movement only, pushes dynamic bodies
```

`RigidBody::Dynamic` is the default. Bodies auto-require:
- `Position`, `Rotation` (global physics transforms, separate from `Transform`)
- `LinearVelocity`, `AngularVelocity`
- `ComputedMass`, `ComputedAngularInertia`, `ComputedCenterOfMass`
- `AccumulatedLocalAcceleration`
- `PreSolveDeltaPosition`, `PreSolveDeltaRotation`

### 4.4 Core Components

```rust
// Velocity
LinearVelocity(Vector)        // (m/s)
AngularVelocity(Vector)       // (rad/s) — axis × speed in 3D

// Damping
LinearDamping(Scalar)         // Default: 0.0
AngularDamping(Scalar)        // Default: 0.0

// Speed limits
MaxLinearSpeed(Scalar)        // Default: INFINITY
MaxAngularSpeed(Scalar)       // Default: INFINITY

// Gravity
GravityScale(Scalar)          // Default: 1.0. 0.0 disables, 2.0 doubles

// Locked axes
LockedAxes::ROTATION_LOCKED   // Freeze rotation (common for characters)
LockedAxes::TRANSLATION_LOCKED // Freeze position

// Dominance (infinite mass in collisions with lower dominance)
Dominance(i8)                 // Range: -127..127, default: 0

// Disable body without despawning
RigidBodyDisabled             // Marker component — just insert/remove

// Disable collider without despawning
ColliderDisabled              // Marker component
```

### 4.5 Physics Materials

```rust
Friction::new(1.0)
    .with_combine_rule(CoefficientCombine::Min)
    // Other combine rules: Max, Average, Multiply, Default

Restitution::ZERO
    .with_combine_rule(CoefficientCombine::Min)
    // Default combine rule for both: Average

// Default friction for all new entities (resource)
DefaultFriction(Scalar)       // Default: 0.5
DefaultRestitution(Scalar)    // Default: 0.0
```

The `CoefficientCombine` enum controls how friction/restitution is computed when two bodies with different values collide:
- `Average` — `(a + b) / 2`
- `Min` — `min(a, b)`
- `Max` — `max(a, b)`
- `Multiply` — `a * b`
- `Default` — use the default combine rule (Average for both)

### 4.6 Collision Layers

```rust
CollisionLayers::default()
// Bitmask: memberships and filter
CollisionLayers::new(memberships: LayerMask, filter: LayerMask)
// Two bodies collide if: memberships1 & filter2 != 0 AND memberships2 & filter1 != 0

LayerMask::from_bits(0b0011)  // Create from raw bits
LayerMask::layer(0)           // Layer 0 (bit 0)
LayerMask::layer(1)           // Layer 1 (bit 1)
// etc.

PhysicsLayer                  // Sealed trait for custom layer enums
```

### 4.7 Sleeping

```rust
// Insert Sleeping component to deactivate a body (auto-managed by default)
// Sleeping bodies: zero velocity, no collision processing, "frozen"
// Re-awakened when a non-sleeping body interacts with them

// Configuration components:
SleepingThreshold { linear: Scalar, angular: Scalar }
DeactivationTime(Duration)     // How long before a body falls asleep
SleepTimer(Duration)           // Current timer (elapsed time since velocity < threshold)
TimeSleeping(Duration)         // How long the body has been asleep
TimeToSleep(Duration)          // Remaining time before sleeping

// Disable sleeping for a specific body
SleepingDisabled

// Programmatic wake/sleep
SleepBody(marker component)    // Force body to sleep
WakeBody(marker component)     // Force body to wake
WakeUpBody(marker component)   // Also wakes neighbors
WakeIslands(Event)             // Event to wake all islands
SleepIslands(Event)            // Event to sleep all islands
```

Sleeping works via **body islands** (groups of interconnected bodies). When an entire island is at rest below the threshold for `DeactivationTime`, all bodies in the island sleep.

### 4.8 Gravity

```rust
// Default: (0.0, -9.81, 0.0) in 3D
app.insert_resource(Gravity(Vector::NEG_Y * 9.81));
app.insert_resource(Gravity::ZERO);  // Disable globally

// Per-body control via GravityScale
commands.spawn((RigidBody::Dynamic, GravityScale(2.0)));  // Double gravity
commands.spawn((RigidBody::Dynamic, GravityScale(0.0)));  // No gravity
```

---

## 5. Collider Shapes and Configuration

### 5.1 Primitive Colliders

All from `Collider` enum:

```rust
// Primitive — half-extents based!
Collider::cuboid(1.0, 1.0, 1.0)           // Half-extents = 1.0 → full size 2.0
Collider::sphere(0.5)
Collider::capsule(0.4, 1.0)               // radius, half_height
Collider::cylinder(0.5, 1.0)              // radius, half_height
Collider::cone(0.5, 1.0)                  // radius, half_height (exact naming not verified)
Collider::halfspace(Vector::Y)             // Infinite half-plane

// From Bevy shapes (requires bevy_mesh for some)
Collider::from(Cuboid::default())
Collider::from(Capsule3d::default())
Collider::from(Sphere::default())
Collider::from(Cylinder::default())
Collider::from(Cone::default())

// Trimesh (from Mesh)
Collider::trimesh_from_mesh(&mesh).unwrap()

// Convex hull (from Mesh)
Collider::convex_hull_from_mesh(&mesh).unwrap()

// Voxel collider from point cloud
Collider::voxels_from_points(voxel_size: Scalar, &[Vector])
```

**Critical**: `Collider::cuboid` takes **half-extents**, not full extents. A 2×2×2 cube uses `Collider::cuboid(1.0, 1.0, 1.0)`. `Collider::capsule(0.4, 1.0)` has total height `2 * 1.0 = 2.0`.

### 5.2 Collider Construction from Meshes (Scene Loading)

```rust
// Direct component constructors:
ColliderConstructor::TrimeshFromMesh                    // Just the triangles
ColliderConstructor::ConvexHullFromMesh                  // Single convex hull
ColliderConstructor::ConvexDecompositionFromMesh         // Multiple convex hulls
ColliderConstructor::VoxelizedTrimeshFromMesh

// For glTF scenes with hierarchy:
ColliderConstructorHierarchy::new(ColliderConstructor::ConvexDecompositionFromMesh)
    .with_density_for_name("part_name", 3.0)

// Used with Scene entities: add ColliderConstructorHierarchy component
// to auto-generate colliders when a scene finishes loading
```

### 5.3 Collider Hierarchy and Transforms

```rust
// Child colliders automatically follow parent rigid body:
commands.spawn((RigidBody::Dynamic, Transform::default()))
    .with_child((Collider::sphere(0.5), Transform::from_xyz(1.0, 0.0, 0.0)))
    .with_child((Collider::sphere(0.5), Transform::from_xyz(-1.0, 0.0, 0.0)));

// ColliderOf component connects colliders to their rigid body parent
ColliderOf { body: Entity }

// RigidBodyColliders — maps body entity to its collider children
```

### 5.4 Collision Margin

```rust
CollisionMargin(0.01)  // Small gap around colliders to prevent edge sticking
```

The collision margin helps prevent objects from catching on internal edges when sliding across surfaces. Used in the `conveyor_belt` example.

### 5.5 Sensors

```rust
Sensor  // Marker component — trigger-style, no collision response
```

Sensor colliders detect overlaps without generating contact constraints or pushing bodies apart. Used for trigger volumes, pressure plates, etc.

---

## 6. Mass Properties

From `src/dynamics/rigid_body/mass_properties/`:

```rust
// Auto-computed from colliders with density
ColliderDensity(2.0)                         // Default: 1.0

// Override manually
Mass(5.0)
CenterOfMass::new(0.0, -0.5, 0.0)
AngularInertia(Mat3::IDENTITY * 2.0)

// Computed results (read-only, set by the engine):
ComputedMass(Scalar)
ComputedAngularInertia(Mat3)
ComputedCenterOfMass(Vector)

// Bundle for convenience:
MassPropertiesBundle::from_shape(&Cuboid::from_length(1.0), 1.0)

// Prevent child colliders from contributing:
NoAutoMass
NoAutoAngularInertia
NoAutoCenterOfMass
```

Mass properties are computed by `MassPropertyPlugin` in `PhysicsSystems::Prepare`. Child collider masses are combined by default unless `NoAuto*` components are used.

---

## 7. External Forces, Impulses, and Acceleration

From `src/dynamics/rigid_body/forces/`:

```rust
// Persistent external force/torque components:
ConstantForce(Vector)
ConstantTorque(AngularVector)            // World-space
ConstantLocalTorque(AngularVector)        // Local-space (3D only)
ConstantAcceleration(Vector)             // World-space linear acceleration
ConstantLocalAcceleration(Vector)        // Local-space linear acceleration
ConstantAngularAcceleration(Vector)      // Angular acceleration (3D only)

// Lower-level:
// Forces and torques from the previous frame
Forces { value: Vector, torque: AngularVector }

// For component-based force accumulation
RigidBodyForces(Forces)

// Use with entities:
commands.spawn((
    RigidBody::Dynamic,
    ConstantForce(Vector::X * 10.0),
    ConstantTorque(Vector::Z * 5.0),
));
```

---

## 8. Continuous Collision Detection (CCD)

From `src/dynamics/ccd/`:

```rust
// Speculative collision (default, lightweight)
// Enabled globally via NarrowPhaseConfig
SpeculativeMargin(Scalar)  // Per-body margin for speculative contacts

// Swept CCD per rigid body
SweptCcd
SweepMode::LinearOnly      // Only linear sweep
SweepMode::Full            // Linear + angular sweep (more expensive)
```

Speculative collision is the default CCD mode. It predicts collisions by extending colliders slightly in their movement direction. For fast-moving objects, `SweptCcd` provides more accurate (but more expensive) collision detection.

---

## 9. Joints and Constraints

### 9.1 Joint Pattern

Joints are **separate entities** that reference two body entities via `body1` and `body2` fields:

```rust
let body1 = commands.spawn(RigidBody::Dynamic).id();
let body2 = commands.spawn(RigidBody::Dynamic).id();
commands.spawn(FixedJoint::new(body1, body2));
```

### 9.2 Joint Reference Frames

Each joint uses a `JointFrame` per body, consisting of a `JointAnchor` (translation) and `JointBasis` (rotation):

```rust
JointFrame {
    anchor: JointAnchor,  // Local(offset) or FromGlobal(world_pos)
    basis: JointBasis,    // Local(rotation) or FromGlobal(world_rot)
}
```

- `JointAnchor::Local(Vector)` — local-space offset from body center
- `JointAnchor::FromGlobal(Vector)` — world-space anchor (auto-converted to Local next step)
- `JointBasis::Local(Rot)` — local rotation
- `JointBasis::FromGlobal(Rot)` — world-space basis (auto-converted)
- `JointFrame::IDENTITY` — zero anchor, identity rotation

### 9.3 Available Joints

| Joint | Allowed 3D DOF | Features | Example |
|---|---|---|---|
| `SphericalJoint` | 3 rotations (ball-and-socket) | Swing/twist limits, twist axis | `chain_3d.rs` |
| `FixedJoint` | None (rigid lock) | Point+angle compliance | `fixed_joint_3d.rs` |
| `DistanceJoint` | 2 translations + 3 rotations | Min/max distance limits, compliance (spring) | `distance_joint_3d.rs` |
| `PrismaticJoint` | 1 translation (slider) | Slider axis, distance limits | `prismatic_joint_3d.rs` |
| `RevoluteJoint` | 1 rotation (hinge) | Hinge axis, angle limits | `revolute_joint_3d.rs` |

### 9.4 SphericalJoint — Ball and Socket (3D only)

```rust
SphericalJoint::new(body1, body2)
    .with_local_anchor1(Vector::NEG_Y * 0.5)
    .with_local_anchor2(Vector::Y * 0.5)
    .with_twist_axis(Vector::Y)                     // Default: Y
    .with_swing_limits(-0.5, 0.5)                   // Radians, cone half-angle
    .with_twist_limits(-0.3, 0.3)                   // Radians
    .with_point_compliance(0.0)                     // m/N
    .with_swing_compliance(0.0)                     // N*m/rad
    .with_twist_compliance(0.0)                     // N*m/rad

// Example: chain_3d.rs — 100-particle chain with spherical joints
commands.spawn(
    SphericalJoint::new(previous, current)
        .with_local_anchor1(Vector::NEG_Y * radius * 1.1)
        .with_local_anchor2(Vector::Y * radius * 1.1)
        .with_point_compliance(0.00001),
);
```

### 9.5 FixedJoint — Rigid Connection

```rust
FixedJoint::new(anchor, object)
    .with_local_anchor1(Vector::X * 1.5)
    .with_local_anchor2(Vector::ZERO)
    .with_point_compliance(0.0)                     // m/N
    .with_angle_compliance(0.0)                     // N*m/rad

// Example: fixed_joint_3d.rs
// Kinematic rotating anchor + dynamic object connected with FixedJoint
// Uses SubstepCount(50) for stability
```

### 9.6 DistanceJoint — Spring with Limits

```rust
DistanceJoint::new(body1, body2)
    .with_local_anchor1(Vector::ZERO)
    .with_local_anchor2(Vector::splat(0.5))
    .with_limits(1.5, 1.5)                         // min, max distance
    .with_compliance(1.0 / 400.0)                  // m/N (0 = rigid)

// Example: distance_joint_3d.rs
// Static cube + dynamic cube, spring-like connection
// Uses PhysicsDebugPlugin for visualization
```

Compliance is the **inverse of stiffness** (`compliance = 1/stiffness`). Zero = rigid. Higher = softer spring.

### 9.7 PrismaticJoint — Sliding Axis

```rust
PrismaticJoint::new(anchor, object)
    .with_slider_axis(Vector::X)                   // Default: X
    .with_local_anchor1(Vector::X)
    .with_limits(0.5, 2.0)                         // Translation limits
    .with_align_compliance(0.0)                    // m/N
    .with_angle_compliance(0.0)                    // N*m/rad
    .with_limit_compliance(0.0)                    // m/N

// Example: prismatic_joint_3d.rs
// Kinematic rotating anchor + dynamic object, slider axis along X
// Uses SubstepCount(50) for stability
```

### 9.8 RevoluteJoint — Hinge

```rust
RevoluteJoint::new(anchor, object)
    .with_hinge_axis(Vector::Z)                    // Default: Z (3D)
    .with_local_anchor2(Vector::Y * 2.0)
    .with_angle_limits(-1.0, 1.0)                  // Radians
    .with_point_compliance(0.0)                    // m/N
    .with_align_compliance(0.0)                    // N*m/rad (3D only)
    .with_limit_compliance(0.0)                    // N*m/rad

// Example: revolute_joint_3d.rs
// Kinematic rotating anchor + dynamic object, hinge with angle limits
// Uses SubstepCount(50) for stability
```

### 9.9 Joint Accessories

```rust
// Damping on relative motion
JointDamping { linear: 0.1, angular: 0.1 }

// Read joint forces (for breakable joints)
JointForces::new()          // Adds components to the joint entity
// Query: for force in query.iter() { force.force(), force.torque() }

// Disable collision between joined bodies
JointCollisionDisabled

// Temporarily disable a joint
JointDisabled

// Required: MapEntities — for entity remapping during Scene spawning
// All joints implement MapEntities
```

### 9.10 Breakable Joint Pattern

From the joint docs at `src/dynamics/joints/mod.rs`:

```rust
fn break_joints(
    mut commands: Commands,
    query: Query<(Entity, &JointForces), Without<JointDisabled>>,
) {
    for (entity, joint_forces) in &query {
        if joint_forces.force().length() > BREAK_THRESHOLD {
            commands.entity(entity).insert(JointDisabled);
        }
    }
}
```

### 9.11 Custom XPBD Constraint Pattern

From `custom_constraint.rs`:

```rust
// 1. Define constraint component with #[require(SolverData)]
#[derive(Component)]
#[require(CenterDistanceConstraintSolverData)]
struct CenterDistanceConstraint {
    entity1: Entity, entity2: Entity,
    rest_distance: Scalar,
    compliance: Scalar,
}
// 2. Define solver data
#[derive(Component, Default)]
struct CenterDistanceConstraintSolverData { center_difference: Vector }
impl XpbdConstraintSolverData for CenterDistanceConstraintSolverData {}

// 3. Implement EntityConstraint<2> and XpbdConstraint<2>
impl EntityConstraint<2> for CenterDistanceConstraint { ... }
impl XpbdConstraint<2> for CenterDistanceConstraint {
    type SolverData = CenterDistanceConstraintSolverData;
    fn prepare(&mut self, bodies, solver_data) { ... }
    fn solve(&mut self, bodies, inertias, solver_data, dt) { ... }
}
impl PositionConstraint for CenterDistanceConstraint {}
impl AngularConstraint for CenterDistanceConstraint {}
impl MapEntities for CenterDistanceConstraint { ... }

// 4. Register in app:
app.add_plugins(JointGraphPlugin::<CenterDistanceConstraint>::default());
// Add prepare system to PhysicsSchedule → SolverSystems::PreSubstep
// Add solve system to SubstepSchedule → XpbdSolverSystems::SolveUserConstraints
```

---

## 10. Collision Detection

### 10.1 Collision Events

```rust
// Enable events per entity:
commands.spawn((Collider::cuboid(1.0, 1.0, 1.0), CollisionEventsEnabled));

// As Messages (efficient for bulk processing):
fn my_system(mut start_reader: MessageReader<CollisionStart>) {
    for event in start_reader.read() {
        // event.collider1, event.collider2 — Entity
        // event.body1, event.body2 — Option<Entity>
    }
}

// As observers (entity-specific):
commands.spawn((Collider::cuboid(1.0, 1.0, 1.0), Sensor, CollisionEventsEnabled))
    .observe(on_player_stepped_on_plate);

fn on_player_stepped_on_plate(event: On<CollisionStart>, ...) {
    // event.collider1 = the entity with the observer
    // event.collider2 = the other entity
}
```

Event structs:
```rust
CollisionStart {
    pub collider1: Entity,   // #[event_target] — for observers
    pub collider2: Entity,
    pub body1: Option<Entity>,
    pub body2: Option<Entity>,
}
CollisionEnd { /* same fields */ }
```

Scheduling: Events are emitted **after** the physics step in `CollisionEventSystems` system set (in the narrow phase). At that point, the solver has already run and contact impulses are updated.

### 10.2 Accessing Contact Data

```rust
// Collisions system param — only TOUCHING contacts:
fn collision_system(collisions: Collisions) {
    for contacts in collisions.iter() {  // Iterator<Item = ContactPair>
        for manifold in &contacts.manifolds {
            for point in &manifold.points {
                // point.penetration, point.normal_impulse
                // point.anchor1, point.anchor2 (world-space, relative to COM)
                // point.point (world-space midpoint)
            }
        }
        // Compute total impulse:
        let total_impulse = contacts.total_normal_impulse();
        let max_impulse = contacts.max_normal_impulse();
    }
}

// ContactGraph resource — all contacts (including non-touching):
fn graph_system(contact_graph: Res<ContactGraph>) {
    for edge in contact_graph.iter_contacts() {
        // edge.collider1, edge.collider2
        // edge.is_touching(), edge.is_sleeping()
    }
}
```

### 10.3 ContactPair API

From `src/collision/contact_types/mod.rs`:

```rust
// On ContactPair:
contacts.collision_started()           // Started touching this frame
contacts.collision_ended()             // Stopped touching this frame
contacts.is_touching()                 // Currently touching
contacts.aabbs_disjoint()              // AABBs no longer overlap
contacts.generates_constraints()       // Has contact constraints (not sensor)
contacts.find_deepest_contact()        // Option<&ContactPoint>
contacts.total_normal_impulse()        // Vector sum of all normal impulses
contacts.total_normal_impulse_magnitude() // Scalar sum
contacts.max_normal_impulse()          // Largest impulse as Vector
contacts.max_normal_impulse_magnitude()  // Largest impulse magnitude
```

### 10.4 Collision Hooks

```rust
// Define hooks (SystemParam + CollisionHooks trait):
#[derive(SystemParam)]
struct MyHooks<'w, 's> {
    conveyor_query: Query<'w, 's, (Read<ConveyorBelt>, Read<GlobalTransform>)>,
}

impl CollisionHooks for MyHooks<'_, '_> {
    fn filter_pairs(&self, collider1: Entity, collider2: Entity, commands: &mut Commands) -> bool {
        // Return false to reject contact pair (called in broad phase)
        true
    }

    fn modify_contacts(&self, contacts: &mut ContactPair, commands: &mut Commands) -> bool {
        // Return false to remove contact pair (called in narrow phase)
        // Return true to accept
        // Can set manifold.tangent_velocity for conveyor belts
        true
    }
}

// Activate hooks per entity:
ActiveCollisionHooks::FILTER_PAIRS
ActiveCollisionHooks::MODIFY_CONTACTS
ActiveCollisionHooks::all()                           // Both

// Example: conveyor_belt.rs sets manifold.tangent_velocity
for manifold in contacts.manifolds.iter_mut() {
    manifold.tangent_velocity = conveyor_speed * direction;
}
```

**Important**: Collision hooks are set at plugin construction time via `PhysicsPlugins::default().with_collision_hooks::<MyHooks>()`. Only one set of hooks per app.

### 10.5 CollidingEntities

```rust
// Component auto-populated with entities currently touching this collider
CollidingEntities(Vec<Entity>)
```

---

## 11. Spatial Queries

### 11.1 Ray Casting

Two approaches:

**Component-based** (every frame):
```rust
commands.spawn(RayCaster::new(Vec3::ZERO, Dir3::X)
    .with_max_distance(100.0)
    .with_max_hits(10)
    .with_solidness(true)
    .with_ignore_self(true)
    .with_query_filter(filter));

// Read hits from RayHits component
fn check(query: Query<(&RayCaster, &RayHits)>) {
    for (ray, hits) in &query {
        for hit in hits.iter_sorted() {  // Sorted by distance
            // hit.entity, hit.distance, hit.normal
        }
        // Or unsorted iteration:
        for hit in hits.iter() { ... }
    }
}
```

**SystemParam-based** (on-demand):
```rust
fn raycast(query: SpatialQuery) {
    let filter = SpatialQueryFilter::default();

    // Single closest hit:
    if let Some(hit) = query.cast_ray(origin, direction, max_dist, solid, &filter) {
        // hit.entity, hit.distance, hit.normal
    }

    // Multiple hits:
    let hits: Vec<RayHitData> = query.ray_hits(origin, direction, max_dist, 20, solid, &filter);

    // Callback variant:
    query.ray_hits_callback(origin, direction, max_dist, solid, &filter, |hit| {
        println!("Hit: {:?}", hit);
        true  // Return false to stop iteration
    });

    // Predicate variant (per-entity filter):
    query.cast_ray_predicate(origin, direction, max_dist, solid, &filter, &|entity| {
        !invisible.contains(entity)
    });
}
```

### 11.2 Shape Casting (Sweep Testing)

**Component-based**:
```rust
commands.spawn(ShapeCaster::new(
    Collider::sphere(0.5),    // Shape
    Vec3::ZERO,               // Origin
    Quat::default(),          // Shape rotation
    Dir3::NEG_Y               // Direction
)
    .with_max_distance(0.2)
    .with_max_hits(10)
    .with_solidness(true)
    .with_ignore_self(true));

fn check(query: Query<(&ShapeCaster, &ShapeHits)>) {
    for (caster, hits) in &query {
        for hit in hits.iter() {
            // hit.entity, hit.distance, hit.normal1, hit.normal2
            // hit.point1, hit.point2
        }
    }
}
```

**SystemParam-based**:
```rust
fn shape_cast(query: SpatialQuery) {
    let config = ShapeCastConfig::from_max_distance(100.0);
    let filter = SpatialQueryFilter::default();

    // Single closest hit:
    if let Some(hit) = query.cast_shape(&shape, origin, rotation, direction, &config, &filter) {
        // hit.entity, hit.normal1, hit.normal2, hit.distance, hit.point1, hit.point2
    }

    // Multiple hits:
    query.shape_hits(&shape, origin, rotation, direction, 20, &config, &filter);

    // Predicate variant:
    query.cast_shape_predicate(&shape, origin, rotation, direction, &config, &filter, &|e| {
        !invisible.contains(e)
    });
}
```

### 11.3 Point Projection

```rust
fn project(query: SpatialQuery) {
    if let Some(proj) = query.project_point(point, solid, &filter) {
        // proj.point — the projected point
        // proj.is_inside — whether the point is inside the collider
        // proj.entity — the collider entity
    }

    // Predicate variant:
    query.project_point_predicate(point, solid, filter, &|entity| { ... });
}
```

### 11.4 Intersection Tests

```rust
fn intersections(query: SpatialQuery) {
    // Point intersection (find colliders containing a point):
    let entities: Vec<Entity> = query.point_intersections(point, &filter);
    query.point_intersections_callback(point, &filter, |entity| { true });

    // AABB intersection (find colliders with overlapping AABBs):
    let aabb = Collider::sphere(0.5).aabb(Vec3::ZERO, Quat::default());
    let entities: Vec<Entity> = query.aabb_intersections_with_aabb(aabb);
    query.aabb_intersections_with_aabb_callback(aabb, |entity| { true });

    // Shape intersection (find colliders intersecting a shape):
    let entities: Vec<Entity> = query.shape_intersections(&shape, position, rotation, &filter);
    query.shape_intersections_callback(&shape, position, rotation, &filter, |entity| { true });
}
```

### 11.5 Query Filters

```rust
SpatialQueryFilter::default()                                 // All colliders
SpatialQueryFilter::from_mask(LayerMask::layer(0))            // Only layer 0
SpatialQueryFilter::from_excluded_entities([entity1])         // Exclude specific entities

// Builder methods:
SpatialQueryFilter::default()
    .with_mask(LayerMask::from_bits(0b0011))
    .with_excluded_entities([entity1, entity2])
```

The filter's `test()` method checks: the entity is not excluded AND its collision layers intersect the filter mask.

### 11.6 Shape Cast Configuration

```rust
ShapeCastConfig {
    max_distance: Scalar,
    // Other fields...
}
ShapeCastConfig::from_max_distance(100.0)
```

---

## 12. Character Controllers

Avian does **not** have a built-in character controller. These are example-level patterns. Two approaches:

### 12.1 Dynamic Character Controller

Uses `RigidBody::Dynamic` with `LockedAxes::ROTATION_LOCKED`.

**Setup** (from `dynamic_character_3d/plugin.rs`):
```rust
CharacterControllerBundle::new(collider)
    .with_movement(acceleration, damping, jump_impulse, max_slope_angle)
```

The bundle contains:
- `RigidBody::Dynamic`
- `LockedAxes::ROTATION_LOCKED`
- `ShapeCaster` (slightly smaller copy of collider, pointing `Dir3::NEG_Y`, max_distance 0.2)
- `MovementBundle { acceleration, damping, jump_impulse, max_slope_angle }`

**Ground detection**: `ShapeCaster` with a 0.99x scaled capsule, checking `ShapeHits` with slope angle filter:
```rust
let is_grounded = hits.iter().any(|hit| {
    if let Some(angle) = max_slope_angle {
        (rotation * -hit.normal2).angle_between(Vector::Y).abs() <= *angle
    } else {
        true
    }
});
```

**Movement** (in `Update` schedule):
```rust
linear_velocity.x += direction.x * acceleration * delta_time;
linear_velocity.z -= direction.y * acceleration * delta_time;
```

**Jumping**: `linear_velocity.y = jump_impulse` (only when `Grounded`).

**Damping** (manual XZ damping to preserve Y velocity):
```rust
linear_velocity.x *= damping_factor;
linear_velocity.z *= damping_factor;
```

**Input**: Uses `Message<MovementAction>` events (`Move(Vector2)`, `Jump`) written by keyboard/gamepad systems.

### 12.2 Kinematic Character Controller

Uses `RigidBody::Kinematic`. Manual collision response.

**Setup** (from `kinematic_character_3d/plugin.rs`):
```rust
CharacterControllerBundle::new(collider, gravity_vector)
```

Key differences from dynamic:
- Has `ControllerGravity(Vector)` — applied manually in `Update`
- Collision response runs in `PhysicsSchedule` at `NarrowPhaseSystems::Last`
- Manual penetration resolution

**Collision response pattern**:
```rust
fn kinematic_controller_collisions(
    collisions: Collisions,
    collider_rbs: Query<&ColliderOf, Without<Sensor>>,
    mut controllers: Query<(&mut Position, &mut LinearVelocity, Option<&MaxSlopeAngle>),
        (With<RigidBody>, With<CharacterController>)>,
    time: Res<Time>,
) {
    for contacts in collisions.iter() {
        for manifold in contacts.manifolds.iter() {
            for contact in manifold.points.iter() {
                if contact.penetration > 0.0 {
                    position.0 += normal * contact.penetration;
                }
            }
            // Slope handling: snap velocity for climbable, reject for walls
            // Speculative contact handling for predictive collision avoidance
        }
    }
}
```

**Warning from the example**: The collision logic is basic and buggy. Recommends "collide-and-slide" or `bevy_tnua` for production.

### 12.3 Recommendations

- **Dynamic** is simpler and more robust for most cases (Avian handles collision response)
- **Kinematic** gives more control but requires manual collision resolution
- Both use `ShapeCaster` for ground detection
- Neither is built-in — both are example-level patterns
- Third-party: `bevy_tnua` supports Avian

---

## 13. Transform Handling

### 13.1 Position and Rotation Components

`Position(Vector)` and `Rotation(Quat/Rot2)` are the physics-internal transforms. They are **global** (no hierarchy), which avoids issues with scale/shearing.

- Use `Position`/`Rotation` when working inside `PhysicsSchedule` or `PhysicsSystems::StepSimulation`
- Use `Transform` in most other contexts (auto-synced by `PhysicsTransformPlugin`)
- `Transform` ↔ `Position`/`Rotation` sync happens in `PhysicsSystems::Writeback`

### 13.2 Transform Interpolation/Extrapolation

Built-in via `PhysicsInterpolationPlugin` (included in `PhysicsPlugins` by default).

```rust
// Per-entity:
commands.spawn((RigidBody::Dynamic, Transform::default(), TransformInterpolation));
commands.spawn((RigidBody::Dynamic, Transform::default(), TransformExtrapolation));

// Per-component:
commands.spawn((Transform::default(), TranslationInterpolation));
commands.spawn((Transform::default(), RotationInterpolation));

// All bodies by default:
app.add_plugins(PhysicsPlugins::default().set(PhysicsInterpolationPlugin::interpolate_all()));
app.add_plugins(PhysicsPlugins::default().set(PhysicsInterpolationPlugin::extrapolate_all()));

// Opt out:
commands.spawn((Transform::default(), NoTransformEasing));

// Hermite interpolation (smoother, uses velocity):
commands.spawn((Transform::default(), TransformInterpolation, TransformHermiteEasing));
```

---

## 14. Debug Rendering

Requires `debug-plugin` feature (enabled by default).

```rust
app.add_plugins(PhysicsDebugPlugin);  // Must be added separately

// Configure via PhysicsGizmos resource:
app.insert_resource(PhysicsGizmos {
    aabb_color: Some(Color::WHITE),
    contact_color: Some(Color::RED),
    joint_anchor_color: Some(Color::GREEN),
    joint_separation_color: Some(Color::YELLOW),
    ..default()
});
```

Debug rendering draws:
- Collider AABBs
- Contact points and normals
- Joint anchors and separations
- Sleeping islands (color coded)

---

## 15. Determinism and Serialization

### 15.1 enhanced-determinism Feature

```toml
avian3d = { version = "0.6.1", default-features = false, features = ["3d", "f32", "parry-f32", "enhanced-determinism"] }
```

**Critical: the `parallel` feature (enabled by default) must be disabled for determinism.** Multi-threaded
execution produces non-deterministic scheduling order, which breaks bit-identical replay even with
`enhanced-determinism`. The deterministic config uses `default-features = false` and explicitly
selects only the features needed:

```toml
avian3d = { version = "0.6.1", default-features = false, features = ["3d", "f32", "parry-f32", "enhanced-determinism", "xpbd_joints", "serialize"] }
```

This disables `parallel`, `debug-plugin`, `collider-from-mesh`, `bevy_scene`, and `bevy_picking` —
none of which are needed on a headless deterministic server.

What `enhanced-determinism` enables (from `Cargo.toml`):
- `dep:libm` — cross-platform math (sin, cos, sqrt, etc.)
- `bevy_math/libm` — deterministic math in Bevy
- `bevy_heavy/libm`
- `parry3d?/enhanced-determinism` — deterministic collision detection in Parry
- `parry3d-f64?/enhanced-determinism`

**Without `enhanced-determinism`**, same-machine determinism works (same CPU, same OS) because
Avian's scalar math is deterministic within the same platform. Cross-platform determinism
(different CPU architectures, different OS) requires `enhanced-determinism` because standard
math libraries (`sin`, `cos`, `sqrt`) can give slightly different results on different platforms.

Verified by the test at `src/tests/mod.rs`:
```rust
// cubes_simulation_is_locally_deterministic — confirmed:
// 4x4x4 cube stack, 5 seconds at 60fps, run 4 times, all identical
```

Our benchmark (`prototype-physics-bench`) also confirms same-machine determinism for 10k-100k body
simulations at 500 steps without `enhanced-determinism`.

### 15.2 serialize Feature

```toml
avian3d = { version = "0.6.1", features = ["serialize"] }
```

What it enables:
- `dep:serde` derives on: `RigidBody`, `Collider`, `Position`, `Rotation`, `LinearVelocity`, `AngularVelocity`, `Friction`, `Restitution`, `GravityScale`, `CollisionLayers`, all joints, `JointFrame`, `JointAnchor`, `JointBasis`, `CollisionMargin`, `ColliderDensity`, `Mass`, `CenterOfMass`, `AngularInertia`, `SleepingThreshold`, `SubstepCount`, `SpatialQueryFilter`, etc.
- `parry3d?/serde-serialize` — Parry shape serialization

### 15.3 Server/Headless Pattern

From `src/tests/mod.rs`:

```rust
app.add_plugins((
    MinimalPlugins,
    TransformPlugin,
    PhysicsPlugins::default(),  // Or PhysicsPlugins::new(FixedUpdate)
));

// Manual stepping:
let strategy = TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));
app.insert_resource(strategy);
app.update();
```

---

## 16. Test-Backed Usage Reference

From `src/tests/mod.rs` and deterministic tests:

| Test | What it verifies |
|---|---|
| `it_loads_plugin_without_errors` | Plugin group initializes without panics, 500 ticks |
| `body_with_velocity_moves` | `LinearVelocity(Vector::X)` moves body 1 unit/s for 500 steps |
| `cubes_simulation_is_locally_deterministic` | 4×4×4 cube stack produces identical results across runs |
| `no_ambiguity_errors` | Schedule has no ambiguous system ordering (custom schedule + ColliderHierarchyPlugin disabled) |
| `determinism_2d` | Determinism under `enhanced-determinism` feature (2D) |

---

## 17. Physics Picking

Requires `bevy_picking` feature (enabled by default):

```rust
app.add_plugins(PhysicsPickingPlugin);

// Components:
PhysicsPickable         // Marker to make entity pickable
PhysicsPickingFilter    // Filter for pickable entities
PhysicsPickingSettings  // Configuration
```

---

## 18. Diagnostics

```rust
// Requires bevy_diagnostic feature:
app.add_plugins(PhysicsDiagnosticsPlugin);

// Requires diagnostic_ui feature:
app.add_plugins(PhysicsDiagnosticsUiPlugin);

// Internal diagnostic types:
PhysicsDiagnostics              // Collision detection timers
SpatialQueryDiagnostics         // Spatial query timers
CollisionDiagnostics            // Broad/narrow phase timers
```

---

## 19. Gotchas & Footguns

### 19.1 Version Compatibility

1. **Bevy 0.18 only.** Both Avian 0.5.0 and 0.6.1 target Bevy 0.18.

2. **Joint motors and articulations are not supported.** Documentation explicitly states: "Joint motors and articulations are not supported yet, but they will be implemented in a future release."

3. **No built-in character controller.** The examples are patterns, not reusable plugins. Third-party: `bevy_tnua`.

4. **`Position` and `Rotation` are separate from `Transform`.** Auto-synced by `PhysicsTransformPlugin`. Do NOT directly modify `Transform` during `PhysicsSystems::StepSimulation` — use `Position`/`Rotation`.

5. **`Collider::cuboid` takes half-extents.** `Collider::capsule(radius, half_height)` — total height = `2 * half_height`.

6. **`SubstepCount` default is 6**, not 4. The `chain_3d.rs` example uses 80 for stability. Joint-heavy scenes often need more substeps.

7. **`CollisionHooks` are set at plugin construction time.** Cannot change at runtime. Only one set per app.

8. **Debug builds are 100×+ slower than release.** Use `opt-level = 1` in `[profile.dev]`.

9. **`Sensor` colliders do not generate contact constraints.** They detect overlaps but don't push bodies apart. They need `CollisionEventsEnabled` to emit events.

10. **`Position`/`Rotation` on joint entities are only used for auto-tracking the entity's transform.** The joint's actual constraint position is determined by `JointFrame` values.

11. **`Gravity` is a resource** (`Gravity(Vector)`), not a component. Use `GravityScale(Scalar)` per body.

12. **The `xpbd_joints` feature is required for joint support** (enabled by default in the standard feature set).

### 19.2 Legacy Avian 0.5 Notes (Historical)

These details applied to the now-retired `prototype-physics-lightyear`, which
pinned Avian 0.5 through upstream `lightyear_avian3d`. The engine and remaining
prototypes use Avian 0.6.1. Kept for historical reference only.

- 0.5.0 uses `parry3d 0.25`; 0.6.1 uses `parry3d 0.26`
- The custom constraint API (`XpbdConstraint`, `EntityConstraint`, `JointGraphPlugin`) is explicitly documented as unstable: "prone to large sweeping changes and breakage"
- `PhysicsPluginsWithHooks` is a separate type in 0.5.0 and is merged into `PhysicsPlugins` in 0.6.1.

### 19.3 Determinism Footguns

1. **`parallel` feature breaks determinism.** The `parallel` feature (enabled by default) enables
   `parry3d/parallel` and `bevy/multi_threaded`. Thread scheduling is non-deterministic, so the
   same simulation will produce different results run-to-run. For determinism, use
   `default-features = false` and explicitly opt in to only `["3d", "f32", "parry-f32",
   "enhanced-determinism"]`.

2. **`enhanced-determinism` alone is not enough.** You must also disable `parallel` (single-threaded
   execution via `bevy/multi_threaded` off, `thread_local` dependency removed). The
   `default-features = false` approach handles both.

3. **`enhanced-determinism` IS portable.** With `enhanced-determinism` + `parallel` disabled, the
   simulation produces bit-identical results across different platforms (different CPUs, different
   OS). This is the whole point of the feature — `libm` replaces platform-specific math functions
   with deterministic equivalents, and Parry's `enhanced-determinism` flag disables
   architecture-specific code paths.

4. **Without `enhanced-determinism`, same-machine only.** Our `prototype-physics-bench` confirms
   same-machine determinism for 10k-100k bodies at 500 steps without `enhanced-determinism`, but
   only because the same CPU+OS uses the same math library and scheduling order each run.

### 19.4 Performance Notes

- `parallel` feature helps large simulations (1000+ bodies)
- `simd` feature enables SIMD in Parry
- Broad phase can be replaced (`custom_broad_phase.rs` example)
- Large trimesh colliders are slow — prefer convex decomposition for complex meshes
- Joint-heavy scenes need higher `SubstepCount` (chain_3d uses 80)
- `PhysicsDiagnosticsPlugin` helps identify bottlenecks

### 19.5 Schedule Integration

- Physics runs on `FixedPostUpdate` by default (Bevy's fixed timestep)
- Use `PhysicsPlugins::new(FixedUpdate)` for networking/server patterns
- `PhysicsSchedule` is single-threaded (ExecutorKind::SingleThreaded)
- Add custom systems to `PhysicsSchedule` via `app.add_systems(PhysicsSchedule, ...)`
- Add to `SubstepSchedule` via `app.get_schedule_mut(SubstepSchedule).unwrap().add_systems(...)`
- `NarrowPhaseSystems::Last` is the correct set for post-collision processing
- `CollisionEventSystems` is where collision events are emitted

---

## 20. Integration with Afterglow

- Afterglow uses Avian 0.6.1 with f32 precision and workspace features `3d`, `f32`, `parry-f32`, `xpbd_joints`, and `parallel`
- The `serialize` feature is used in `prototype-physics-serialize` (Avian 0.6.1) for save/load research. The retired `prototype-physics-lightyear` (Avian 0.5 legacy) previously also used it for lag-compensation research.
- The `enhanced-determinism` feature is available but our benchmarks show same-machine determinism without it
- Character controller patterns should follow the dynamic approach (simpler, more robust)
- Joints are separate entities — Afterglow's snapshot system must handle joint→body reference mapping
- Physics runs on `FixedUpdate` in Afterglow for deterministic server stepping (per `prototype-physics-bench`)
- Engine integration source: `crates/afterglow-engine/src/physics.rs`

---

*Generated from exhaustive reading of avian3d 0.5.0 and 0.6.1 source code, Cargo.toml, tests, examples, and documentation strings. API surfaces cross-verified against the workspace-locked versions.*
