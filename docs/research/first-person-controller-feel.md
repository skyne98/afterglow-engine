# First-Person Controller Feel Research

Sources:

- Fabrice Piquet, "True First Person Camera in Unreal Engine 4" (2018): https://www.gamedeveloper.com/programming/true-first-person-camera-in-unreal-engine-4
- Andrei Neacsu, "Recreating Quake / GoldSrc Movement in Godot 4.0" (2023): https://aneacsu.com/blog/2023-04-09-quake-movement-godot
- Project Borealis, "Open Source Movement" (2019): https://projectborealis.com/movement/
- Mark Venturelli, "Game Feel Tips II: Speed, Gravity, Friction" (2014): https://www.gamedeveloper.com/design/game-feel-tips-ii-speed-gravity-friction
- Mark Venturelli, "Game Feel Tips III: More On Smooth Movement" (2014): https://www.gamedeveloper.com/design/game-feel-tips-iii-more-on-smooth-movement
- Evan Todd, "The Poor Man's Character Controller" (2015): https://etodd.io/2015/04/03/poor-mans-character-controller/
- Game Design Framework, "Building Character Feel in a First Person Game": https://gamedesignframework.net/building-character-feel-in-a-first-person-game/

Additional implementation references found by research agent:

- Quake III Arena `bg_pmove.c`: https://github.com/id-Software/Quake-III-Arena/blob/master/code/game/bg_pmove.c
- Project Borealis character movement repository: https://github.com/ProjectBorealis/PBCharacterMovement

## Goal

Afterglow needs a first-person controller that is useful for Arx/Thief/System
Shock-style immersive sim spaces first, then open-world RPG traversal later. The
controller should feel grounded and readable, but it should not be a stiff
floating camera. It must consume `PlayerCommand` values from the engine input
system, so keyboard, gamepad, touch, AI possession, cutscene scripts, network
prediction, and tests all drive the same path.

The implementation target is not exact Quake, Source, or Mirror's Edge. The
target is a conservative engine controller with knobs that can emulate pieces of
those feels per game.

## Source Takeaways

### True First-Person Body Awareness

The UE4 true-first-person article argues for a full-body mesh where the camera
is attached to the character body instead of using disembodied first-person
arms. The camera orientation is represented as local aim offsets on the animated
body, not as an independent camera transform. The important implementation
detail is ordering: camera/aim data must be updated before animation and camera
evaluation or the player sees a one-frame delay.

For Afterglow:

- Keep controller movement independent from visual body awareness.
- Store authoritative look intent as controller state: yaw, pitch, optional roll.
- Drive the gameplay capsule from yaw, but drive camera/head/mesh presentation
  from a separate presentation layer.
- Do not require a skeletal mesh for the first controller milestone.
- Later, true-first-person mode should read the controller view state and feed
  animation/aim offsets before camera extraction.
- For networked play, replicated state should stay minimal: body transform,
  velocity, stance, and compressed view yaw/pitch. Local-only camera smoothing,
  head bob, weapon sway, and animation offsets should be presentation state.

### Quake/GoldSrc Acceleration

The Quake/Godot article shows the core shape we want: compute a per-tick wish
direction, normalize it so diagonals are not faster, then update velocity
differently for grounded and airborne movement. Ground movement applies
friction; air movement applies gravity and reduced acceleration. The important
non-obvious detail is Quake-style acceleration limiting by projected speed along
the wish direction, not direct clamping of total velocity.

For Afterglow:

- Use command axes like `move.x`, `move.y`, `look.x`, `look.y`, `jump`,
  `crouch`, and `sprint` as configurable strings, but keep defaults optional.
- Convert move axes into a normalized wish direction in the actor yaw plane.
- Keep horizontal velocity separate from vertical velocity for tuning.
- Apply ground friction before acceleration, except during jump takeoff if we
  want bunny-hop-friendly behavior.
- Implement acceleration using projection:
  `current = dot(horizontal_velocity, wish_dir)`;
  add at most `max_speed - current` along `wish_dir`.
