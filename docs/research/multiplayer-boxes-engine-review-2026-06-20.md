# Multiplayer Boxes / Engine Correctness Review — 2026-06-20

Ten read-only `opencode-go/deepseek-v4-flash` reviewers audited the current
working tree against Bevy, Lightyear, Leafwing, Avian, and Replicon-style
networking best practices. This file is the human synthesis; raw subagent logs
were treated as temporary research output.

## Overall Verdict

The direction is now much cleaner than the earlier rope-intent design: player
input is Lightyear/Leafwing `ActionState`, rope state is replicated gameplay
state, render assets stay local, and player/cube prediction is mostly aligned
with Lightyear patterns.

The review still found several actionable correctness and hardening items. The
highest-priority items are around rope `PreSpawned` identity, Lightyear test
lifecycle, presentation query filters, and controlled-link ordering.

## Highest Priority Findings

| Priority | Area | Finding | Recommended next action |
|---|---|---|---|
| Fixed 2026-06-20 | Rope `PreSpawned` matching | `rope_id_for_input(owner, target, tick)` used local processing tick. In client/server mode the predicted client tick and authoritative delayed-input server tick can differ by `input_delay_ticks`, so the client `PreSpawned` hash may not match the server confirmation. | Fixed by deriving rope id/hash from owner + target only and adding `rope_id_is_stable_across_client_server_input_delay_tick_offsets`. |
| Major | Rope rollback/lifecycle | Rope attach/detach produces entity lifecycle commands from `just_released`. Reviewers disagree on whether to guard during rollback; blindly skipping gameplay replay may be wrong, but duplicate predicted spawns/despawns during rollback need a test. | Add rollback replay tests for rope attach and detach. Decide whether idempotent deterministic spawn/despawn is sufficient or whether a rollback-specific guard is needed. |
| Major | Presentation queries | Camera, highlight, and rope gizmo queries often use raw `PlayerBox`/`KinematicBox` without excluding confirmed roots or selecting predicted/presentation copies. This can choose confirmed roots or duplicate copies depending on Lightyear lifecycle. | Add explicit filters/helpers for presentation-space entities: local/remote player presentation, box presentation, and rope-physics participants. Add tests with confirmed + predicted copies present. |
| Major | Dedicated server role | Several multiplayer-boxes systems are gated for `Host | Client` but not `Server`. If `LightyearRole::Server` is intended to work for the demo, it will not spawn scene/player state or run movement/rope. | Either document multiplayer-boxes as Host/Client only, or include Server in authoritative scene/movement/rope systems. |
| Major | Lightyear test lifecycle | Some manual Bevy tests install Lightyear plugins but do not call `finish()` / `cleanup()`. Some use fixed retry loops for `plugins_state()` instead of a while loop. | Audit all Lightyear-using test apps. Call `finish()` / `cleanup()` before updates; replace fixed plugin-state loops with the documented while loop. |

## Medium Priority Findings

