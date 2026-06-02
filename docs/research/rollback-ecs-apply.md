# Applying RunDelta to Bevy ECS Components

**Date:** 2026-05-19
**Status:** Historical physics-snapshot research only. The current networking
baseline does not apply `RunDelta` snapshots to ECS for rollback. It uses fixed
server input delay, deterministic fixed-tick simulation, and Lightyear
reconciliation.

## Problem

Server sends a `RunDelta` for frame T. We have cached bytes for frame T-1. We need to update the Bevy ECS — `Transform`, `LinearVelocity`, `AngularVelocity`, and joint state — for the relevant entities.

## Data Model

Each physics entity carries a `PhysicsIndex(u32)` component that matches its position in the `PhysicsSnapshot.bodies` array:

```rust
#[derive(Component)]
struct PhysicsIndex(u32);

commands.spawn((
    PhysicsIndex(i),
    RigidBody::Dynamic,
    Transform::from_xyz(...),
    LinearVelocity(...),
    AngularVelocity(...),
));
```

A `PhysicsEntityMap` resource maintains the reverse lookup for O(1) access:

```rust
#[derive(Resource)]
struct PhysicsEntityMap(Vec<Entity>);
```

## Full Apply (every body)

Deserialize the full snapshot, iterate over all bodies, write to ECS.

```rust
fn apply_full_snapshot(
    map: Res<PhysicsEntityMap>,
    snapshot: PhysicsSnapshot,
    mut transforms: Query<&mut Transform>,
    mut linvels: Query<&mut LinearVelocity>,
    mut angvels: Query<&mut AngularVelocity>,
) {
    for body in snapshot.bodies {
        let entity = map.0[body.index as usize];
        if let Ok(mut t) = transforms.get_mut(entity) {
            t.translation = Vec3::from_array(body.translation);
            t.rotation = Quat::from_array(body.rotation);
        }
        if let Ok(mut v) = linvels.get_mut(entity) {
            v.0 = Vec3::from_array(body.linvel);
        }
        if let Ok(mut a) = angvels.get_mut(entity) {
            a.0 = Vec3::from_array(body.angvel);
        }
    }
}
```

Cost: `1× postcard::from_bytes + N × (3× get_mut + memcpy)`.
At 10k bodies: ~160 μs deser + ~30 μs ECS writes = ~190 μs.

## Sparse Apply (only changed bodies)

The `RunDelta.runs` tell us exactly which byte ranges changed. If we know the per-body serialized size, we can map runs back to body indices. But postcard's varint encoding for `BodySnapshot.index` makes per-body size variable (`index < 128`: 1 byte varint; `index >= 128`: 2 bytes).

Workaround: serialize the bodies Vec at a known fixed offset. The first body starts at:

```
postcard header:  ~1 byte  (Vec length prefix for PhysicsSnapshot.bodies)
body 0:           variable (depends on BodySnapshot.index value)
```

For deterministic sizes, use a custom `SnapshotBody` with all `u32` fields using fixed-width encoding. Or simpler: just deserialize the full snapshot — the deserialize cost (~160 μs) already dominates the ECS write cost.

The optimization path without per-body size tracking: use the `RunDelta` as a hint to skip the `for body in snapshot.bodies` loop for bodies that didn't change. This requires knowing which body index a given offset range maps to.

**Simpler sparse approach**: store a `changed_flags: Vec<bool>` alongside the snapshot. The first phase of snapshot generation records which indices changed. The delta carries these index flags separately from the byte diff.

## Recommended Architecture for Rollback

```
                         ┌──────────────────────┐
                         │   RunDelta from server │
                         └──────┬───────────────┘
                                │
                                ▼
                    ┌──────────────────────┐
                    │ apply_bytes          │  ~0.2 μs per delta
                    │ (memcpy into cached  │
                    │  frame T-1 bytes)    │
                    └──────┬───────────────┘
                           │
                           ▼
                    ┌──────────────────────┐
                    │ postcard::from_bytes │  ~160 μs
                    │ → PhysicsSnapshot    │
                    └──────┬───────────────┘
                           │
                           ▼
                    ┌──────────────────────┐
                    │ for each body:       │  ~30 μs (10k bodies)
                    │   get_mut(entity)    │
                    │   write Transform    │
                    │   write LinVel       │
                    │   write AngVel       │
                    └──────┬───────────────┘
                           │
                           ▼
                    ┌──────────────────────┐
                    │   Resim N ticks      │
                    │   deterministic      │
                    └──────────────────────┘
```

Total per rollback (10k bodies, max 240 deltas):
- 48 μs chain-apply deltas
- 160 μs deserialize
- 30 μs ECS writes
- **~238 μs before resimulation starts**

## Joint Handling

Joints reference bodies by `body1_index`/`body2_index`. On first spawn, store a `JointEntities { body1: Entity, body2: Entity, kind: JointKind }` component or maintain a `Vec<JointEntity>` resource. On snapshot apply, joints only need updating if their referenced bodies' transforms changed — but since joints in Avian are ECS components referencing `Entity`, they update automatically when the bodies move.

If joints need re-creation (e.g., on full snapshot restore), iterate the `joints` vec, look up each `bodyX_index` in `PhysicsEntityMap`, and spawn/replace the joint component.
