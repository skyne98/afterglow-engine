# Interaction System

HPL2-style physics-based interactions using avian3d joints and PID force control.

## Architecture

The interaction module provides three interaction types:

- **Hinged doors**: `RevoluteJoint` with angle limits, PID torque control, sticky limits
- **Slider drawers**: `PrismaticJoint` with distance limits, PID force control, sticky limits
- **Grabbable objects**: Dual PID force/torque controller (not a joint), gravity/mass modulation

## Components

### InteractionTarget

Attached to any entity the player can interact with:

```rust
pub struct InteractionTarget {
    pub kind: InteractionKind,
    pub max_focus_distance: f32,
    pub focus_crosshair: FocusCrosshair,
}
```

### InteractionKind

```rust
pub enum InteractionKind {
    Grabbable {
        mass_mul: f32,          // body mass * this when held (HPL2: 0.1)
        throw_impulse: f32,     // impulse on right-click throw
        force_mul: f32,         // PID force output multiplier
        torque_mul: f32,        // PID torque output multiplier
        min_depth: f32,         // min hold distance
        max_depth: f32,         // max hold distance (mouse wheel)
        max_leave_linear_speed: f32,
        max_leave_angular_speed: f32,
    },
    HingedDoor {
        move_max_speed: f32,        // HPL2 default: 13.5
        move_slow_down_factor: f32, // HPL2 default: 3.0
        move_speed_factor: f32,
        move_throw_impulse: f32,    // HPL2 default: 6.0
    },
    SliderDrawer {
        move_max_speed: f32,
        move_slow_down_factor: f32,
        move_speed_factor: f32,
        move_throw_impulse: f32,
    },
}
```

## Joint Components

### HingeJointConfig

Angular joint config for doors and levers:

```rust
pub struct HingeJointConfig {
    pub axis: Vec3,
    pub limit_min: f32,
    pub limit_max: f32,
    pub auto_close_angle: Option<f32>,
    pub break_impulse: Option<f32>,
    pub move_max_speed: f32,
    pub move_slow_down_factor: f32,
    pub move_speed_factor: f32,
    pub move_throw_impulse: f32,
}
```

### PrismaticJointConfig

Linear joint config for drawers/sliders:

```rust
pub struct PrismaticJointConfig {
    pub axis: Vec3,
    pub limit_min: f32,
    pub limit_max: f32,
    pub state_count: Option<usize>,
    pub move_max_speed: f32,
    pub move_slow_down_factor: f32,
    pub move_speed_factor: f32,
    pub move_throw_impulse: f32,
}
```

## PlayerInteractionState

```rust
pub struct PlayerInteractionState {
    pub focus_entity: Option<Entity>,
    pub focus_body: Option<Entity>,
    pub focus_distance: f32,
    pub active_interaction: Option<ActiveInteraction>,
}
```

## Systems

### Focus Raycast

`update_focus` runs every frame in `Update`:
- 20m ray from camera center
- Finds closest entity with `InteractionTarget` within `max_focus_distance`
- Stores result in `PlayerInteractionState`

### Door Interaction

`interact_door_system`:
- Press E/LMB on a focused door → spawns `RevoluteJoint` connecting door to its parent
- Hold E/LMB + mouse → speed accumulation with slowdown decay, PID torque to body
- Release → despawns joint, optional break impulse

`sticky_door_limits`:
- Reads `JointForces` on each RevoluteJoint
- When impulse exceeds threshold at limit, zeros the door body's angular velocity
- Prevents bounce-back (HPL2 sticky limit behavior)

### Drawer Interaction

`interact_drawer_system`:
- Same as door but with `PrismaticJoint` and linear force

`sticky_drawer_limits`:
- Same sticky limit behavior for prismatic joints

### Grab Interaction

`interact_grab_start`:
- Press E/LMB on grabbable target
- Inserts `Grabbed` + `ConstantForce` + `ConstantTorque` components
- Saves/restores body mass

`update_grabbed_objects`:
- Dual PID controller (HPL2: P=400/D=40 force, P=40/D=0.4 torque)
- Force PID tracks camera-relative position goal
- Torque PID aligns body rotation to camera rotation
- Updates `ConstantForce`/`ConstantTorque` each frame

`release_grabbed_on_interact_release`:
- Removes `Grabbed` + `ConstantForce` + `ConstantTorque` on E/LMB release

`release_distant_grabbed_objects`:
- Auto-release when grabbed object exceeds `GrabConfig.grab_deactivate_distance`

`throw_grabbed_object`:
- Right-click: zeros body velocity, applies forward impulse

## Resources

### GrabConfig

```rust
pub struct GrabConfig {
    pub grab_force_p: f32,       // 400
    pub grab_force_d: f32,       // 40
    pub grab_torque_p: f32,      // 40
    pub grab_torque_d: f32,      // 0.4
    pub max_force: f32,          // 1000
    pub max_torque: f32,         // 1000
    pub grab_deactivate_distance: f32, // 3.0
}
```

### DoorPidState / DrawerPidState / GrabPidState

Per-interaction-type PID state, matching HPL2 tuning:

| Interaction | P | I | D | Window |
|-------------|---|---|---|--------|
| Door rotate | 10 | 0 | 1 | 10 |
| Drawer speed | 6 | 0 | 0.1 | 10 |
| Grab force | 400 | 0 | 40 | 20 |
| Grab torque | 40 | 0 | 0.4 | 20 |

## Usage

```rust
// Spawn a hinged door
commands.spawn((
    PhysicsBody::dynamic(),
    PhysicsCollider::cuboid(door_size),
    Mesh3d(meshes.add(Cuboid::from_size(door_size))),
    HingeJointConfig::new_door(Vec3::Y),
    InteractionTarget {
        kind: InteractionKind::HingedDoor { .. },
        max_focus_distance: 3.0,
        focus_crosshair: FocusCrosshair::LevelDoor,
    },
));

// Spawn a grabbable object
commands.spawn((
    PhysicsBody::dynamic(),
    PhysicsCollider::sphere(0.3),
    InteractionTarget {
        kind: InteractionKind::Grabbable { .. },
        ..
    },
));
```

## Edge Cases Handled

- Door blocked by another body: joint absorbs PID torque, no explosion
- Sticky limits: velocity zeroed at limit to prevent bounce-back
- Grab at max distance: auto-release
- Grab release: `ConstantForce`/`ConstantTorque` removed
- Multi-body interaction: separate joints per entity pair
- Zero-mass parent in joint: normalized axis, clamped defaults
