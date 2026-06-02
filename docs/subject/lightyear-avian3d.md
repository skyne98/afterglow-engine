# lightyear_avian3d v0.26.4 — Definitive Reference

**Repository**: <https://github.com/cBournhonesque/lightyear>
**Crate**: `lightyear_avian3d` 0.26.4 · `avian3d` 0.5 · `bevy` 0.18 · `lightyear` 0.26.4
**MSRV**: 1.88 · **License**: MIT / Apache-2.0

---

## 1. Overview

`lightyear_avian3d` is the official bridge crate between the Lightyear networking
framework and the Avian physics engine. It solves three problems:

1. **Replicated physics** — Syncs Avian's `Position`/`Rotation` (or `Transform`)
   over the network with Lightyear's prediction, interpolation, and rollback
   systems, including schedule ordering so physics state is sampled at the
   correct point in the frame.

2. **Lag compensation on the server** — Records historical `(Position, Rotation,
   ColliderAabb)` per tick so server-side raycasts can be rewound to the pose
   the client saw when it fired. Uses Avian's `SpatialQuery` for broad-phase,
   with its own narrow-phase for exact historical intersection.

3. **Visual correction after rollback** — When predicted state is corrected,
   smoothly blends the visual `Transform` from the old predicted value to the
   new correct value using `FrameInterpolation` and `VisualCorrection` components.

A companion `lightyear_avian2d` crate mirrors this API for 2D games; both share
the same source tree behind `cfg(feature = "2d")` / `cfg(feature = "3d")`.

---

## 2. Feature Catalog

