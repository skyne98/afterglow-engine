# Rapier3d in Bevy — Research Notes

## 1. How to Use It in Bevy 0.18

### Dependency
```toml
[dependencies]
bevy_rapier3d = "0.33"
```

### Basic Setup
```rust
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(RapierPhysicsPlugin::<NoUserData>::default())
        .add_plugins(RapierDebugRenderPlugin::default())
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    // Ground (fixed)
    commands.spawn((
        Collider::cuboid(100.0, 0.1, 100.0),
        Transform::from_xyz(0.0, -2.0, 0.0),
    ));
    // Dynamic ball
    commands.spawn((
        RigidBody::Dynamic,
        Collider::ball(0.5),
        Restitution::coefficient(0.7),
        Transform::from_xyz(0.0, 4.0, 0.0),
    ));
}
```

### Key Components
| Component | Purpose |
|---|---|
| `RigidBody::Dynamic/Fixed/KinematicPositionBased/KinematicVelocityBased` | Body type |
| `Collider::ball/cuboid/round_cuboid/cylinder/cone/capsule/triangle_mesh` | Collision shape |
| `Collider::trimesh` for arbitrary meshes (requires `IndexBuffer::U32/U16`) | Mesh collision |
| `Restitution::coefficient(f32)` | Bounciness (0=no bounce, 1=max) |
| `Friction::coefficient(f32)` | Surface friction |
| `Density(f32)` | Mass per volume (auto-computes mass) |
| `AdditionalMassProperties` | Override mass/inertia |
| `Velocity` | Initial velocity |
| `ExternalForce` / `ExternalTorque` | Persistent forces |
| `LockedAxes` | Restrict translation/rotation per-axis |
| `ActiveEvents::COLLISION_EVENTS \| CONTACT_FORCE_EVENTS` | Enable event generation |
| `ActiveHooks::FILTER_CONTACT_PAIRS \| MODIFY_SOLVER_CONTACTS` | Enable callback hooks |
| `Ccd::enabled()` | Enable continuous collision detection per-body |

### Plugin Configuration
The plugin type parameter `NoUserData` can be replaced with a custom type implementing `BevyPhysicsHooks` for contact filtering/modification.

Feature flags: `simd-stable`, `simd-nightly`, `parallel`, `debug-render-3d`, `serde-serialize`, `enhanced-determinism`.

---

## 2. How to Make It High Quality

### IntegrationParameters (key quality knobs)

| Parameter | Default | High Quality | Effect |
|---|---|---|---|
| `num_solver_iterations` | 4 | 8-15 | Constraint accuracy, stacking stability |
| `num_internal_pgs_iterations` | 1 | 2-4 | Inner PGS convergence |
| `dt` | 1/60 | 1/120 or smaller by substepping | Smaller timestep = tighter simulation |
| `max_ccd_substeps` | 1 | 4-8 | Better CCD for fast objects |
| `warmstart_coefficient` | 1.0 | 1.0 (keep default) | (already optimal) |
| `contact_softness` | default | Reduce or zero for stacking | Harder contacts |
| `friction_model` | default | `FrictionModel::Solid` | Better friction for stacking |

In Bevy, set via:
```rust
fn setup_physics(mut rapier_config: ResMut<RapierConfiguration>) {
    let mut params = rapier_config.integration_parameters;
    params.num_solver_iterations = 10;
    params.dt = 1.0 / 60.0;  // or smaller with substepping
    rapier_config.integration_parameters = params;
}
```

### Best Practices for Quality
1. **Enable CCD** on player-speed objects: `.insert(Ccd::enabled())`
2. **Use `LockedAxes`** to prevent rotational drift on characters
3. **Set appropriate `Restitution` + `Friction`** — high restitution + low friction = jittery
4. **Use `Density`** not raw mass — consistent physical behavior
5. **Avoid `KinematicPositionBased`** for fast-moving platforms — use `KinematicVelocityBased`
6. **Substep manually** if 60Hz isn't enough: run `step()` multiple times per frame with smaller dt
7. **Tune `length_unit`** — if your game uses non-meter units, set this or everything feels wrong
8. **Parallel feature** (`features = ["parallel"]`) can improve multi-core utilization

### Quality vs Performance Tradeoff
| Iterations | Stack of 10 boxes | Stack of 50 boxes |
|---|---|---|
| 4 | OK, slight wobble | Jittery, may collapse |
| 8 | Solid | Good, minor drift |
| 12 | Rock solid | Solid |

---

## 3. Cloth Physics with Rapier

