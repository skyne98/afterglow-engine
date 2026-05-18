# First-Person Controller API

Afterglow's first-person controller is input-abstraction driven. The rewrite
target is to consume `ActionState<AfterglowAction>` from Leafwing-controlled
player entities, never raw keyboard, mouse, or gamepad state. Physical input,
scripted cutscenes, AI possession, tests, and network prediction should all drive
the same fixed-tick movement path.

## Plugin

| Item | Description |
|---|---|
| `AfterglowFirstPersonControllerPlugin` | Registers controller reflection, initializes `FirstPersonControllerTrace`, and runs controller authoring plus movement in the fixed gameplay schedule. Included in `AfterglowRuntimePlugins`. |

## Components

| Type | Description |
|---|---|
| `FirstPersonController` | Attach to the player body. Stores `FirstPersonControllerConfig`. |
| `FirstPersonControllerConfig` | Tunable jump enablement, crouch mode, HPL2 local speed channels, acceleration/deacceleration, gravity, jump/coyote/buffer windows, look sensitivity, slope limit, HPL2 raycast stair climbing, HPL2 ground hysteresis, stance dimensions, and Avian depenetration knobs. |
| `FirstPersonMotorState` | Runtime motor state: velocity, grounded flag, ground normal, stance, yaw/pitch, coyote/jump-buffer windows, jump input latch, ground-contact hysteresis, Amnesia jump-assist timer, stair timer, and climbing latch. |
| `FirstPersonCameraRig` | Attach to a camera entity. Points at a controller body and applies first-person presentation effects without feeding back into collision or gameplay state. |
| `FirstPersonCameraConfig` | Tunable eye heights, position/crouch smoothing, Amnesia/HPL2 walk/run/crouch bob amplitude plus min/max bob speed, landing bounce, FOV kick, impulse decay, and head-offset smoothing. |
| `FirstPersonCameraState` | Runtime camera-only state: initialized flag, smoothed eye height/position, bob phase, bobbing flag, current bob amplitude, landing bounce, roll, FOV, grounded/stair smoothing latches, impulse offsets, smoothed head offset, and footstep count. |
| `FirstPersonHeadOffset` | Optional child component on the controller body. Adds weighted procedural offsets for interaction, damage, scripted, horror, or crouch effects. |
| `FirstPersonCameraImpulse` | Optional child component on the controller body. One-shot pitch/yaw/roll kick consumed by the camera rig and then despawned. |
| `ControllerStance` | `Standing` or `Crouching`. |

## Debug Trace

| Type | Description |
|---|---|
| `FirstPersonControllerTrace` | Disabled-by-default resource for diagnosing movement and camera jitter. Insert `FirstPersonControllerTrace::enabled(max_frames)` or set `enabled = true` to record a bounded ring of recent frames. |
| `FirstPersonControllerTraceFrame` | One controller-body frame: action axes/buttons, phase positions, intended horizontal delta, horizontal depenetration pushback, stair trace, gravity/vertical delta, vertical pushback, final grounding, local speed, and velocity. |
| `FirstPersonStepTrace` | One HPL2 stair attempt: whether the check ran, whether it accepted, per-frame lift, ray count, forward length, max step, final rejection reason, and per-ray results. |
| `FirstPersonStepRayTrace` | One stair ray result: ray start/end, hit distance, inferred step height, fit-test position, and reason (`NoRayHit`, `TooLow`, `TooHigh`, `ShapeBlocked`, or `Accepted`). Frame-level attempts can also report `RateLimited` or `NoHorizontalDelta`. |
| `FirstPersonCameraTraceFrame` | One camera presentation frame: target body, smoothed base position, bob/landing offset, final camera position, bob phase/amplitude, landing bounce, and footstep emission. |

Use the trace when the controller jitters: if `after_horizontal_position`
alternates because `horizontal_pushback` alternates, the issue is body collision
or depenetration; if body phase positions are smooth but `bob_offset` alternates,
the issue is camera presentation; if stair rays flip between `Accepted` and
`ShapeBlocked`/`TooHigh`, the issue is step detection or the fit pose.

## Messages

| Type | Description |
|---|---|
| `FirstPersonFootstep` | Emitted by the camera rig when the bob phase crosses a footfall. Audio and surface systems can consume this without duplicating movement timing. |