| Area | Finding | Recommended next action |
|---|---|---|
| Fixed 2026-06-20 | `apply_movement` no longer had an explicit `With<PlayerBox>` filter. It was harmless for known entities because non-players lack `ActionState`, but broader than intended. | Restored `With<PlayerBox>` and added `non_player_predicted_entities_are_not_moved_by_player_input_system`. |
| Player prediction/interpolation | Current fix predicts players to all clients and disables interpolation targets. This avoids lifecycle spam and gives responsive contacts, but remote player presentation is now predicted rather than interpolated. | Keep as a demo choice for now. If interpolated remote visuals are desired later, split remote physics proxies from visible interpolated presentation rather than targeting one entity as both. |
| Static arena replication | Server arena walls/floor are replicated while clients also spawn local arena visuals/colliders. Because render/physics components are not registered, replicated arena entities are mostly dead logical transforms. | Stop replicating static arena visuals/colliders if clients author them locally, or replicate only a small logical arena descriptor. |
| Controlled ownership | `ControlledEntityPlugin` depends on `MemberLinkMap` being initialized by the Lightyear bridge and has no explicit ordering with `update_member_link_map`. Stale entries may exist briefly during leave/despawn. | Initialize `MemberLinkMap` in `ControlledEntityPlugin` too, add explicit system ordering, and clear map entries on leave/session-ended paths. |
| Frame/update costs | `ensure_replication_channels` and `update_member_link_map` scan every frame. This is fine now but can be optimized. | Consider observer/on-add driven setup once behavior is stable. |
| Asset churn | Visual attachment creates mesh/material assets on first attach. Player visuals have a guard, kinematic visuals use `Without<Mesh3d>`. Rollback removing non-replicated components could recreate assets. | Add explicit visual-attached guard components for all local presentation bundles that should survive or be restored deliberately. |
| Tests with weak assertions | Some harness tests assert only "no panic" or `> 0` while names imply stricter behavior. | Strengthen despawn, edge, leave, reconnect, and packet-loss tests. |
| Test runtime/flakiness | Some UDP tests use sleeps inside per-app/per-frame loops and dynamic port allocation with TOCTOU race. | Reduce sleeps, centralize timing budgets, and add retry-on-bind failure or port reservation strategy. |

## Lower Priority / Technical Debt

- Several non-demo files remain over the 500 LOC project rule, including
  `crates/delta/src/lib.rs`, `engine-rpg-harness` scenario files, and some legacy
  mock/test files. These should be split in cleanup work.
- `delta-lightyear` was named like a Lightyear integration crate but did not
  depend on Lightyear; both `delta` and `delta-lightyear` have since been
  retired as orphaned crates.
- `crates/afterglow-engine/src/lib.rs` exposes demo runner APIs from the engine
  crate; consider moving demo launching to `agx` or a demo crate.
- `docs/api/network.md` and some research docs contain stale test counts and old
  examples that should be refreshed after the current networking changes.
- The static arena and visual presentation architecture would benefit from a
  small documented "logical state vs local presentation" helper pattern.

## Findings To Validate Before Acting

Some reviewer findings are plausible but require deeper Lightyear/Leafwing
validation before changing code:

- **Rollback guard on gameplay consumers:** Input writers must be rollback
  guarded, but gameplay consumers normally must replay during rollback. Rope
  spawns/despawns need idempotence tests before adding a blanket rollback skip.
- **`PredictionDisable` as active/inactive filter:** It is a Lightyear lifecycle
  marker, not a gameplay rope state. It is still reasonable for local effects to
  ignore prediction-disabled entities. Add tests before introducing another
  local gameplay marker.
- **`just_released` across multiple fixed steps:** Leafwing fixed-state handling
  may already avoid repeated edges. Add a test simulating multiple fixed ticks in
  one render frame before reintroducing any custom latch.
- **Replicon:** Replicon is not used. Reviewers did not find a reason to migrate;
  current architecture should stay Lightyear-native.

## Positive Confirmations

- The old parallel `RopeIntent` command path is gone; rope now derives from
  input + world state.
- `ActionState` remains pure input-device state.
- Lightyear `InputChannel` is owned by the native input plugin, not manually
  re-registered.
- Netcode client link no longer pre-inserts `LocalId`/`RemoteId` before the
  handshake.
- `Transform` interpolation registration uses `.add_interpolation_with(...)`,
  not only `InterpolationRegistry::set_interpolation`.
- Render assets are not replicated as protocol components.
- The multiplayer boxes demo now avoids overlapping player prediction and
  interpolation targets on the same receiver.

## Suggested Next Work Order

1. Add/fix tests for rope `PreSpawned` hash matching under input delay; then fix
   `rope_id_for_input` if the test fails.
2. Add presentation filters for camera/highlight/rope gizmo and tests with
   confirmed + predicted copies present.
3. Harden Lightyear test app lifecycle (`finish()` / `cleanup()`) and weak
   assertions.
4. Decide/document whether `LightyearRole::Server` supports multiplayer boxes.
5. Clean controlled-link resource initialization/ordering.
6. Split oversized files and refresh docs/test counts.
