# HPL2 Physics Interaction Model

This note records how Amnesia: The Dark Descent / HPL2 handles physics-based
interactions (doors, drawers, grab, levers, wheels) and how Afterglow maps that
design onto avian3d joints and Bevy ECS.

## Primary Source Files

### Physics Engine Layer (HPL2/core/)

- `sources/physics/` — Newton Physics joint wrappers
- `sources/impl/PhysicsJointHingeNewton.cpp` — custom 6-DOF hinge implementation
- `sources/impl/PhysicsJointSliderNewton.cpp` — slider with limit callback
- `sources/impl/PhysicsJointScrewNewton.cpp` — corkscrew joint
- `sources/impl/PhysicsJointBallNewton.cpp` — ball (spherical) joint
- `include/physics/PhysicsJoint.h` — base joint API with sticky limits, break forces,
  sound modulation, controller chaining, limit auto-sleep
- `include/physics/PhysicsJointHinge.h` — hinge: `SetMinAngle`, `SetMaxAngle`
- `include/physics/PhysicsJointSlider.h` — slider: `SetMinDistance`, `SetMaxDistance`
- `include/physics/PhysicsController.h` — data-driven PID/Spring joint controllers
- `include/math/PidController.h` — generic PID with rolling error window

### Game Layer (amnesia/src/game/)

- `LuxPlayer.cpp` — player state machine, `DoAction` dispatch
- `LuxPlayerState_DefaultBase.cpp` — focus raycast, `OnDoAction` → `OnInteract`
- `LuxPlayerState_InteractSwingDoor.cpp` — hinge door interaction
- `LuxPlayerState_InteractSlide.cpp` — slider/drawer interaction
- `LuxPlayerState_InteractGrab.cpp` — grab using dual PID controllers
- `LuxPlayerState_InteractLever.cpp` — lever with configurable auto-return
- `LuxPlayerState_InteractWheel.cpp` — wheel with circular gesture detection
- `LuxPlayerState_InteractPush.cpp` — push heavy objects
- `LuxPlayerState_InteractRotateBase.cpp` — base class for rotary interactions (PID velocity control)
- `LuxProp_SwingDoor.cpp` — swing door prop: sticky limits, auto-close, breakable
- `LuxProp_MultiSlider.cpp` — notched multi-position slider
- `LuxProp_Lever.cpp` — lever with auto-move, stuck states
- `LuxProp_Wheel.cpp` — wheel with spin direction constraints
- `LuxProp_Object.cpp` — grabbable object config
- `LuxProp_MoveObject.cpp` — scripted linear/angular move objects
- `LuxMapHelper.cpp` — `GetClosestEntity` raycast with filtering

## Joint Architecture

### Joint Types

HPL2 uses Newton Physics and wraps four joint types:

| Type | Enum | HPL2 Class | Newton API | DOF |
|------|------|------------|------------|-----|
| Ball | `ePhysicsJointType_Ball` | `iPhysicsJointBall` | `NewtonConstraintCreateBall` | 3 rotation |
| Hinge | `ePhysicsJointType_Hinge` | `iPhysicsJointHinge` | Custom 6-DOF user joint | 1 rotation |
| Slider | `ePhysicsJointType_Slider` | `iPhysicsJointSlider` | `NewtonConstraintCreateSlider` + callback | 1 translation |
| Screw | `ePhysicsJointType_Screw` | `iPhysicsJointScrew` | `NewtonConstraintCreateCorkscrew` | 1 rotation→translation |

### Joint Creation API

```cpp
// All joints share: pivotPoint (world-space), pinDir (axis direction), parent/child body
// Parent can be NULL (attached to world/static)
iPhysicsJointHinge* = world->CreateJointHinge(name, pivot, pinDir, parentBody, childBody);
iPhysicsJointSlider* = world->CreateJointSlider(name, pivot, pinDir, parentBody, childBody);
```

### Base Joint Parameters (iPhysicsJoint)

