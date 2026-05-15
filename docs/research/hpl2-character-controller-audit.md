# HPL2 Character Controller Audit

This note records how Amnesia: The Dark Descent / HPL2 handles the player
controller and how Afterglow maps that design onto Avian. The intent is not a
generic FPS controller. The target feel is Amnesia first, with jump support
preserved because Amnesia does have a jump path.

## Primary Source Files

- HPL2 body core:
  `/tmp/amnesia-td-src/HPL2/core/sources/physics/CharacterBody.cpp`
- HPL2 body API:
  `/tmp/amnesia-td-src/HPL2/core/include/physics/CharacterBody.h`
- Amnesia normal move state:
  `/tmp/amnesia-td-src/amnesia/src/game/LuxMoveState_Normal.cpp`
- Amnesia move-state speed application:
  `/tmp/amnesia-td-src/amnesia/src/game/LuxMoveState.cpp`
- Amnesia player input/camera/head offsets:
  `/tmp/amnesia-td-src/amnesia/src/game/LuxInputHandler.cpp`,
  `/tmp/amnesia-td-src/amnesia/src/game/LuxPlayer.cpp`,
  `/tmp/amnesia-td-src/amnesia/src/game/LuxPlayerHelpers.cpp`

The source was cloned to `/tmp/amnesia-td-src`. Opencode agents were launched
with `--dangerously-skip-permissions` and separate source-root directories
after the first run hit external-directory restrictions.

## Collision Body

HPL2 creates a character body in `CharacterBody.cpp:244-281`. Radius is
`max(size.x, size.z) * 0.5`; almost-spherical bodies get a sphere, otherwise the
normal player gets a vertical cylinder. The physical body is massless, gravity
is disabled on the physics body, and custom character integration owns gravity,
movement, and collision callbacks.

Extra sizes are separate body/shape pairs (`CharacterBody.cpp:495-514`).
`SetActiveSize` (`CharacterBody.cpp:518-541`) preserves feet position when
swapping standing/crouching shapes.

Afterglow maps this to:

- `FirstPersonController` authoring creates Avian `Collider::cylinder` and
  `PhysicsCollider::cylinder`.
- `FirstPersonMotorState.stance` drives the active cylinder height.
- stance changes preserve feet via `feet_stable_center_delta`.
- standing from crouch tries HPL2's five tiny feet offsets: center, +/-X,
  +/-Z, with a 0.001 upward bias.

## Frame Schedule

HPL2 `iCharacterBody::Update` (`CharacterBody.cpp:948-1027`) is a strict custom
pipeline:

1. consume move delay
2. update body connection
3. clear gravity attachment
4. store last position
5. rebuild yaw-derived movement matrix
6. refresh previous step-climbing state
7. convert local speed channels to horizontal position add
8. project horizontal add onto the current ground normal
9. apply horizontal position add and horizontal pushback
10. run step-climbing check
11. update gravity/external force velocity
12. resolve force velocity as XZ then Y
13. apply external-force friction
14. update force pushing, gravity attachment, body, connection, camera, entity

The crucial ordering is horizontal movement before gravity. Stairs are checked
after normal horizontal movement and pushback, not before movement and not after
a combined slide that already mixed gravity into the sweep.

Afterglow's controller now follows the same shape:

- `drive_first_person_controllers` updates step-climbing state first.
- input only updates local HPL2-style speed channels and jump state.
- horizontal movement is applied before gravity. For normal non-step blockers,
  Afterglow now uses a Source/HL2-style shape sweep with plane clipping instead
  of applying Avian depenetration raw; this prevents tangent-axis solver
  correction from becoming visible player motion. When the HPL2 step ray already
  proves the obstacle is climbable, Afterglow keeps HPL2 target-pose overlap
  movement so Amnesia's gradual `step_climb_speed` lift remains intact.
- the stair attempt runs immediately after horizontal pushback.
- gravity is applied after the stair attempt and skipped during accepted climb.
- vertical force collision is resolved separately and owns grounded state.
- ground ray probing only refreshes normals while already grounded.

## Local Movement

HPL2 local movement is in `CharacterBody.cpp:1411-1532`. It has separate
forward/right channels, per-channel max positive/negative speeds, acceleration,
deacceleration, and optional diagonal scaling by `0.7071`. Movement deaccelerates
only on ground unless `mbDeaccelerateMoveSpeedInAir` is enabled. Opposite-
direction acceleration multipliers are applied only while grounded.

Amnesia applies normal/run/crouch/air speed multipliers in
`LuxMoveState_Normal.cpp:484-533`, then writes them to the body in
`LuxMoveState.cpp:73-82`.