Adding `FirstPersonController` automatically authors a kinematic Avian cylinder
for the entity: `PhysicsBody::kinematic()`, `PhysicsCollider::Cylinder`,
`CustomPositionIntegration`, and a zero speculative margin. `PhysicsBody` and
`PhysicsCollider` remain the single authoring source; `AfterglowPhysicsPlugin`
mirrors them to Avian `RigidBody`/`Collider` components. The controller derives
its active collision shape from config/state each frame, so movement does not
depend on direct game-authored Avian colliders. This matches HPL2's character
body shape, where normal standing/crouching bodies are vertical cylinders and
only sphere-like bodies use a sphere fallback.

## Actions

The controller reads entity-scoped `ActionState<AfterglowAction>`:

| Action | Target enum variant |
|---|---|
| Move axis | `AfterglowAction::Move` dual axis |
| Look axis | `AfterglowAction::Look` dual axis |
| Jump action | `AfterglowAction::Jump` |
| Crouch action | `AfterglowAction::Crouch` |
| Sprint action | `AfterglowAction::Sprint` |

Games can override bindings through Leafwing `InputMap`; the controller reads the
action state, not raw devices or string action names. The FPS demo installs WASD,
mouse look, gamepad sticks, Space, Shift, Ctrl, and mouse/action bindings as one
example.

`crouch` is hold-to-crouch by default. Set
`FirstPersonControllerConfig::toggle_crouch = true` for games that want a
toggle instead.

`jump_enabled` defaults to `true`. Set it to `false` for grounded horror,
inventory, ladder, cinematic, swimming, or game-specific movement modes where a
jump action may still exist but must not create an impulse or variable-height
jump relief.

## Movement Model

The current motor implements:

- yaw-relative normalized wish direction
- HPL2/Amnesia-style local forward and side speed channels
- HPL2/Amnesia-style acceleration measured in speed-per-second, capped by per-axis target speed
- Amnesia/HPL2-style diagonal speed scaling
- Amnesia/HPL2-style opposite-direction acceleration multiplier
- separate forward/backward/side speed, acceleration, and deacceleration tuning
- Amnesia/HPL2-style ground contact hysteresis over tiny contact gaps
- HPL2-style horizontal movement pass before gravity
- Source/HL2-style horizontal blocker handling for non-step obstacles: sweep the
  cylinder, move to the hit fraction, collect blocking planes, clip velocity
  along those planes, and stop when the clipped motion turns against the
  original intent
- horizontal sweeps ignore floor/slope contacts using the configured
  `max_slope_angle` threshold and use a tiny lifted cast origin so ground
  contact cannot masquerade as a wall
- when normal horizontal movement loses forward progress against a climbable low
  riser, the controller retries from a raised pose, sweeps horizontally over the
  step, then casts down to a walkable landing within `max_step_height`; this
  avoids the visible bump-then-climb pause at stair faces
- HPL2-style slope-normal-aligned horizontal movement while grounded
- after horizontal movement, raycast-based step detection remains as a reactive
  fallback using the HPL2 algorithm: 1 or 3 rays from chest to feet in the
  movement direction
- stair candidates are accepted only when the detected height is within the
  configured step range and the full cylinder fits at the raised/forward pose;
  low-ceiling and over-height candidates stay blocked
- jump takeoff preserves the current grounded local speed, so sprint jumps
  carry farther than walk jumps before normal air control limits apply
- jump takeoff and coyote refresh require a walkable ground normal; ground
  steeper than `max_slope_angle` cannot be used as a jump surface
- `accurate_climbing` (default `false`) enables 3 rays instead of 1, matching
  HPL2's `AccurateClimbing` for wider lateral coverage
- `climb_forward_mul` (default `1.0`) scales forward position in the fit test,
  matching HPL2's `ClimbForwardMul`
- reactive fallback steps are lifted by `step_climb_speed * dt` each frame;
  raised step-up sweeps set the authoritative body on the landing immediately
  and rely on camera-only stair smoothing for presentation
- gravity is applied after the stair attempt and skipped on frames where step
  climbing or a raised step-up succeeds
- vertical force collision is resolved separately from horizontal movement, and
  vertical collision owns grounded state
- vertical force collision applies only vertical depenetration in the vertical
  phase; floor, ceiling, and gravity resolution cannot inject sideways solver
  correction
- ground probing only refreshes normals while already grounded; it does not
  magnet-snap airborne bodies into the floor
- hold-to-crouch by default, with opt-in Amnesia/HPL2 toggle crouch for games
  that want it