| Method | Description |
|--------|-------------|
| `SetCollideBodies(bool)` | Whether jointed bodies collide with each other |
| `SetStiffness(float)` | Constraint stiffness [0,1] |
| `SetBreakable(bool)` + `SetBreakForce(float)` | Joint breakage |
| `SetStickyMinLimit(bool)` / `SetStickyMaxLimit(bool)` | Zero body velocity at limit when parent is static |
| `SetLimitAutoSleep(bool, dist, steps)` | Auto-disable body near limit (oscillation avoidance) |
| `GetForceSize()` | Current constraint force magnitude |

### Custom Hinge Joint (PhysicsJointHingeNewton.cpp:152-264)

The hinge is a custom 6-DOF user joint with explicit constraint rows:

```
Row 0-2 (linear): Lock pivot point in all 3 axes → pin bodies together at anchor
Row 3-4 (linear): Lock pin-perpendicular axes at a point 50 units along pin
                  → prevent pin axes from separating
Row 5 (angular):  Free rotation around pin axis (this is the hinge DOF)
```

Angular limits use error-correction angular rows:
```
if angle < minAngle:
    add angular row with (angle - minAngle) error, stiffness=1.0, maxFriction=0
    → hard stop, free to move away from limit
if angle > maxAngle:
    add angular row with (angle - maxAngle) error, stiffness=1.0, minFriction=0
```

Sticky limits: when body was at limit on previous frame AND parent is static,
zero out child body's linear + angular velocity. This prevents doors from
bouncing back when released at the fully open position.

### Slider Joint (PhysicsJointSliderNewton.cpp:35-184)

Uses Newton's built-in `NewtonConstraintCreateSlider` with a limit callback:

```cpp
unsigned LimitCallback(const NewtonJoint*, NewtonHingeSliderUpdateDesc* pDesc) {
    float dist = NewtonSliderGetJointPosit(slider);
    CheckLimitAutoSleep(this, minDist, maxDist, dist);
    if (dist < minDist) {
        pDesc->m_accel = NewtonSliderCalculateStopAccel(slider, pDesc, minDist);
        pDesc->m_minFriction = 0;  // free to move away
        return 1;
    }
    // ... same pattern for maxDist
    // sticky limits: zero velocity when at limit + parent static
}
```

### Sticky Limits (Feel-Critical Feature)

When a door or drawer is released at its limit:
1. HPL2 checks `mbStickyMinLimit` / `mbStickyMaxLimit`
2. If true AND parent body is static (or NULL, meaning world-attached):
3. `mpChildBody->SetAngularVelocity(0)` / `SetLinearVelocity(0)`
4. This prevents the natural bounce-back from constraint restitution

Without sticky limits, doors and drawers feel springy and cheap. With sticky
limits, they feel solid and weighty.

## Interaction State Machine

### Flow

```
Player presses Interact (Left Click / E)
  → LuxInputHandler (LuxInputHandler.cpp:1062)
  → LuxPlayer::DoAction(Interact, true) (LuxPlayer.cpp:735)
  → Current player state's OnDoAction():
      Normal state (DefaultBase.cpp:158):
        Check CanInteractWithEntity()
          → entity in focus? within max distance? entity accepts interaction?
        If yes: entity->OnInteract(body, position)
          → entity calls SetupInteraction() + ChangeState(newState)
      Interact states (Grab/Push/Slide/SwingDoor/Lever/Wheel):
        Handle release, throw, or continue
```

### Player States

| State | Purpose |
|-------|---------|
| `InteractGrab` | Pick up and carry objects with dual PID controller |
| `InteractPush` | Push heavy objects with velocity PID |
| `InteractSlide` | Open/close drawers and sliding doors |
| `InteractSwingDoor` | Push/pull hinged doors |
| `InteractLever` | Operate levers (with auto-return) |
| `InteractWheel` | Turn wheel valves (circular gesture detection) |

### Focus Raycast (every frame)

```cpp
// 20m ray from camera center, physics raycast
// Filters: active entities only, skip characters, skip non-collide unless CanInteract
// Returns: closest entity + body + distance
// Used for: crosshair display, outline, interaction target
```

