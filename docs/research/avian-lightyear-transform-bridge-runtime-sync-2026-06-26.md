# Avian 0.6 + Lightyear Transform Bridge Runtime Sync Investigation

Date: 2026-06-26

## Question

Live `multiplayer_boxes` clients showed stale visible movement even after player
and block authoritative state changed. A plausible root-cause hypothesis was that
`afterglow-lightyear-avian3d` scheduled `TransformToPosition` after physics and
therefore overwrote solver-updated `Position` before `PositionToTransform` could
write the replicated `Transform`.

## Findings

- Avian 0.6's default physics schedule is `FixedPostUpdate`, not the ordinary
  Bevy `FixedUpdate` schedule. The bridge's Transform-mode ordering therefore
  runs in the same schedule family as Avian's step.
- The bridge regression
  `afterglow-lightyear-avian3d::tests::transform_mode_writes_physics_position_back_to_transform`
  installs Avian with `PhysicsTransformPlugin` and `PhysicsInterpolationPlugin`
  disabled, installs `AfterglowAvianPlugin`, gives a dynamic body
  `LinearVelocity`, advances fixed ticks, and asserts both Avian `Position` and
  Bevy `Transform` move and match. This passed.
- The stale visible sync reproduced in the two-client harness when authoritative
  server `Transform` updates reached Lightyear's `Confirmed<Transform>` but the
  client-facing `Predicted` copy's visible `Transform` remained at the spawn
  pose. The failing assertion read the actual client `Transform`, not
  `Confirmed<Transform>`.
- The implemented fix is demo-side presentation/physics reconciliation for
  remote predicted entities: when `Confirmed<Transform>` changes,
  `sync_remote_predicted_confirmed_transforms` copies the confirmed pose to
  non-local predicted `PlayerBox`es and predicted `KinematicBox`es, mirrors
  Avian `Position`/`Rotation`, and mirrors `Confirmed<LinearVelocity>` when
  present. Locally owned players remain input-predicted.
- Rope physics is now authority-scoped: the server and owning predicted client
  create `DistanceJoint`s; non-owning clients keep `RopeLink` visual-only and
  follow confirmed poses. This avoids divergent remote rope constraints fighting
  confirmed corrections.

## Schedule Caveat

In Transform replication mode, game systems should prefer physics components
(`LinearVelocity`, forces, impulses, joints) for physics-active entities. Direct
`Transform` writes on bodies with Avian `RigidBody` are treated as authoritative
pose edits and are synchronized back into Avian `Position`; they can bypass or
replace constraint output for that tick. This is acceptable for server-authored
pose-replication tests but should not be used for normal rope/contact gameplay.

## Verification

Focused commands run in `nix develop`:

```bash
cargo test -p afterglow-lightyear-avian3d transform_mode_writes -- --nocapture
cargo test -p afterglow-engine --features test-support -- demos::multiplayer_boxes -- --nocapture
cargo test -p engine-rpg-harness multiplayer_boxes -- --nocapture
```

Results: all focused tests passed. The harness tests run one server plus two
clients with the Avian/Lightyear bridge installed and assert visible client
`Transform`s for player input movement, server block movement, and stable rope +
player/block movement.
