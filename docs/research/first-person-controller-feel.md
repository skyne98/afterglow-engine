# First-Person Controller Feel Research

Sources:

- Fabrice Piquet, "True First Person Camera in Unreal Engine 4" (2018): https://www.gamedeveloper.com/programming/true-first-person-camera-in-unreal-engine-4
- Andrei Neacsu, "Recreating Quake / GoldSrc Movement in Godot 4.0" (2023): https://aneacsu.com/blog/2023-04-09-quake-movement-godot
- Project Borealis, "Open Source Movement" (2019): https://projectborealis.com/movement/
- Mark Venturelli, "Game Feel Tips II: Speed, Gravity, Friction" (2014): https://www.gamedeveloper.com/design/game-feel-tips-ii-speed-gravity-friction
- Mark Venturelli, "Game Feel Tips III: More On Smooth Movement" (2014): https://www.gamedeveloper.com/design/game-feel-tips-iii-more-on-smooth-movement
- Evan Todd, "The Poor Man's Character Controller" (2015): https://etodd.io/2015/04/03/poor-mans-character-controller/
- Game Design Framework, "Building Character Feel in a First Person Game": https://gamedesignframework.net/building-character-feel-in-a-first-person-game/
- Frictional Games, `AmnesiaTheDarkDescent` source: https://github.com/FrictionalGames/AmnesiaTheDarkDescent
- Jolt Physics `CharacterVirtual`: https://jrouwe.github.io/JoltPhysics/class_character_virtual.html

Additional implementation references found by research agent:

- Quake III Arena `bg_pmove.c`: https://github.com/id-Software/Quake-III-Arena/blob/master/code/game/bg_pmove.c
- Project Borealis character movement repository: https://github.com/ProjectBorealis/PBCharacterMovement
- Jolt `CharacterVirtual.cpp`: https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Character/CharacterVirtual.cpp
- Jolt `CharacterVirtual.h`: https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Character/CharacterVirtual.h

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
- Drive the gameplay cylinder from yaw, but drive camera/head/mesh presentation
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
smooth camera height over uneven ground, and add jump forgiveness. Generic
controller guidance often prefers capsule colliders, but HPL2's actual player
body uses vertical cylinders; afterglow follows HPL2 for this controller.
Coyote time and input buffering make jump timing less frustrating.

For Afterglow:

- Use an Avian cylinder collider for the player body to match HPL2.
- Use a kinematic/character-controller style motor rather than a fully dynamic
  rigid body for the main player, unless tests prove dynamic behavior is stable.
- Separate collision body height from camera height; smooth camera height but
  keep collision state exact.
- Implement wall slide by projecting attempted horizontal displacement onto the
  contact plane when allowed by slope rules.
- Implement coyote time and jump buffering as first-class controller knobs.
- Implement step handling with explicit step height and forward/up clearance
  probes, not by relying on random physics impulses.
- Reject stance changes when the expanded cylinder would overlap geometry.

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

### Amnesia: The Dark Descent / HPL2 Character Controller

The Amnesia source is a strong horror-controller reference because it separates
the hard collision motor from the player feel layer. The HPL2 engine owns an
`iCharacterBody` with a vertical cylinder body, explicit move axes, custom
gravity, force velocity, collision pushback, slope handling, step climbing,
ground hysteresis, and attached camera/entity updates. The game code owns
`cLuxPlayer`, move states, interaction states, crouch, jump, camera roll, head
offsets, head bob, sanity/event speed multipliers, and sounds.

Recorded source spans:

- Engine body API and update path:
  `HPL2/core/sources/physics/CharacterBody.cpp:787-794`,
  `948-1027`.
- Horizontal acceleration/deceleration:
  `HPL2/core/sources/physics/CharacterBody.cpp:1411-1516`.
- Slope projection, horizontal collision, and step rays:
  `HPL2/core/sources/physics/CharacterBody.cpp:1536-1690`.
- Gravity, force collision, ground retention, and no-slide slope logic:
  `HPL2/core/sources/physics/CharacterBody.cpp:1711-1912`.
- Camera attachment and camera-position smoothing:
  `HPL2/core/sources/physics/CharacterBody.cpp:1996-2056`.
- Player update ordering:
  `amnesia/src/game/LuxPlayer.cpp:352-388`.
- Player input pass-through to state filters and character body:
  `amnesia/src/game/LuxPlayer.cpp:645-669`, `675-712`.
- Character body creation and tuning from config:
  `amnesia/src/game/LuxPlayer.cpp:1430-1458`.
- Normal movement state order and speed multipliers:
  `amnesia/src/game/LuxMoveState.cpp:65-80`,
  `amnesia/src/game/LuxMoveState_Normal.cpp:231-253`,
  `453-536`.