Crosshair types per entity: `Grab`, `Push`, `Ignite`, `Pick`, `LevelDoor`, `Ladder`.

## Rotational Interaction (SwingDoor, Lever, Wheel)

### Shared Base Class (InteractRotateBase.cpp)

All rotary interactions inherit from `iLuxPlayerState_InteractRotateBase`:
```
mfRotSpeed  ← accumulated rotational speed (decays via SlowDownFactor)
mfMaxTorque = 1000 (config)
PID: P=10, I=0, D=1, error window=10 frames

Update each frame:
  1. Check distance from pivot → release if too far
  2. mfRotSpeed -= sign(mfRotSpeed) * abs(mfRotSpeed) * SlowDownFactor * dt
  3. mfRotSpeed += GetSpeedAdd(camera) * 3000.0 * MoveSpeedFactor * dt
  4. Clamp mfRotSpeed to ±MoveMaxSpeed
  5. vWantedVel = hingePinDir * mfRotSpeed
  6. vTorque = PID(vWantedVel - currentAngVel) * inertiaMatrix
  7. body->AddTorque(clamp(vTorque, ±maxTorque))
```

### GetSpeedAdd (mouse → torque direction)

```cpp
// Project mouse movement onto the hinge rotation plane:
vJointToBody = normalize(bodyCenter - jointPivot)
vPushAmount = cameraUp*mouseY + cameraRight*(-mouseX)
vPushRotateDir = cross(vJointToBody, vPushAmount)
speedAdd = dot(vPushRotateDir, hingePinDir)
```

This maps camera-relative mouse drag to signed torque around the hinge axis.
Moving the mouse in the direction the door should swing → positive torque.

### Door-Specific Behavior (SwingDoor.cpp)

- **Close**: Shrink hinge limits to ±2°, disable sticky limits, apply tiny force
- **Auto-close**: If door angle < 10° and not interacted, auto-close
- **Unlock**: Restore original hinge limits, enable sticky limits
- **Throw**: Impulse along joint forward direction
- **Breakable**: Health system, spawn broken entity on break
- **Config**: `MoveMaxSpeed=13.5`, `MoveSlowDownFactor=3.0`, `MoveThrowImpulse=6.0`

### Lever-Specific Behavior (Lever.cpp)

- **Auto-return**: Self-centers to middle/min/max via direct velocity set + PID torque
- **Stuck states**: When stuck, hinge limits narrowed to tight range around position
- **Config**: `AutoMoveToAngle=true`, `AutoMoveSpeedFactor=2.0`, `AutoMoveMaxSpeed=8.0`

### Wheel-Specific Behavior (Wheel.cpp)

- **Circular gesture detection**: Analyzes last 10 mouse deltas for circular motion
- **Spin direction**: Can be restricted to one-way or both-ways
- **Config**: Default limits ±360°, `SlowDownRotation=true`

## Grab (Dual PID Controller)

### Critical Design: NOT a Joint

The grab is not implemented as a physics constraint. It uses two PID controllers
that apply forces and torques directly to the body every frame.

### Configuration (per-object, from entity file)

| Parameter | Default | Description |
|-----------|---------|-------------|
| `GrabMassMul` | 0.1 | Body mass multiplier when held |
| `GrabThrowImpulse` | 10.0 | Impulse on throw |
| `GrabForceMul` | 1.0 | PID force output multiplier |
| `GrabTorqueMul` | 1.0 | PID torque output multiplier |
| `GrabMinDepth` | 1.0 | Min hold distance |
| `GrabMaxDepth` | 2.0 | Max hold distance |

### Force PID

```
P = 400, I = 0, D = 40, error window = 20

Goal position = camera transform * grabOffset * bodyRotation
Position error = goal - body.position
Force = PID(position error) * totalMass
Force = clamp(force, ±maxForce)
body->AddForce(force * forceMul)
```

### Torque PID