| Feature | What it enables | Default? |
|---------|----------------|----------|
| `3d` | `types_3d` module (lerp/hash for Position, Rotation, LinearVelocity, AngularVelocity), `plugin.rs` with 3d imports, `correction_3d` | Yes |
| `2d` | `types_2d` module, `plugin.rs` with 2d imports, `correction_2d` | No |
| `lag_compensation` | `lag_compensation/` module (LagCompensationPlugin, history, query) | No |
| `deterministic` | Hash functions using `seahash` for Position/Rotation (enables `DeterministicHashing` in Lightyear's prediction) | No |
| `f32` | `avian3d/f32` scalar type | Yes (via `avian3d/parry-f32`) |
| `std` | `lightyear_prediction/std` (std support for prediction) | Yes |

**Default feature set**: `["std", "3d", "avian3d/parry-f32"]`

**Realistic usage for a 3D deterministic server**:
```toml
lightyear_avian3d = { version = "0.26.4", features = ["3d", "lag_compensation", "deterministic"] }
```

---

## 3. Module Map

### `lib.rs` — Root

```rust
#![no_std]  // requires explicit #[cfg(feature = "std")] extern crate std
```

- Re-exports `types_3d as types` (or `types_2d as types`).
- Gated modules:
  - `#[cfg(feature = "lag_compensation")] pub mod lag_compensation`
  - `#[cfg(any(feature = "2d", feature = "3d"))] pub mod plugin`
  - `#[cfg(feature = "2d")] mod correction_2d` (private)
  - `#[cfg(feature = "3d")] mod correction_3d` (private)
- `pub mod prelude` — re-exports `LightyearAvianPlugin` and all lag-compensation types.

### `plugin.rs` — Plugin Setup & Schedule Wiring

Public types: `LightyearAvianPlugin`, `AvianReplicationMode`.

Private helpers: `sync_transform_to_position`, `sync_position_to_transform`,
`add_transform`, `update_child_collider_position`.

**Key design**: This plugin must be added manually — it is NOT added by
`ClientPlugins`/`ServerPlugins`. It configures schedule ordering per
replication mode and wires the Position↔Transform syncs that Avian normally
handles, but that must be intercepted when components are replicated.

### `types_3d.rs` — Lerp & Hash Functions

Each submodule exposes a `pub fn lerp(start: &T, other: &T, t: f32) -> T`.
With `deterministic`, also `pub fn hash(value: &T, hasher: &mut SeaHasher)`.

| Submodule | Type | Lerp method |
|-----------|------|-------------|
| `position` | `Position` | Linear: `start.0 * (1-u) + other.0 * u` |
| `rotation` | `Rotation` | `start.slerp(*other, t)` — spherical linear |
| `linear_velocity` | `LinearVelocity` | Linear |
| `angular_velocity` | `AngularVelocity` | Linear |

`types_2d.rs` mirrors this; rotation lerp in 2D wraps shortest angle via
`(((diff % 360) + 540) % 360) - 180`.

### `correction_3d.rs` — Post-Rollback Visual Correction (Private)

Two systems wired automatically in `PositionButInterpolateTransform` mode:

- **`update_frame_interpolation_post_rollback`** — Runs in `PreUpdate →
  RollbackSystems::EndRollback`. Reads corrected Position/Rotation and their
  `PredictionHistory`, computes the "before-last" correct transform, updates
  `FrameInterpolate<Transform>`, computes `VisualCorrection<Isometry3d>` error.
  Uses `to_transform(pos, rot)` which sets `scale: Vec3::ONE` (TODO: handle scale).

- **`add_visual_correction`** — Runs in `PostUpdate →
  RollbackSystems::VisualCorrection`. Applies `VisualCorrection<Isometry3d>` to
  `Transform` via `bypass_change_detection().apply_diff()`. Decays the error
  using `EasingCurve::linear(identity, error)` sampled at
  `correction_policy.lerp_ratio`. Removes the correction component when error
  is small enough.

### `lag_compensation/mod.rs` — Module Root

Re-exports `history` and `query`.

### `lag_compensation/history.rs` — History Recording & AABB Envelope

Public types: `LagCompensationPlugin` (unit struct), `LagCompensationConfig`,
`LagCompensationHistory` (type alias), `AabbEnvelopeHolder` (marker component),
`LagCompensationSystems` (system set enum).

### `lag_compensation/query.rs` — Historical Spatial Query

Public type: `LagCompensationSpatialQuery` (`SystemParam`, `ReadOnlySystemParam`).

Methods: `cast_ray`, `cast_ray_predicate`.

---

## 4. Plugin Setup

### `LightyearAvianPlugin`

```rust
pub struct LightyearAvianPlugin {
    pub replication_mode: AvianReplicationMode,  // default: Position
    pub update_syncs_manually: bool,              // default: false
    pub rollback_resources: bool,                 // default: false
    pub rollback_islands: bool,                   // default: false
}
```

### `AvianReplicationMode`

| Variant | Replicates | Correction + Interpolation target | Use case |
|---------|-----------|-----------------------------------|----------|
| `Position` (default) | `Position` | `Position` (FrameInterpolation + PredictionHistory on Position) | Pure physics replication; users operate on Position directly |
| `PositionButInterpolateTransform` | `Position` | Correction + FrameInterpolation on `Transform` | Serialize Position (smaller), but visual blending on Transform |
| `Transform` | `Transform` | Correction + FrameInterpolation on `Transform` | Traditional transform-based networking |

#### Schedule Ordering — Position Mode

```text
RunFixedMainLoop:
  PhysicsSystems::Prepare (TransformToPosition)
    .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
    .before(FrameInterpolationSystems::Restore)

FixedPostUpdate:
  PhysicsSystems::StepSimulation
    →
  (PredictionSystems::UpdateHistory, FrameInterpolationSystems::Update)

PostUpdate (after FixedPostUpdate runs):
  FrameInterpolationSystems::Interpolate
    →
  RollbackSystems::VisualCorrection
    →
  PhysicsSystems::Writeback (PositionToTransform)
    →
  TransformSystems::Propagate
```

#### Schedule Ordering — PositionButInterpolateTransform Mode

```text
PreUpdate:
  RollbackSystems::EndRollback
    (correction::update_frame_interpolation_post_rollback added here)

RunFixedMainLoop:
  PhysicsSystems::Prepare (TransformToPosition)
    .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
    .after(FrameInterpolationSystems::Restore)

FixedPostUpdate:
  PhysicsSystems::StepSimulation
    →
  PredictionSystems::UpdateHistory
    →
  (PhysicsSystems::Writeback, FrameInterpolationSystems::Update).chain()

PostUpdate:
  FrameInterpolationSystems::Interpolate
    →
  RollbackSystems::VisualCorrection
    (correction::add_visual_correction added here)
    →
  TransformSystems::Propagate
```

Also registers `TransformLinearInterpolation::lerp` in `InterpolationRegistry`
for `Transform` (even though Transform is not replicated).

#### Schedule Ordering — Transform Mode

```text
FixedPostUpdate:
  PhysicsSystems::Prepare (TransformToPosition)
    →
  PhysicsSystems::StepSimulation
    →
  PhysicsSystems::Writeback (PositionToTransform)
    →
  (PredictionSystems::UpdateHistory, FrameInterpolationSystems::Update)

PostUpdate:
  FrameInterpolationSystems::Interpolate
    →
  RollbackSystems::VisualCorrection
    →
  TransformSystems::Propagate
```

Also adds `update_child_collider_position` in `FixedPostUpdate` →
`PhysicsTransformSystems::PositionToTransform` → before `position_to_transform`.

### `rollback_resources` and `rollback_islands`

When `rollback_resources: true`:
- Registers `ContactGraph`, `ConstraintGraph` for resource rollback.
- Registers `CollidingEntities` for component rollback.

When `rollback_islands: true` (implies `rollback_resources: true`):
- Also registers `PhysicsIslands` (resource rollback), `BodyIslandNode` and
  `Sleeping` (component rollback).

These are needed for deterministic (input-based) replication where the full
physics state is rolled back. For state-based replication they are optional.

### Recommended Avian Plugin Config

```rust
PhysicsPlugins::default()
    .build()
    .disable::<PhysicsTransformPlugin>()     // handled by lightyear_avian
    .disable::<PhysicsInterpolationPlugin>() // FrameInterpolation handles this
    // .disable::<IslandPlugin>()            // only for deterministic replication
    // .disable::<IslandSleepingPlugin>()    // only for deterministic replication
```

### `update_child_collider_position` (public helper)

Updates child collider `Position`/`Rotation` after physics runs. In Avian this
is normally done in `PhysicsSystems::First`, so the bridge re-does it after
physics to ensure accurate replication. It queries `ColliderTransform`,
`ColliderOf`, and the parent rigidbody's `Position`/`Rotation`, computing:

```rust
position.0 = rb_pos.0 + rb_rot * collider_transform.translation;
rotation = (rb_rot.0 * collider_transform.rotation.0).normalize().into();
```

### `add_transform` (auto-adds Transform when Position+Rotation added)

A query-based system that detects entities with `Position` + `Rotation` but
without `Transform`, and inserts the correct derived `Transform`. Handles
parenting (`ChildOf`) by computing the local transform relative to the
parent's Position/Rotation. Runs in `PhysicsTransformSystems::PositionToTransform`
in the configured schedule.

---

## 5. Lag Compensation

### `LagCompensationPlugin`

Unit struct implementing `Plugin`. Server-only (requires a `Server` entity).

**What it does in `build()`**:

1. Initializes `LagCompensationConfig` resource.
2. Registers observer `spawn_broad_phase_aabb_envelope` on `On<Add, LagCompensationHistory>`.
3. Adds two systems to `PhysicsSchedule` in `LagCompensationSystems::UpdateHistory`:
   - `update_collision_layers`
   - `update_collider_history`
4. Configures set ordering in `PhysicsSchedule`:
   ```
   Solver → UpdateHistory (ambiguous_with Sleeping) → SpatialQuery → Collisions (ambiguous_with Finalize)
   ```
5. Also configures `FixedPostUpdate`:
   ```
   LagCompensationSystems::Collisions → after(PhysicsSystems::Prepare)
   ```

### `LagCompensationConfig`

```rust
#[derive(Resource)]
pub struct LagCompensationConfig {
    pub max_collider_history_ticks: u8,  // default: 35
}
```

Default of 35 ticks ≈ 500ms at 64 Hz. Maximum is 255 (u8). This limits how
far back `LagCompensationSpatialQuery` can query.

### `LagCompensationHistory` (Component)

```rust
pub type LagCompensationHistory = HistoryBuffer<(Position, Rotation, ColliderAabb)>;
```

A component on any server entity that needs lag compensation. The history is
keyed by tick and supports `add_update(tick, data)`, `clear_until_tick(tick)`,
iteration over all entries, and lookups by tick.

### `AabbEnvelopeHolder` (Component)

```rust
#[derive(Component)]
pub struct AabbEnvelopeHolder;
```

Marker on the child entity that holds the broad-phase AABB envelope collider.
Inserted automatically by the observer when `LagCompensationHistory` is added
to an entity.

### Child Entity Architecture

When `LagCompensationHistory` is added to entity E, the observer spawns a child
of E with:

```rust
Collider::cuboid(1.0, 1.0, 1.0),  // placeholder; resized every tick
Position::default(),
Rotation::default(),
AabbEnvelopeHolder,
CollisionLayers  // copied from parent at spawn time
```

Every tick, `update_collider_history`:

1. Reads the parent's current `(Position, Rotation, ColliderAabb)`.
2. Pushes into `LagCompensationHistory` keyed by current tick.
3. Prunes entries older than `current_tick - max_collider_history_ticks`.
4. Folds over all AABBs in the history → computes union → sets child collider
   to a cuboid covering the entire envelope.
5. Sets child `Position` to the center of the envelope.

The child's `CollisionLayers` are synced from the parent on change via
`update_collision_layers`.

**Why a child entity?** Avian's `SpatialQuery` operates on entity colliders.
By having a child with a cuboid collider covering the union of all historical
AABBs, broad-phase checks against the child will hit whenever *any* historical
position would have been within ray range.

### `LagCompensationSpatialQuery` (SystemParam)

```rust
#[derive(SystemParam)]
pub struct LagCompensationSpatialQuery<'w, 's> {
    pub timeline: Res<'w, LocalTimeline>,
    // private:
    server: Single<'w, 's, (), With<Server>>,  // ensures server-only
    spatial_query: SpatialQuery<'w, 's>,
    parent_query: Query<'w, 's, (&'static Collider, &'static CollisionLayers, &'static LagCompensationHistory)>,
    child_query: Query<'w, 's, &'static ChildOf, With<AabbEnvelopeHolder>>,
}
```

Read-only, `Send + Sync`. Systems using it should run *after*
`LagCompensationSystems::UpdateHistory`.

#### `cast_ray`

```rust
pub fn cast_ray(
    &self,
    interpolation_delay: InterpolationDelay,
    origin: Vector,
    direction: Dir,
    max_distance: Scalar,
    solid: bool,
    filter: &mut SpatialQueryFilter,
) -> Option<RayHitData>
```

Delegates to `cast_ray_predicate` with `&|_| true` as the predicate.

#### `cast_ray_predicate`

```rust
pub fn cast_ray_predicate(
    &self,
    interpolation_delay: InterpolationDelay,
    origin: Vector,
    direction: Dir,
    max_distance: Scalar,
    solid: bool,
    predicate: &dyn Fn(Entity) -> bool,
    filter: &mut SpatialQueryFilter,
) -> Option<RayHitData>
```

**Two-phase algorithm**:

**Phase 1 — Broad phase**: Calls `self.spatial_query.cast_ray_predicate(...)`
with the user's filter. The child AABB-envelope colliders will be hit if the
ray passes through any area the entity occupied within the history window.

**Phase 2 — Narrow phase** (inside the spatial query predicate closure):
1. When a child envelope is hit, look up the child's `ChildOf` → get parent.
2. Run `filter.test(parent, collision_layers)` — if the parent is excluded, reject.
3. Compute `interpolation_tick` and `interpolation_overstep` from
   `interpolation_delay.tick_and_overstep(current_tick)`.
4. Find the history entry at `interpolation_tick` (the "start" entry).
5. Get the next consecutive entry (the "target" entry at `interpolation_tick + 1`).
   - **Panics if `source_idx + 1` is out of bounds** — this means there must
     always be a consecutive entry. If the delay is fractional and the next
     tick has not been recorded yet, this panics.
6. Interpolate position: `start_position.lerp(target_position, overstep)`.
7. Interpolate rotation: `start_rotation.slerp(target_rotation, overstep)`.
8. Run `collider.cast_ray(interpolated_position, interpolated_rotation, ...)` using
   the **current** `Collider` component from the parent entity (NOT the historical
   collider shape — only position/rotation are historical).
9. If hit and `predicate(parent)` is true, return `RayHitData { entity: parent, distance, normal }`.

**Key insight**: The returned `entity` is the **parent** entity (the one with
`LagCompensationHistory`), not the child envelope.

### `InterpolationDelay`

```rust
pub struct InterpolationDelay {
    pub delay: PositiveTickDelta,  // e.g. PositiveTickDelta::lit("3") or PositiveTickDelta::lit("0.5")
}
```

**`tick_and_overstep(tick: Tick) -> (Tick, f32)`**:

- `interpolation_tick = tick - delay.floor()` (integer part of delay subtracted)
- `interpolation_overstep = delay.fract()` (fractional part, 0.0 if whole number)

**Examples**:
- Current tick 3, delay `"3"` → queries tick 0, overstep 0.0 (exact tick lookup)
- Current tick 3, delay `"1"` → queries tick 2, overstep 0.0
- Current tick 1, delay `"0.5"` → queries tick 0, overstep 0.5 (interpolated 50% between tick 0 and 1)
- Current tick 5, delay `"2.75"` → queries tick 2, overstep 0.75

`PositiveTickDelta::lit(src)` is a `const fn` that parses a string literal.

### History Recording System

**System `update_collider_history`** in `PhysicsSchedule` →
`LagCompensationSystems::UpdateHistory`:

```rust
fn update_collider_history(
    timeline: Res<LocalTimeline>,
    server: Single<(), With<Server>>,
    config: Res<LagCompensationConfig>,
    mut parent_query: Query<(&Position, &Rotation, &ColliderAabb, &mut LagCompensationHistory), Without<AabbEnvelopeHolder>>,
    mut children_query: Query<(&ChildOf, &mut Collider, &mut Position), With<AabbEnvelopeHolder>>,
)
```

For each child envelope entity:
1. Lookup parent by `ChildOf::parent()`.
2. `history.add_update(tick, (parent_position, parent_rotation, parent_aabb))`.
3. `history.clear_until_tick(tick - max_collider_history_ticks)`.
4. Fold all AABBs → compute union → set child collider to cuboid covering union.
5. Set child `Position` to envelope center.

### What Is NOT Available

- **No `cast_sphere`**, `cast_shape`, `point_projection`, or any other
  spatial query variant. Only `cast_ray` and `cast_ray_predicate`.
- **No 2D lag compensation** in this crate (separate `lightyear_avian2d`).
- **No host-server mode** handling (there is a comment in query.rs:132:
  "TODO: handle this in host-server mode!").
- **No collider shape history** — only Position, Rotation, and ColliderAabb are
  stored. If the collider shape itself changes, the narrow phase uses the
  current shape at the historical pose, which may produce incorrect results.
- **No automatic delay estimation** — `InterpolationDelay` must be computed
  by the user (or taken from `InterpolationDelay` component on client entities
  as stored by Lightyear's interpolation system).

---

## 6. Physics Correction / Rollback

The correction modules are `pub(crate)` — they are wired automatically when
using `AvianReplicationMode::PositionButInterpolateTransform`.

### `update_frame_interpolation_post_rollback`

**Schedule**: `PreUpdate → RollbackSystems::EndRollback`

**What it does**:
1. Reads the current corrected `Position` and `Rotation` (after rollback).
2. Reads `PredictionHistory<Position>` and `PredictionHistory<Rotation>`.
3. Computes the "before-last correct transform" via
   `position_history.second_most_recent(tick)`.
4. Sets `FrameInterpolate<Transform>`:
   - `current_value = to_transform(position, rotation)`
   - `previous_value = to_transform(before_last_pos, before_last_rot)`
5. Computes the displayed value at the current overstep:
   `current_visual = registry.interpolate(before_last, last, overstep)`
6. Computes error = `current_visual.diff(&previous_visual)` where
   `previous_visual` comes from `PreviousVisual<Position>` /
   `PreviousVisual<Rotation>`.
7. Inserts `VisualCorrection<Isometry3d> { error }` and removes the
   `PreviousVisual` components.

If `SkipFrameInterpolation` is present, both current and previous are set to
the corrected transform, and `SkipFrameInterpolation` is removed.

### `add_visual_correction`

**Schedule**: `PostUpdate → RollbackSystems::VisualCorrection`

**What it does**:
1. Gets lerp ratio from `manager.correction_policy.lerp_ratio(time.delta())`.
2. For each entity with `VisualCorrection<Isometry3d>`:
   - If error is small enough (checked via `prediction.should_rollback` on
     Position and Rotation separately), removes the component.
   - Otherwise: samples `EasingCurve::new(identity, error, Linear)` at lerp
     ratio, applies to `Transform` via `bypass_change_detection().apply_diff()`,
     updates the remaining error.

### `to_transform` helper

```rust
fn to_transform(pos: &Position, rot: &Rotation) -> Transform {
    Transform {
        translation: pos.f32(),
        rotation: rot.f32(),
        scale: Vec3::ONE,  // TODO: handle scale
    }
}
```

---

## 7. Lerp Functions (`types_3d` / `types_2d`)

These are used by Lightyear's prediction history interpolation and frame
interpolation systems. They are imported implicitly when the crate registers
them in `InterpolationRegistry` or through the `avian3d` feature in
`lightyear_replication`.

### 3D

| Module | Lerp |
|--------|------|
| `position` | `Position::new(start.0 * (1.0 - u) + other.0 * u)` |
| `rotation` | `start.slerp(*other, Scalar::from(t))` |
| `linear_velocity` | `LinearVelocity(start.0 * (1.0 - u) + other.0 * u)` |
| `angular_velocity` | `AngularVelocity(start.0 * (1.0 - u) + other.0 * u)` |

### 2D

| Module | Lerp |
|--------|------|
| `position` | Same linear formula |
| `rotation` | Wraps shortest angle: `(((diff % 360) + 540) % 360) - 180` |
| `linear_velocity` | Same linear formula |
| `angular_velocity` | Same linear formula |

### Deterministic Hashing (with `deterministic` feature)

Writes each component as `f32::to_bits()` through `seahash::SeaHasher`:

| Type | Components |
|------|-----------|
| `Position` (3D) | x, y, z |
| `Position` (2D) | x, y |
| `Rotation` (3D) | `rot.to_array()` → [x, y, z, w] |
| `Rotation` (2D) | cos, sin |

---

## 8. Test-Backed Usage

There are **no tests inside the crate itself**. All test coverage lives in
`crates/prototypes/prototype-physics-lightyear/src/main.rs` (5 tests).

### Test: `lag_compensation_app()` — Minimal test harness

Source: `prototype-physics-lightyear/main.rs:135`

```rust
fn lag_compensation_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(PhysicsPlugins::new(FixedUpdate).build());
    app.add_plugins(LagCompensationPlugin);
    app.init_resource::<LocalTimeline>();
    app.insert_resource(LagRayReports::default());
    app.world_mut().spawn(Server::default());
    app.finish();
    app.cleanup();
    app
}
```

Key requirements:
- `MinimalPlugins` (for schedule execution)
- `PhysicsPlugins` (for `PhysicsSchedule` and `SpatialQuery`)
- `LagCompensationPlugin` (for history recording)
- `LocalTimeline` resource (for tick management)
- `Server::default()` entity (required by `LagCompensationSpatialQuery`'s `Single<(), With<Server>>`)

### Test: `spawn_lag_compensated_cube()` — Entity setup

Source: `prototype-physics-lightyear/main.rs:150`

```rust
fn spawn_lag_compensated_cube(app: &mut App) -> Entity {
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(1.0, 1.0, 1.0),
        Position(Vec3::new(5.0, 0.0, 0.0)),
        Rotation::IDENTITY,
        CollisionLayers::default(),
        LagCompensationHistory::default(),  // triggers child spawn
    )).id()
}
```

After spawn + flush, the entity has a child with `AabbEnvelopeHolder` and a
placeholder cuboid collider.

### Test: `record_target_position()` — Manual history recording

Source: `prototype-physics-lightyear/main.rs:165`

```rust
fn record_target_position(app: &mut App, entity: Entity, tick: u16, position: Vec3) {
    let current = app.world().resource::<LocalTimeline>().tick().0;
    app.world_mut()
        .resource_mut::<LocalTimeline>()
        .apply_delta(tick as i16 - current as i16);
    app.world_mut().entity_mut(entity).insert(Position(position));
    app.world_mut().run_schedule(PhysicsSchedule);
}
```

Manually advances the timeline to the target tick, sets the desired position,
and runs `PhysicsSchedule` to trigger history recording.

### Test: `lag_compensation_ray_hits_historical_collider_tick`

Source: `prototype-physics-lightyear/main.rs:88`

Records the cube at x=5 for ticks 0-1, then x=20 for ticks 2-3. At current
tick 3:
- Delay "3" → queries tick 0 (cube at x=5, within 10-unit ray from origin) → **hits**
- Delay "1" → queries tick 2 (cube at x=20, outside ray) → **misses**

### Test: `lag_compensation_interpolates_between_historical_ticks`

Source: `prototype-physics-lightyear/main.rs:109`

Records cube at x=5 (tick 0), x=9 (tick 1). At current tick 1 with delay
"0.5": queries tick 0, overstep 0.5 → interpolated position at x=7. Ray hits
the near face of the 1-unit cuboid at ~6.5 units.

### Test: `rollback_removes_predicted_joint`

Source: `prototype-physics-lightyear/main.rs:49`

Verifies that adding/removing `SphericalJoint` during rollback does not destroy
the `RigidBody` or `Collider` components on affected entities.

### Test: `joint_survives_physics_step`

Source: `prototype-physics-lightyear/main.rs:67`

Verifies that `SphericalJoint` is not removed by Avian's physics schedule
execution.

---

## 9. Complete Gotchas & Footguns

### From `plugin.rs` (source line refs in comments)

1. **Predicted entities get `Confirmed<Position>`** — When Lightyear replicates
   Position on a predicted entity, it arrives as `Confirmed<Position>`, which
   triggers an immediate rollback. This is expected, but if your system runs
   during rollback and reads `Position` directly, it may see stale or
   partially-rolled-back values. (plugin.rs:5)

2. **Interpolated entities may lack `Position` or `Rotation`** — Lightyear's
   interpolation only inserts the real component after receiving two remote
   updates for it. If `Rotation` is updated less frequently than `Position`,
   an interpolated entity may have `Position` but not `Rotation`. Avian's
   `sync_pos_to_transform` only fires when **both** are present, so the entity
   might display at `Transform::default()`. Mitigation: only add rendering
   components when both `Position` and `Rotation` are present on interpolated
   entities. (plugin.rs:8-13)

3. **Do NOT add `RigidBody` on interpolated entities** — `RigidBody` auto-inserts
   `Position`, `Rotation`, and `Transform` with defaults, and runs unwanted
   avian systems on interpolated entities. (plugin.rs:16-18)

4. **Child collider Position must be manually updated** — Avian normally does
   this in `PhysicsSystems::First`. The bridge does it again in
   `update_child_collider_position` after physics, but if you access child
   collider `Position` between physics and the bridge system, it will be stale.
   (plugin.rs:586-618)

5. **PositionButInterpolateTransform: Transform propagation broken** — The
   author explicitly states "I believe that this currently does NOT handle
   TransformPropagation to children correctly." (plugin.rs:88). If you need
   hierarchical transforms and use this mode, test carefully.

6. **`add_transform` does not handle `ChildOf` inserted after Position/Rotation**
   — The query-based `add_transform` checks `is_added()` on Position/Rotation,
   and if `ChildOf` was added in a different schedule point, the transform may
   be computed without parent awareness. This is documented in comments
   (plugin.rs:486-494).

### From `correction_3d.rs`

7. **Scale is always `Vec3::ONE`** — The `to_transform` helper ignores scale.
   If you use non-uniform scale in `Transform`, the correction will reset it.
   (correction_3d.rs:178)

8. **`update_frame_interpolation_post_rollback` uses overstep from `Time<Fixed>`**
   — It reads `time.overstep_fraction()` which is the overstep from the
   *previous* frame since it runs before `RunFixedMainLoop`. This is correct
   by design but subtle. (correction_3d.rs:53-55)

9. **`second_most_recent(tick)` may be `None`** — If the prediction history
   doesn't have enough entries, the system returns early without updating.
   This means the first frame after a prediction correction might be skipped.
   (correction_3d.rs:88-93)

### From `lag_compensation/query.rs`

10. **`cast_ray_predicate` filter doesn't apply to child envelope directly** —
    The user's `SpatialQueryFilter` is used for the broad-phase cast, which
    hits the child AABB envelope entity, not the parent. If the user excluded
    the parent via the filter, the exclusion is **ineffective** for broad-phase:
    the child is not filtered out, so the narrow phase runs anyway. The narrow
    phase does check `filter.test(parent, collision_layers)`, so the final
    result is correct, but the broad phase still wastes work. There is a TODO
    about this: "the user could have excluded the Parent entity from the filter,
    which would do nothing since we are checking collisions with the child!"
    (query.rs:89-90)

11. **Panic if no consecutive history entry** — In the narrow phase, the code
    does `history.into_iter().nth(source_idx + 1).unwrap()`. If the
    interpolation tick is the most recent history entry, there is no "next"
    entry, and this **panics**. This happens when `interpolation_tick ==
    current_tick - 1` and the next tick hasn't been recorded yet. (query.rs:132-133)

12. **Collider shape is not historical** — The narrow phase uses the *current*
    `Collider` component from the parent entity, swept to the historical
    position/rotation. If the collider shape changed between the historical tick
    and now, the intersection test is against the wrong geometry. (query.rs:145)

13. **ColliderAabb is stored but only used for envelope, not narrow phase** —
    The AABB is only used to compute the broad-phase envelope. The narrow phase
    ignores it and uses the current collider shape.

14. **No host-server mode** — The TODO at query.rs:131 says
    "TODO: handle this in host-server mode!" — there may be issues when the
    server and client are in the same process.

### From `lag_compensation/history.rs`

15. **`max_collider_history_ticks` is `u8`** — Maximum 255 ticks (~4 seconds
    at 60Hz). Default 35 (~500ms). The buffer cannot hold more.

16. **History assumes `PhysicsSchedule::Solver` runs every tick** — The system
    records after `Solver`. If physics is sub-stepped or paused, history may
    not be recorded for some ticks.

17. **ColliderAabb is computed before solver step** — The source comment
    explains: "The ColliderAABB gets updated in the BroadPhase set (before the
    Solver step) which might cause a 1-tick delay but that shouldn't matter
    much because we are just using it to compute an aabb envelope of all ticks"
    (history.rs:77-79).

18. **`update_collision_layers` uses `Changed<CollisionLayers>`** — If the
    parent's collision layers change and then change back before the next
    frame, the child may not be updated (the `Changed` detections may be
    missed between frames).

### General

19. **No built-in `InterpolationDelay` estimation** — The `InterpolationDelay`
    in Lightyear is stored as a component on `Client` entities. You must either
    read it from there or compute it manually. The crate does not provide a
    helper to look up a client's delay for a given entity.

20. **This crate uses `#![no_std]`** — Panics (like the `.unwrap()` in
    query.rs:133) will abort in `no_std` environments unless the `std` feature
    is enabled.

21. **`RayHitData.entity` points to the parent** — In lag compensation,
    the returned entity is always the parent with `LagCompensationHistory`,
    never the AABB envelope child. This is correct for gameplay code but may
    be surprising if you expected the hit collider entity.

---

## 10. Integration with Afterglow

### Current usage

Afterglow uses `lightyear_avian3d` only for lag-compensation research in the
physics prototype:

- **`crates/prototypes/prototype-physics-lightyear`** — Depends on
  `lightyear_avian3d = { version = "0.26.4", features = ["3d", "lag_compensation"] }`.
  Uses `LagCompensationPlugin`, `LagCompensationHistory`,
  `LagCompensationSpatialQuery`, and `InterpolationDelay` in its test harness.

- **`crates/engine-rpg-harness`** — Does **not** use the avian bridge at all.
  It uses raw `lightyear` plugins, `PreSpawned` entity matching, and custom
  game components for interactions/cues. Physics tests use Avian through the
  engine physics plugin, not `LightyearAvianPlugin`.

- **`crates/mock-rpg-network-tests/src/network_e2e/physics_grab.rs`** — Legacy
  oracle with the older `PreSpawned` grab pattern. It also does not use the
  avian bridge.

- **Engine runtime (`afterglow_engine`)** — Does **not** depend on
  `lightyear_avian3d` at all. Physics is standalone Avian. Lightyear networking
  does not wire the bridge into `AfterglowPhysicsPlugin`.

### What this means

1. If Afterglow later needs automatic Lightyear replication of Avian native
   components, it can add `LightyearAvianPlugin` to both client and server apps.
   The likely mode for Afterglow's setup (Position-only serialization) is
   `AvianReplicationMode::Position`.

2. The `LagCompensationPlugin` is server-only and requires the `Server` entity.
   Do not add it to the baseline. Use it only if a future FPS/twitch fairness
   feature explicitly needs historical collider queries.

3. Before the `physics_grab.rs` test can use the bridge, it needs to be updated
   to add `LightyearAvianPlugin` and move from manual `Confirmed` reconciliation
   to the bridge's automatic correction/interpolation.

4. The known issues in section 9 are directly relevant:
   - Issue #2 (interpolated entities missing Position/Rotation) affects
     Afterglow's rendering setup.
   - Issue #5 (broken Transform propagation in
     `PositionButInterpolateTransform`) means Afterglow should use
     `Position` or `Transform` mode if hierarchy is important.
   - Issue #11 (panic if no consecutive history entry) must be handled if
     lag compensation is used near the leading edge of history.
   - Issue #10 (filter not applying to child) must be understood when
     designing server hit-validation filters.

### Optional setup for future engine networking research

```rust
app.add_plugins(LightyearAvianPlugin {
    replication_mode: AvianReplicationMode::Position,
    update_syncs_manually: false,
    rollback_resources: false,
    rollback_islands: false,
});

// Server only, optional research:
app.add_plugins(LagCompensationPlugin);
```

For deterministic (input-based) bridge experiments:
```rust
app.add_plugins(LightyearAvianPlugin {
    replication_mode: AvianReplicationMode::Position,
    rollback_resources: true,
    rollback_islands: true,
    ..default()
});
```

---

*A note on the `physics_grab.rs` test pattern*: The grab interaction uses
`PreSpawned` entity matching on a custom `GrabConstraint` component rather
than manipulating Avian physics components directly. This is the recommended
Lightyear pattern for transient predicted interactions. The avian bridge is
not needed for this pattern — the bridge is for when you want automatic
replication of Avian's native `Position`/`Rotation`/`Transform` components
with rollback correction.