- Use separate knobs for ground acceleration, air acceleration, max ground speed,
  max air wish speed, friction, stop speed, gravity, jump impulse, and terminal
  fall speed.
- Keep the original Quake movement code as a reference for the order of
  operations: friction, acceleration, slide/clip, step attempt, and final
  collision flags are all part of the feel.

### Source / Project Borealis Movement

Project Borealis published an Unreal movement implementation intended to match
Half-Life 2 / Source movement, including air strafing, Source-style input
acceleration, accelerated backhopping, smooth crouch transitions, damage
boosting, wall strafing, ramp sliding, and surfing. Their process also matters:
they validated movement with expert playtesters because small edge cases define
the feel.

For Afterglow:

- Do not implement every Source quirk by default.
- Make the base controller deterministic and conservative, with optional knobs
  for advanced movement.
- Expose wall sliding, ramp sliding, and air acceleration as tunable policies.
- Keep crouch transitions smooth visually, but keep physics stance changes
  preflighted by clearance checks.
- Add regression tests for movement quirks as features are enabled, because
  source-like movement has many behavior-defining edge cases.

### Speed, Gravity, Friction

Mark Venturelli's game-feel articles frame movement as a balance of acceleration,
friction, and maximum speed. Instant stop/start can feel precise on paper but
often feels worse in practice. Higher acceleration plus higher friction can keep
movement responsive without making it weightless. Lower acceleration can improve
fine positioning, but if it is too low the first step feels heavy.

For Afterglow:

- Avoid binary velocity changes. Always move through acceleration/friction
  curves, even if tuned aggressively.
- Preserve a "responsive" preset with high acceleration and high friction.
- Preserve a "deliberate immersive sim" preset with lower max speed, firm stop
  friction, modest air control, and high head/camera stability.
- Treat absolute values as meaningful, not only ratios. Doubling both
  acceleration and friction changes feel.
- Tests should assert qualitative behavior as numeric invariants: reaches 90%
  target speed within N ticks, stops below threshold within N ticks, reversing
  direction does not overshoot beyond a chosen bound.

### Collision, Steps, Slopes, And Forgiveness

Evan Todd's controller writeup highlights production problems: simple character
controllers get stuck on small geometry, seams, voxel edges, and wall glancing.
The practical lessons are to use forgiving collision shapes, slide along walls,
smooth camera height over uneven ground, and add jump forgiveness. Capsule
colliders avoid many seam issues compared with cylinders. Coyote time and input
buffering make jump timing less frustrating.

For Afterglow:

- Use an Avian capsule collider for the player body.
- Use a kinematic/character-controller style motor rather than a fully dynamic
  rigid body for the main player, unless tests prove dynamic behavior is stable.
- Separate collision body height from camera height; smooth camera height but
  keep collision state exact.
- Implement wall slide by projecting attempted horizontal displacement onto the
  contact plane when allowed by slope rules.
- Implement coyote time and jump buffering as first-class controller knobs.
- Implement step handling with explicit step height and forward/up clearance
  probes, not by relying on random physics impulses.
- Reject stance changes when the expanded capsule would overlap geometry.

### Control Mapping And Responsiveness

The Game Design Framework article emphasizes conventional mappings, grouped
controls, and fast perceptual feedback. It gives a useful practical target:
players should see a response within roughly the first tenth of a second after
input. This supports our command-driven input design: control sources should be
configurable, but the controller should provide clear semantic command names and
small latency from command to motion.

It also calls out camera/input comfort: response curves, optional raw/linear
input, restrained camera roll, quick recentering of secondary motion, and
avoiding forced view movement that fights aiming.

For Afterglow:

- The engine should provide suggested action/axis constants or examples, but
  should not hardcode one game's bindings.
- Context-aware input should keep movement controls grouped and easy to replace.
- Command-to-motion should run in the fixed simulation/update path without an
  extra frame of buffering.