- Jump hold, head bob, footstep timing:
  `amnesia/src/game/LuxMoveState_Normal.cpp:308-338`,
  `537-673`.
- Crouch fit checks and smoothed head height:
  `amnesia/src/game/LuxMoveState_Normal.cpp:356-414`,
  `amnesia/src/game/LuxPlayer.cpp:1193-1232`.

Important design observations:

- Movement input is accumulated as intent, not applied directly to position.
  `cLuxPlayer::Move` lets player and movement states veto/modify the input,
  then calls `iCharacterBody::Move`, which only records per-axis acceleration
  and a moving flag. The body consumes and clears that intent during its update.
- The engine keeps forward/right movement speeds as stateful values. When input
  stops, speeds decay with configured deacceleration. When input reverses, an
  opposite-direction acceleration multiplier makes turns feel responsive.
- Acceleration is added as `move_input * move_acc * dt`, then capped against the
  relevant max speed. It is not multiplied by max speed; doing that makes short
  left/right taps jump too hard and stops feeling like Amnesia.
- Forward/backward and sideway movement are separate channels. HPL2 exposes
  separate max forward, max backward, max sideway, forward acceleration, sideway
  acceleration, forward deacceleration, sideway deacceleration, and reverse
  acceleration multipliers. A single normalized horizontal wish vector is less
  faithful and loses the careful low-speed precision of the original.
- Diagonal movement is normalized with a `0.7071` multiplier when both axes are
  active. The code deliberately leaves some advanced strafe behavior rather than
  over-clamping all velocity.
- The body update order is explicit: update orientation basis, step-climb state,
  integrate horizontal character speed, align motion to ground normal, solve
  horizontal collision, apply gravity/forces, solve force collision, apply
  friction, then update attached camera/entity.
- Grounding is not a single frame boolean. `mlOnGroundCount` gives hysteresis so
  small contact gaps do not immediately make the player airborne. This is useful
  for horror spaces with uneven floors and authored clutter.
- Step climbing is a deliberate ray/probe feature, not a side effect of rigid
  body impulses. It casts one or three downward rays in the movement direction,
  checks max step height, then verifies the body fits before climbing.
- Crouch changes use separate body sizes and a fit preflight before standing.
  Head height moves separately through `MoveHeadPosAdd`, so the collision stance
  can change exactly while the camera eases into place.
- Camera presentation is layered but still anchored to the body. The character
  body can smooth camera position by averaging recent head positions, while
  `cLuxPlayer` layers FOV, aspect, roll, lean roll, head offsets, head spin, and
  head bob on top.
- Jump is split into an initial upward force plus a timed gravity-relief
  window. Release does not cancel the assist in Amnesia; the timer owns it.
- Player feel is heavily config-driven. Body gravity, mass, slope angle, push
  forces, step size, step speed, camera smoothing, movement multipliers, run,
  crouch, jump, and bob values are loaded from game config.

### Jolt CharacterVirtual Stair Handling

Jolt's `CharacterVirtual::ExtendedUpdate` is a useful robustness reference for
stairs. The important upstream spans are `CharacterVirtual.cpp:1395-1437` for
normal update, `1542-1562` for `CanWalkStairs`, `1564-1703` for `WalkStairs`,
and `1705-1812` for `StickToFloor`/`ExtendedUpdate`; the public settings are in
`CharacterVirtual.h:397-451`.

Jolt does not run a detached stair ray before movement. It first performs normal
shape movement. If desired horizontal progress was not achieved and the active
contacts show a too-steep blocking surface, it tries stairs as a second pass.
The stair pass casts the full character shape upward by the maximum step height,
moves the raised shape forward, checks that the forward motion made real
progress against the steep blocking normal, then casts the shape downward and
accepts only a walkable landing normal.

Two details matter for Afterglow:

- The progress check prevents speed boosts and false positives while sliding
  along a wall or stair edge.
- The extra forward/down test handles high frame rates where the first downward
  cast hits the side or edge of a stair and reports a too-steep normal.

Afterglow now uses the exact HPL2 raycast algorithm: 1 or 3 rays (configurable
via `accurate_climbing`) from chest height downward in the movement direction,
a shape fit test at the raised position, and direct lift by `step_climb_speed *
dt`. The reactive system re-detects the step every climbing frame, matching
HPL2's `UpdateStepClimbing` → `CheckStepClimbing` loop without an accumulator.

Afterglow implications:

- Keep our controller as a small motor plus optional presentation components.
  Do not bake horror head bob, sanity, interaction slowdown, or camera roll into
  the authoritative movement component.
- Keep state filtering generic. Game/player states should be able to consume a
  movement command and decide whether it reaches the motor, but the motor should
  stay unaware of ladders, item interaction, sanity, or cutscenes.