Rapier has **no built-in cloth solver**. Cloth is done manually using constraint networks:

### Approach: Spring-Mass Network with Joints
Create a grid of small rigid-bodies connected by joints:
- Each vertex = a small dynamic `RigidBody` (possibly with zero collision radius to avoid self-intersection)
- Edges = `SphericalJoint` or custom `SpringJoint` constraints
- Shear/bend constraints across diagonals for structural stability

### Better Approach: Custom Constraint via GenericJoint
Use `GenericJointBuilder` with customized degrees of freedom and motor settings to simulate spring forces:
```rust
let joint = GenericJointBuilder::new(JointAxesMask::LIN_X)
    .motor_position(JointAxis::LinX, rest_length, stiffness, damping);
```

### Best Approach: Use a Dedicated Cloth Crate
- **`bevy_cloth`**: Experimental, uses position-based dynamics
- **Custom WGSL compute shader**: PBD cloth on GPU, sync positions back to Rapier for collisions
- **`bevy_xpbd`**: Alternative physics engine with built-in cloth via PBD

For "beautiful cloth", PBD on GPU is the gold standard (60fps for 10k vertices). Rapier joints for cloth caps out around 500-1000 bodies before performance degrades.

### Quality Tips for Cloth
1. **High solver iterations** (12-20) — cloth needs many iterations to look soft
2. **Small timestep** (1/120+ substepping) — prevents stretchy/jittery cloth
3. **Proper damping** — 0.1-0.3 linear damping per particle removes high-frequency noise
4. **Gravity scale < 1.0** — cloth looks heavier/more natural with reduced gravity
5. **Bend constraints** — 0.1-0.3× the stiffness of structural constraints prevents collapsing
6. **Wind via `ExternalForce`** — add per-particle forces with spatial variation

---

## 4. Physics LOD (Distance-Based Iteration Count)

Rapier doesn't support per-body solver iteration counts natively. All bodies in the same island share iterations. **But** there are several strategies:

### Strategy A: Zoned Timestep (Most Practical)
Divide the scene into zones by distance from camera. Run Rapier multiple times per frame:
- **Near zone** (0-15m): Full step with 12 iterations
- **Mid zone** (15-50m): Full step with 4 iterations
- **Far zone** (50m+): Skip physics entirely or use kinematic interpolation

Implementation:
```rust
fn lod_physics_step(
    mut rapier_config: ResMut<RapierConfiguration>,
    query: Query<&Transform, With<RigidBody>>,
    camera: Query<&Transform, With<Camera3d>>,
) {
    let cam_pos = camera.single().translation;
    let mut params = rapier_config.integration_parameters;

    // Count bodies in each zone
    let mut near_count = 0;
    for tf in &query {
        let dist = tf.translation.distance(cam_pos);
        if dist < 15.0 { near_count += 1; }
    }

    if near_count > 0 {
        // High quality step for near bodies
        params.num_solver_iterations = 12;
        rapier_config.integration_parameters = params;
        // Pipeline::step() runs once
    }
}
```

### Strategy B: Multiple Worlds (Isolation)
Use Rapier's **multiple physics contexts** feature:
```rust
commands.spawn(RapierContext {
    integration_parameters: IntegrationParameters {
        num_solver_iterations: 12,
        ..default()
    },
    ..default()
});
commands.spawn(RapierContext {
    integration_parameters: IntegrationParameters {
        num_solver_iterations: 4,
        ..default()
    },
    ..default()
});
```
Put near bodies in context A, far bodies in context B. Run each at different iteration counts. Contexts are completely isolated (no cross-collision).

### Strategy C: Sleep + Dominance Groups
- Far bodies eventually fall asleep (no CPU cost)
- Use `gravity_scale(0.0)` + high damping on far bodies to force them to sleep faster
- Use `dominance_group()` to prevent interaction issues at zone boundaries

### Recommendation
**Strategy A (zoned timestep)** is simplest and most effective for most games. Run physics at 2-3 iteration levels based on distance. Bodies transitioning between zones can have brief quality changes, but these are imperceptible at speed.

---

## References
- Bevy Rapier: https://crates.io/crates/bevy_rapier3d
- Rapier docs: https://rapier.rs/docs/
- IntegrationParameters: https://docs.rs/rapier3d/latest/rapier3d/dynamics/struct.IntegrationParameters.html
- Joint types: https://rapier.rs/docs/user_guides/rust/joints
- Bevy plugin guide: https://rapier.rs/docs/user_guides/bevy_plugin/getting_started_bevy