```
P = 40, I = 0, D = 0.4, error window = 20

Build wanted angular speed from cross-product error:
  bodyUp × goalUp → rotation axis, error = angle, speed += axis * error * 100
  bodyRight × goalRight → same
Torque = PID(wantedSpeed - currentAngVel) * inertiaMatrix
Torque = clamp(torque, ±maxTorque)
body->AddTorque(torque * torqueMul)
```

### On Enter

1. Save body properties (gravity, mass, collision flags)
2. Disable gravity on held body
3. Disable character collision on held body
4. Set mass to `mass * GrabMassMul` (10% of original)
5. Add object mass to player body (prevents player being pushed by heavy object)
6. Slow player movement if object is heavy
7. Zero initial velocities

### On Release

1. Clamp leave speeds (`GrabMaxLeaveLinearSpeed`, `GrabMaxLeaveAngularSpeed`)
2. Restore body properties (gravity, mass, collision)
3. Restore player mass and move speed
4. Enable collision-until-outside on the prop

### On Throw

1. Zero body velocity
2. `body->AddImpulse(cameraForward * GrabThrowImpulse)`
3. Change state to Normal

### Depth Scroll

Mouse wheel adjusts hold distance within [GrabMinDepth, GrabMaxDepth] range.
`mfDepth += scroll * GrabDepthInc`.

## Push Interaction

Push uses a velocity PID per stance (crouch/walk/run) with configurable
max speeds and forces. Push force is applied at contact point when
`mbPushAtPoint` is true. A stop-velocity PID activates when player releases.

## Afterglow → avian3d Mapping

### Joints

| HPL2 | avian3d | Notes |
|------|---------|-------|
| Hinge | `RevoluteJoint` | 1-DOF rotation around axis, with limits |
| Slider | `PrismaticJoint` | 1-DOF translation along axis, with limits |
| Ball | `SphericalJoint` | 3-DOF rotation |
| Screw | Custom (Prismatic + Revolute linked) | Deferred — not needed for initial implementation |

### Sticky Limits

avian3d does not have native sticky limits. Implement as a post-step system:
- Query all active interaction joints
- If joint is at limit AND parent is static AND body was at limit last frame:
  - Zero the child body's linear + angular velocity
- This runs every physics step, after constraint solving

### Grab

No avian3d equivalent. Implement as pure force/torque systems using the
same PID controller pattern as HPL2.

### RevoluteJoint Limits

avian3d has `RevoluteJoint` with `set_limits`:
```rust
let joint = RevoluteJoint::new(entity1, entity2)
    .with_axis(Vec3::Y)
    .with_limits(JointAxis::AngularX, -1.0, 1.0);
// Note: avian3d's limit enforcement uses restitution/impulse, not
// HPL2's error-correction + sticky stop pattern.
```

### PrismaticJoint Limits

avian3d has `PrismaticJoint`:
```rust
let joint = PrismaticJoint::new(entity1, entity2)
    .with_axis(Vec3::X)
    .with_limits(JointAxis::LinearX, 0.0, 0.5);
```

## Implementation Plan

### Module Structure

```
src/interaction/
  mod.rs         — InteractionPlugin, InteractionTarget, InteractionKind
  raycast.rs     — Focus raycast system (20m camera center ray)
  target.rs      — InteractionTarget component, focus state
  door.rs        — HingeJointConfig, HingeJointState, sticky limits system
  drawer.rs      — PrismaticJointConfig, PrismaticJointState
  grab.rs        — Grabbed component, dual PID force/torque systems
  pid.rs         — PidController utility
  tests.rs       — Edge case tests
```

### InteractionTarget Component

```rust
pub struct InteractionTarget {
    pub kind: InteractionKind,
    pub max_focus_distance: f32,
    pub focus_crosshair: FocusCrosshair,
}
```

### InteractionKind Enum