- Visual smoothing must not delay authoritative collision or hit detection.

## Proposed Afterglow Controller Shape

### Components And Resources

```rust
#[derive(Component)]
pub struct FirstPersonController {
    pub player: NetworkPlayerId,
    pub config: FirstPersonControllerConfig,
}

#[derive(Component)]
pub struct FirstPersonMotorState {
    pub velocity: Vec3,
    pub grounded: bool,
    pub ground_normal: Vec3,
    pub stance: ControllerStance,
    pub yaw: f32,
    pub pitch: f32,
    pub coyote_ticks: u8,
    pub jump_buffer_ticks: u8,
}

pub struct FirstPersonControllerConfig {
    pub move_x_axis: String,
    pub move_y_axis: String,
    pub look_x_axis: String,
    pub look_y_axis: String,
    pub jump_action: String,
    pub crouch_action: String,
    pub walk_action: String,
    pub sprint_action: String,
    pub ground_speed: f32,
    pub walk_speed: f32,
    pub sprint_speed: f32,
    pub ground_accel: f32,
    pub air_accel: f32,
    pub friction: f32,
    pub stop_speed: f32,
    pub gravity: f32,
    pub jump_speed: f32,
    pub max_slope_angle: f32,
    pub step_height: f32,
    pub coyote_ticks: u8,
    pub jump_buffer_ticks: u8,
}
```

Names are illustrative. The actual API should keep files below 500 LOC and use
existing engine patterns.

### System Flow

1. Read `PlayerCommandQueue`.
2. Match commands to controller entities by `NetworkPlayerId`.
3. Update yaw/pitch from look axes.
4. Convert move axes into yaw-relative wish direction.
5. Probe grounding and slope normal using Avian queries.
6. Update jump buffer and coyote timers.
7. Apply friction, acceleration, gravity, and jump impulse.
8. Move the capsule with swept collision / Avian character movement.
9. Resolve wall slide, step-up, and stance clearance.
10. Write exact body transform and motor state.
11. Presentation systems derive camera/head/body visuals from motor state.

### Prediction And Networking

The controller is a good fit for current networking:

- Input is already serialized as `PlayerCommand` by tick.
- Server and local single-player can run the same command-to-motor system.
- Client prediction records the same commands and can replay after correction.
- Authoritative snapshots should replicate body transform, velocity, stance,
  grounded flag, and view yaw/pitch.
- Presentation-only camera smoothing should not be replicated.

### Test Plan

Headless physics tests should cover:

- no input keeps the player stable on flat ground
- forward command accelerates toward target speed
- releasing input stops below a small speed threshold
- diagonal input is not faster than cardinal input
- reversing direction remains bounded and deterministic
- jumping works on ground
- coyote-time jump works shortly after leaving ground
- buffered jump fires on landing
- no double jump when disabled
- air acceleration is lower than ground acceleration
- capsule slides along walls instead of stopping dead
- slope below limit is walkable
- slope above limit is not walkable
- step below `step_height` is climbed
- step above `step_height` blocks
- crouch changes stance when clear
- uncrouch is rejected under low ceiling
- scripted/cutscene `VirtualInputState` commands move the controller exactly
  like physical input commands
- replaying the same command sequence from the same initial state gives the same
  final motor state

## Implementation Recommendation

Start minimal and strict:

- capsule body
- command-driven yaw/pitch and movement
- ground/air acceleration
- friction and gravity
- jump, coyote time, jump buffer
- crouch with clearance checks
- slope limit
- basic wall slide
- step handling only after the flat/slope/jump tests are stable

Defer:

- full true-first-person body mesh
- procedural head bob and weapon sway
- Source quirks like accelerated backhopping, surfing, and ramp boosts
- ledge grabbing and parkour moves
- dynamic push/ride platforms

The main engineering risk is not the acceleration math. It is collision
resolution around steps, slopes, seams, and moving geometry. Build the controller
behind tests and keep every special case represented by a regression.
