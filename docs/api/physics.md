# Physics API

## Backend

Afterglow uses `avian3d` as its 3D physics backend. The dependency is configured
without Avian's debug-render, picking, scene, or mesh-collider generation features
so the core runtime stays usable in minimal/headless tests and servers.

The raw Avian prelude is available as `afterglow_engine::physics::avian::*` for
advanced use.

## Plugin

| Item | Purpose |
|---|---|
| `AfterglowPhysicsPlugin` | Adds Avian `PhysicsPlugins`, registers engine authoring components, and syncs global config. |
| `AfterglowPhysicsConfig` | Global physics knobs. Currently owns `gravity: Vec3`, mirrored to Avian's `Gravity` resource. |

`AfterglowRuntimePlugins` includes `AfterglowPhysicsPlugin`.

## Authoring Components

| Item | Fields/Variants | Purpose |
|---|---|---|
| `PhysicsBody` | `kind: PhysicsBodyKind` | Engine-facing rigid body authoring component. Synced to Avian `RigidBody`. |
| `PhysicsBodyKind` | `Dynamic`, `Static`, `Kinematic` | Generic body type. |
| `PhysicsCollider` | `Cuboid { size }`, `Sphere { radius }`, `Cylinder { radius, height }`, `Capsule { radius, length }`, `ConvexHull { points }` | Engine-facing collider authoring component. Synced to Avian `Collider`. Use convex hulls for player-standable ramp/wedge collision instead of raw trimeshes. |
| `PhysicsVelocity` | `linear`, `angular` | Initial or authored velocity. Synced to Avian `LinearVelocity` and `AngularVelocity`. |

These components are intentionally small. Game code may use Avian components
directly when it needs backend-specific features, but first-party engine systems
should prefer the authoring components until a real need appears.

## Lifecycle

Physics bodies are regular Bevy entities. Chunk unloads despawn entities by
stable ID/chunk membership, so Avian state is removed with the entity and does
not require a separate physics-world cleanup path.