```rust
pub enum InteractionKind {
    HingedDoor {
        joint_axis: Vec3,
        limit_min: f32,
        limit_max: f32,
        move_max_speed: f32,
        move_slow_down_factor: f32,
        move_speed_factor: f32,
        move_throw_impulse: f32,
        auto_close_angle: Option<f32>,
        break_impulse: Option<f32>,
    },
    SliderDrawer {
        joint_axis: Vec3,
        limit_min: f32,
        limit_max: f32,
        move_max_speed: f32,
        move_slow_down_factor: f32,
        move_speed_factor: f32,
        move_throw_impulse: f32,
        state_count: Option<usize>, // MultiSlider notch support
    },
    Grabbable {
        mass_mul: f32,
        throw_impulse: f32,
        force_mul: f32,
        torque_mul: f32,
        min_depth: f32,
        max_depth: f32,
        max_leave_linear_speed: f32,
        max_leave_angular_speed: f32,
    },
    Lever {
        joint_axis: Vec3,
        limit_min: f32,
        limit_max: f32,
        middle_angle: f32,
        auto_move_to_angle: bool,
        auto_move_goal: AutoMoveGoal,
        auto_move_speed_factor: f32,
        auto_move_max_speed: f32,
    },
    Wheel {
        joint_axis: Vec3,
        limit_min: f32,
        limit_max: f32,
        spin_dir: SpinDir,
    },
}
```

### PidController

```rust
pub struct PidController {
    pub p: f32,
    pub i: f32,
    pub d: f32,
    pub error_window: usize,
    errors: Vec<f32>,
    time_steps: Vec<f32>,
    error_num: usize,
}

impl PidController {
    pub fn new(p: f32, i: f32, d: f32, error_window: usize) -> Self { ... }
    pub fn output(&mut self, error: f32, dt: f32) -> f32 { ... }
    pub fn reset(&mut self) { ... }
}
```

### Player Interaction State

```rust
#[derive(Resource)]
pub struct PlayerInteractionState {
    pub current: Option<InteractionMode>,
    pub focus_entity: Option<Entity>,
    pub focus_body: Option<Entity>,
    pub focus_distance: f32,
}

pub enum InteractionMode {
    Idle,
    Grabbing { ... },
    PushingDoor { ... },
    SlidingDrawer { ... },
    OperatingLever { ... },
}
```

In ECS, a stateful `PlayerInteractionState` resource is cleaner than HPL2's
vtable-based state machine. Each interaction mode has its own system that runs
when the state matches.

### Implementation Order

1. PidController utility
2. InteractionTarget component + InteractionKind enum
3. Focus raycast system (camera → physics ray each frame)
4. Hinged door: Spawn RevoluteJoint + sticky limits + mouse-drag control
5. Slider drawer: Spawn PrismaticJoint + mouse-drag control
6. Grab: Dual PID force/torque systems + mass/gravity toggle
7. fps_controller playground with door, drawer, and grabbable objects
8. Edge-case tests for each system

### Edge Cases to Cover in Tests

- Door blocked by another body → stops at obstruction, no explosion
- Door auto-close while something is in the way → re-opens or stops
- Sticky limits: door at max angle released → velocity zeroed, no bounce
- Drawer at min/max limit → same sticky behavior
- Grab: body collides with world while held → force fights collision
- Grab: max distance exceeded → auto-release
- Grab: throw into wall → impulse clamped by physics
- Multi-body object grab → all connected bodies affected
- Heavy object grab → player slowed proportionally
- Joint break force exceeded → joint breaks, body flies free
- Interaction raycast ignores player's own body
- Interaction raycast ignores non-interactable bodies
- Focus entity despawned mid-interaction → clean state transition
- Zero-mass parent body in joint → handled (attached to world)
- Frame timing: PID accumulation across variable dt

### PID Tuning Parameters (Initial HPL2 Values for Reference)

| Interaction | P | I | D | Error Window |
|-------------|---|---|---|--------------|
| Grab Force | 400 | 0 | 40 | 20 |
| Grab Torque | 40 | 0 | 0.4 | 20 |
| Door/Lever/Wheel Rotate | 10 | 0 | 1 | 10 |
| Slide Speed | 6 | 0 | 0.1 | - |
| Push Walk | 15 | 0 | 0.1 | - |
| Push Stop | 10 | 0 | 0.1 | - |