- feet-stable crouch/stand stance changes
- low-ceiling uncrouch rejection with real shape intersection tests, including
  HPL2's feet-position based standing-shape test and five tiny stand-up fit
  offsets: center, +/-1 cm X, +/-1 cm Z, all raised by 1 mm
- when uncrouch is rejected, local movement speed is clamped back to the
  actual crouching stance so low nooks cannot grant default walk speed
- with hold-to-crouch, a blocked uncrouch is retried automatically on later
  released frames; with `toggle_crouch = true`, retries match Amnesia and occur
  only on a new toggle, sprint auto-stand, or jump
- airborne movement uses Amnesia/HPL2 in-air speed caps; acceleration stays the
  same, and deacceleration in air is opt-in
- gravity and terminal fall speed
- Amnesia-style jump takeoff that first requests standing, then applies the
  configured vertical velocity
- latched jump input so the first observed down frame, including `Held`, can request one jump
- Amnesia-style timed jump gravity relief; release does not cancel the assist
- coyote time and jump buffering
- crouch/sprint target-speed changes
- Avian depenetration is used only as the HPL2-style pushback primitive; the
  controller no longer feeds horizontal movement, gravity, and stair climbing
  through one combined slide call

## Camera Model

The camera rig follows Amnesia/HPL2-style first-person presentation as a
separate layer over the physical body:

The headbob implementation lives in `controller/camera.rs` for camera state
integration and `controller/camera_motion.rs` for the HPL2 bob formula.

- horizontal camera position follows the controller body directly to avoid input-lag jitter
- bob, footstep speed, and sprint FOV use actual post-collision body velocity,
  not requested input speed, so pushing into a blocker does not animate motion
- grounded non-climbing vertical body movement follows directly so slopes do not feel delayed
- stair-climb vertical body movement is camera-smoothed until the residual step offset is below 2 mm, hiding discrete risers without delaying horizontal aim or collision
- crouch/stand eye height is feet-relative, not center-relative, so collision
  cylinder height changes do not double-apply the visible crouch animation
- crouch/stand eye height eases using Amnesia's head-offset slowdown curve,
  with faster engine defaults: crouch down `3.0`, stand up `3.6`, slowdown
  distance `0.05`
- walk, run, and crouch use Amnesia/HPL2 bob amplitudes: walk `0.03 0.03`, run `0.05 0.06`, crouch `0.06 0.04`
- bob phase speed is interpolated from movement speed using Amnesia/HPL2 min/max bob speeds: walk `0.4..1.8`, run `0.5..2.5`, crouch `0.2..1.2`
- current bob amplitude moves toward the target at `0.1` units per second, matching HPL2's `Vector2IncreaseTo` behavior
- footstep messages are generated from robust Amnesia/HPL2 lowest-bob phase
  crossings, including exact-boundary and large-frame crossings
- landing uses Amnesia/HPL2's two-phase ground-bounce curve, starting at
  minimum impact speed `5.0`; stronger impacts scale the same curve
- side movement does not auto-tilt the camera; Amnesia-style leaning should be driven explicitly through head offsets or scripted roll effects
- sprinting adds a smoothed FOV kick
- child `FirstPersonHeadOffset` components layer interaction, damage, scripted,
  and horror offsets
- child `FirstPersonCameraImpulse` components apply one-shot camera kicks

These are presentation effects only. Hit detection, networking, prediction, and
physics keep using the controller body and replicated gameplay state.

Moving-platform attachment, dynamic-body pushing, and full true-first-person
body animation remain deferred behind regression tests. The current motor keeps
collision/body state separate from local camera feel state so those features can
be added without changing the Leafwing action API.

## Demo

The demo room includes flat-floor movement space plus controller test geometry:
stairs with regular and near-limit riser heights, a too-high step barrier,
real wedge/prism ramps with flush floor entries, a too-steep ramp, and a
low-ceiling crouch tunnel for uncrouch rejection. The regular stair run is
placed directly in the spawn forward path so `W` tests stair climbing without
requiring a turn or sideways approach.

The FPS demo enables `FirstPersonControllerTrace` automatically and logs
diagnostic events under `afterglow::fps_controller_trace`. Reproduce controller
jitter from the demo and keep the terminal output: `body_collision`,
`body_pushback_flip`, `step_accepted`, `step_rejected`, and
`camera_offset_motion` point to the exact phase causing the visible motion.

Run the controller demo with:

```sh
bun run native -- --name fps-controller
```

or directly:

```sh
cargo run -p agx -- --name fps-controller
```