- Add ground hysteresis and exact stance-fit tests before broadening features.
  These are cheap, deterministic, and directly improve controller feel.
- Implement step handling with probes and fit tests once the base Avian motor is
  stable. Do not rely on dynamic impulses to climb small steps.
- Use separate authoritative body state and local camera-feel state. Networked
  play should replicate position, velocity, yaw, pitch, stance, and grounded
  status; camera smoothing/head bob/roll should stay local.
- Expose a small set of presets, but keep all tuning data in config/components
  so a horror game can get Amnesia-like weight while another game can choose a
  faster Source-like controller.

## Proposed Afterglow Controller Shape

### Components And Resources

The implemented API is documented in `docs/api/controller.md`. In short:

- attach `FirstPersonController { player, config }` to the player body
- drive it only with `PlayerCommand` axes/actions, never raw input
- keep runtime state in `FirstPersonMotorState`
- authoring automatically installs the Avian kinematic cylinder
- camera feel is a separate `FirstPersonCameraRig`

### System Flow

1. Read `PlayerCommandQueue`.
2. Match commands to controller entities by `NetworkPlayerId`.
3. Update yaw/pitch from look axes.
4. Update HPL2 local forward/side speed channels.
5. Apply HPL2-style horizontal movement and pushback.
6. Run stair validation after horizontal pushback.
7. Apply gravity/jump assist after stair handling.
8. Resolve vertical collision and grounded hysteresis.
9. Resolve stance clearance with HPL2 fit offsets.
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
- cylinder slides along walls instead of stopping dead
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

- cylinder body
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

Afterglow implementation should keep Amnesia-style camera feel as presentation
state on a camera rig, not as gameplay state on the body. The first pass should
include smoothed eye position, separate crouch eye smoothing, walk/run/crouch
bob knobs, bob-phase footstep messages, landing bounce, lean roll, sprint FOV
kick, one-shot camera impulses, and weighted child head offsets for scripted or
horror-specific effects.

The main engineering risk is not the acceleration math. It is collision
resolution around steps, slopes, seams, and moving geometry. Build the controller
behind tests and keep every special case represented by a regression.

## HPL2 Consistency Todo

The step and ground path is now tracked against
`HPL2/core/sources/physics/CharacterBody.cpp`:

- [x] Replace tick-based step checking with HPL2's float countdown
  `mfCheckStepClimbCount`.
- [x] Add the HPL2 `mbClimbing` latch to the motor state.
- [x] Run HPL2-style `UpdateStepClimbing` before movement: climbing forces
  immediate step checks, pins ground contact, then clears the latch.
- [x] Skip grounded gravity/snap while climbing, matching HPL2's
  `mbGravityActive && mbClimbing == false` guard.
- [x] Replace Jolt-style stair validation with exact HPL2 raycast algorithm
  (1 or 3 rays from chest to feet, shape fit, direct lift, reactive per-frame).
- [x] Add `accurate_climbing` (3 rays) and `climb_forward_mul` config to match
  HPL2 fields.
- [x] Remove `step_forward_distance`, `step_forward_test_distance`, and
  `step_target_y` accumulator (HPL2 uses reactive re-detection).
- [x] Remove the `failed_step_block` speed clipping — HPL2 does not clip intent
  on failed steps.
- [x] Default `step_climb_speed` to `1.0` (HPL2 constructor default).
- [x] Derive stair attempt direction from horizontal character movement only, not
  gravity or grounded snap velocity; HPL2 calls `CheckStepClimbing` before
  `UpdateForces`, so vertical gravity/snap must not tilt stair rays.
- [x] Lift directly by `step_climb_speed * dt` when a step is detected; set
  `climbing = true`, zero vertical velocity. Reactive re-detection handles
  multi-frame climbs (same as HPL2).
- [x] Use `maxStepHeight` only when firmly grounded or already climbing;
  otherwise use `maxStepHeightInAir`.
- [x] Remove the old step-climb clamp helper, because HPL2 does not clamp the
  per-frame climb amount to the detected step height.
- [x] Replace the controller capsule with an Avian vertical cylinder, matching
  HPL2's `CreateCylinderShape(radius, height)` character body and crouch shape.
- [x] Keep ground probing from pulling airborne bodies downward; HPL2 landing
  comes from vertical collision, while ray fallback maintains normals only
  after contact.
- [x] Keep vertical force collision from changing horizontal position; horizontal
  movement owns X/Z correction so low blocker top edges cannot add side jitter.

Remaining known difference: HPL2 has a separate `MaxNoSlideSlopeAngle` gravity
reflection path. Afterglow currently prevents slope drift through ground-normal
snap and Avian collision projection; a literal no-slide reflection pass should
be added if ramp behavior still differs under the updated step path.
