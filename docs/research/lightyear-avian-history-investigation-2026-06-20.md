# Lightyear/Avian History Investigation — 2026-06-20

## Question

Can the official Lightyear + Avian integration history API solve the multiplayer
boxes presentation-query issue where camera, highlight, and rope gizmo systems
may select confirmed roots instead of predicted/presentation entities?

## Findings

### Official `lightyear_avian3d` has two different history mechanisms

1. **Lightyear prediction history**
   - Type: `lightyear_prediction::predicted_history::PredictionHistory<C>`
     (`HistoryBuffer<C>`).
   - Added automatically for registered predicted components.
   - Used internally by Lightyear to compare predicted state against confirmed
     state and trigger rollback/correction.
   - In the official Avian bridge, `AvianReplicationMode::Transform` schedules
     `PredictionSystems::UpdateHistory` after Avian writeback, so predicted
     `Transform` state is captured after physics.
   - Afterglow's Avian 0.6 fork already ports this Transform-mode ordering.

2. **Official Avian lag-compensation history**
   - Feature-gated behind `lightyear_avian3d/lag_compensation`.
   - Types: `LagCompensationPlugin`, `LagCompensationHistory`,
     `LagCompensationSpatialQuery`, `LagCompensationSystems`.
   - Records `(Position, Rotation, ColliderAabb)` each server tick and creates a
     child broad-phase AABB envelope collider.
   - Intended for server-side lag-compensated spatial queries/raycasting against
     the historical pose a client saw.
   - Requires server context (`Single<(), With<Server>>`) and Avian
     `Position`/`Rotation`/`ColliderAabb`.

## Applicability to multiplayer boxes

The remembered official plugin exists, but it solves a different problem than
presentation entity selection:

- **Useful for future gameplay queries**: server-side rewind/lag compensation,
  hitscan, melee traces, or interaction tests against historical collider poses.
- **Not a direct fix for current presentation queries**: camera follow,
  highlighting, and rope gizmos need to choose the correct live ECS entity copy
  (`Predicted`, `Interpolated`, not confirmed root), not rewind historical
  colliders.

For presentation, the correct built-in Lightyear markers are still `Predicted`,
`Interpolated`, `Confirmed`, and `PredictionDisable`, plus `FrameInterpolate` /
`VisualCorrection` for smoothing after the correct entity is selected.

## Current Afterglow fork status

`crates/afterglow-lightyear-avian3d` is a Transform-mode-only Avian 0.6 fork. It
already ports the official Transform-mode schedule ordering:

```text
FixedPostUpdate:
  PhysicsSystems::Prepare
  -> PhysicsSystems::StepSimulation
  -> PhysicsSystems::Writeback
  -> (PredictionSystems::UpdateHistory, FrameInterpolationSystems::Update)

PostUpdate:
  FrameInterpolationSystems::Interpolate
  -> RollbackSystems::VisualCorrection
  -> TransformSystems::Propagate
```

It does **not** currently port the official `lag_compensation` feature. Porting
that would be straightforward but should be treated as a separate gameplay
feature, not as the presentation-query fix.

## Recommendation

1. Fix the presentation-query issue with explicit presentation-space selectors:
   - local player camera target: local owner's `Predicted` copy;
   - box highlight target: client presentation/physics copy, currently
     `KinematicBox + Predicted`;
   - rope gizmo endpoints: active non-`PredictionDisable` rope link plus
     predicted/presentation player and box copies.
2. Add regression tests with confirmed + predicted duplicates present, proving
   camera/highlight/rope visuals do not select confirmed roots.
3. Later, port `LagCompensationPlugin` to `afterglow-lightyear-avian3d` if the
   engine needs server-side lag-compensated raycasts/interactions.

## Source references

- Official plugin: `lightyear_avian3d-0.26.4/src/plugin.rs`
- Prediction history: `lightyear_prediction-0.26.4/src/predicted_history.rs`
- History buffer: `lightyear_core-0.26.4/src/history_buffer.rs`
- Lag compensation history: `lightyear_avian3d-0.26.4/src/lag_compensation/history.rs`
- Lag compensation query: `lightyear_avian3d-0.26.4/src/lag_compensation/query.rs`
- Afterglow fork: `crates/afterglow-lightyear-avian3d/src/lib.rs`