Afterglow maps this to:

- `forward_speed` and `side_speed` in `FirstPersonMotorState`
- `ground_speed`, `backward_speed`, `side_speed`, `sprint_speed`,
  `crouch_speed`, and `air_wish_speed`
- `ground_accel`, `side_accel`, `ground_deaccel`, `side_deaccel`
- diagonal scaling with `FRAC_1_SQRT_2`
- grounded-only opposite-direction acceleration multipliers
- optional `deaccelerate_in_air`

Afterglow intentionally does not expose a Quake-style `air_accel` or generic
ground friction knob for local movement. HPL2's `UpdateFriction`
(`CharacterBody.cpp:1737-1764`) applies to external force velocity and gravity
attachment velocity, not the normal local speed channels.

## Jump

Amnesia has jump support. Input enters through `LuxInputHandler.cpp:1107-1109`
and `LuxPlayer.cpp:759-768`, then reaches `cLuxMoveState_Normal::OnJump`
(`LuxMoveState_Normal.cpp:308-320`). The jump starts only when the button is
pressed, the move state is not already in its jump-assist window, and the
character body reports grounded.

`Jump` (`LuxMoveState_Normal.cpp:324-338`) first tries to stand by calling
`SetCrouch(false)`, plays the jump sound, selects normal or crouch start force,
multiplies by the script jump-force multiplier, applies upward force, and starts
the jump-assist timer.

The assist itself is timer-based, not release-sensitive. In
`LuxMoveState_Normal.cpp:554-571`, while `mbJumping` is true, Amnesia increments
`mfJumpCount`, stops at `mfMaxJumpCount`, and adds upward force that cancels a
shrinking fraction of gravity from roughly `0.9` down to `0.4`.

Afterglow maps this to:

- `jump_enabled` for modes that should ignore jump commands
- jump input latch so a first observed `Pressed` or `Held` requests one jump
- coyote and buffer windows as usability additions around the Amnesia base
- jump requires a walkable ground normal
- jump first requests standing, then applies `jump_speed`
- `jump_hold_ticks` is a timed assist window; release does not cancel it
- `jump_hold_gravity_relief_start` / `end` default to `0.9` / `0.4`

## Crouch And Auto-Stand

Amnesia crouch is toggle-based, not hold-based:
`LuxInputHandler.cpp:1111-1113` and `LuxMoveState_Normal.cpp:298-304`.

`SetCrouch` (`LuxMoveState_Normal.cpp:356-414`) switches to the crouch shape
immediately. When standing, it tests the standing shape with
`CheckCharacterFits(feet + offset, true, 0)` using five fit positions:
center, +/-1 cm X, and +/-1 cm Z, all raised by 1 mm. If one fits,
`SetActiveSize(0)` preserves the accepted feet position while changing the
center height. If none fit, it remains crouched. Head offset moves down with
speed `1.3` and up with speed `1.6`, both using slowdown distance `0.05`.

Running while grounded and moving asks the player to stand:
`LuxMoveState_Normal.cpp:280-291` and `453-480`.

Afterglow maps this to:

- crouch command is hold-to-crouch by default for better engine/demo DX
- `FirstPersonControllerConfig::toggle_crouch` restores Amnesia's toggle
  behavior when a game wants it
- failed uncrouch resets `desired_stance` to crouching
- failed uncrouch clamps local speeds back to the actual crouched stance
- hold-to-crouch retries blocked uncrouch on later released frames; toggle
  mode matches Amnesia's no-automatic-retry behavior
- sprint while grounded and moving asks to stand, with the same clearance path
- camera eye height uses Amnesia's slowdown curve, but faster Afterglow
  defaults: `3.0` down / `3.6` up / `0.05` slowdown

## Grounding And Slopes

HPL2 grounding is hysteretic. Constructor defaults use
`mlMaxOnGroundCount = 12` (`CharacterBody.cpp:355-356`), and `IsOnGround`
checks whether the counter is positive (`CharacterBody.cpp:1067-1070`).
Vertical collision refreshes the counter (`CharacterBody.cpp:1824-1852`).

Slope behavior is split:

- horizontal movement is projected along the ground plane while grounded and not
  moving upward (`CharacterBody.cpp:1536-1549`)
- vertical force collision treats shallow no-slide slopes as world-up when
  removing vertical velocity (`CharacterBody.cpp:1861-1882`)
- fallback ray normal refresh only happens while already grounded and not
  climbing (`CharacterBody.cpp:1893-1911`)

Afterglow maps this to:

- `ground_sticky_ticks = 12`
- horizontal ground projection before horizontal pushback
- vertical collision alone refreshes grounded state
- fallback ground probe cannot snap airborne bodies down
- walkable jump/ground tests use `max_slope_angle`

## Stairs

HPL2 step climbing is in `CharacterBody.cpp:1618-1707`.

Key details:

- check interval defaults to `1 / 20`
- forward ray distance is `max(horizontal_delta_length, 0.05)`
- accurate climbing optionally casts three rays
- valid step height is above `0.025` and below current max step height
- current max is full step height only when firmly grounded or already climbing
- fit test raises the body by step height plus `0.01`
- accepted climb raises Y by `step_climb_speed * dt`
- climbing refreshes grounded state and suppresses gravity that frame

Afterglow now uses HPL2's exact raycast algorithm:

- rate-limited by the same `1/20` interval
- 1 or 3 rays (``accurate_climbing`) from chest height down to feet
- step height validated against `min_step_height` / `max_step_height`
- shape fit test at raised position before lifting
- direct lift by `step_climb_speed * dt` each climbing frame (reactive, no
  accumulator)
- `step_climb_speed` defaults to `1.0` (HPL2 constructor fallback)
- `climb_forward_mul` defaults to `1.0` (HPL2 `mfClimbForwardMul`)
- gravity suppressed while climbing
- grounded state forced during climbing
- `acurrate_climbing` defaults to `false` (single center ray)

The earlier Jolt-style sweep validation was removed in favour of the exact HPL2
algorithm for behavioural fidelity.

## Camera And Effects

Amnesia's player camera is split between the body camera offset and move-state
effects.

`LuxPlayer.cpp:1193-1233` accumulates named head-position offsets and moves each
offset toward its goal with speed and slowdown distance. `CharacterBody.cpp:
1996-2056` then puts the camera at the body position plus standing-height
compensation and the accumulated local offset.

This means crouch presentation is effectively feet-relative. `SetActiveSize`
preserves feet while the body center changes, and `UpdateCamera` adds
`standing_height - current_height / 2` before applying animated head offsets.
Afterglow mirrors this by placing the camera at body center plus
`eye_height - current_body_height / 2`, so the collision cylinder height change
does not double-apply the visible crouch.

Normal movement camera effects live in `LuxMoveState_Normal.cpp:576-700`:

- walk bob max `0.03, 0.03`
- run bob max `0.05, 0.06`
- crouch bob max `0.06, 0.04`
- walk bob speed `0.4..1.8`
- run bob speed `0.5..2.5`
- crouch bob speed `0.2..1.2`
- bob amplitude increases toward the target by `0.1 * dt`
- bob vertical formula is `sin(phase) * y - y`
- bob lateral formula is `sin(phase / 2 - pi / 4) * x`
- footsteps fire at the lowest bob crossing
- landing bounce is a two-phase sine/smooth-curve waveform

Landing starts in `LuxMoveState_Normal.cpp:40-56` when a downward force velocity
exceeds `MinHitGroundBounceSpeed`. Fall damage can scale bounce size/speed in
`LuxMoveState_Normal.cpp:868-901`.

Afterglow maps this to:

- local camera rig separate from gameplay collision state
- Amnesia bob amplitudes and min/max speeds
- amplitude changes by fixed `0.1 * dt`
- Amnesia bob formulas and footstep phase crossing
- no automatic side-strafe tilt
- landing bounce uses the same two-phase curve
- stronger impacts scale the same bounce curve until fall-damage tiers exist
- script/damage/horror offsets are represented as `FirstPersonHeadOffset`
- one-shot camera kicks use `FirstPersonCameraImpulse`

Lean and full fall-damage tier integration are still separate features. The
core camera offset architecture is compatible with both.

## Remaining Gaps

These are known omissions, not accidental behavior:

1. Dynamic-body pushing and gravity-attachment are not implemented yet. HPL2 has
   body pushing (`CharacterBody.cpp:129-197`, `1924-1991`) and moving-platform
   attachment (`CharacterBody.cpp:2143-2248`).
2. Fall damage events are not implemented. Landing bounce exists, but health
   damage and damage-tier bounce multipliers need gameplay messages.
3. Lean is represented by generic head offsets and impulses, but Amnesia's
   collision-aware lean helper (`LuxPlayerHelpers.cpp:2327-2494`) is not copied
   yet.
4. Ledge climbing is intentionally skipped for now. Amnesia's normal move state
   disables it with an early return in `LuxMoveState_Normal.cpp:705-710`.
