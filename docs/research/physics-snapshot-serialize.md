# Physics Snapshot Serialization: Deterministic Round-Trip

**Date:** 2026-05-19
**Crate:** `prototype-physics-serialize` at `crates/prototypes/prototype-physics-serialize`

## Goal

Prove that we can capture the full state of a deterministic Avian physics world (bodies, colliders, joints, velocities), serialize it to bytes, later deserialize and reconstruct a fresh world, and have the simulation produce bit-identical results from that point forward.

This is the foundation for late-joiner recovery in deterministic lockstep (snapshot at tick T + replay input history).

## Design

### Snapshot format (`serde` + `postcard`)

The snapshot is a flat `PhysicsSnapshot` with two vectors:

```
PhysicsSnapshot
├── bodies: Vec<BodySnapshot>
│   ├── index: u32
│   ├── rigidbody_type: Dynamic | Static | Kinematic
│   ├── translation: [f32; 3]
│   ├── rotation: [f32; 4]
│   ├── linvel: [f32; 3]
│   ├── angvel: [f32; 3]
│   └── collider: Option<ColliderSnapshot>
│       ├── Cuboid { hx, hy, hz }
│       ├── Sphere { radius }
│       └── Capsule { radius, half_height }
└── joints: Vec<JointSnapshot>
    ├── body1_index: u32      ← remapped from Entity
    ├── body2_index: u32      ← remapped from Entity
    └── kind: JointKindSnapshot
        ├── Spherical { anchor1, anchor2, twist_axis }
        └── Fixed { anchor1, anchor2 }
```

Entity IDs in joints are replaced with sequential indices into the bodies array. On restore, bodies are spawned first (recording new `Entity` IDs), then joints are reconstructed using the remapped IDs.

### Collider handling

Parry's `SharedShape` does not implement `serde`, and the Avian `Collider`'s internal `shape` field is private. Instead, we match on the shape type at runtime via `shape.as_cuboid()`, `shape.as_ball()`, `shape.as_capsule()` and store the primitive parameters. Only cuboid, sphere, and capsule are currently supported — these cover the common cases.

### The `serialize` feature on Avian

Avian joints (`SphericalJoint`, `FixedJoint`, etc.) conditionally derive `serde::Serialize`/`Deserialize` when the `serialize` feature is enabled. We do not use serde *directly* on joints (to avoid Entity serialization issues), but having serde available simplifies inspection/debugging.

## Verification

The binary self-verifies by:

1. Building a test world with `N` bodies (chains of 10 boxes connected by `SphericalJoint` + loose individual boxes)
2. Running 40 physics steps to reach a non-trivial state
3. Taking a snapshot (serialize to `postcard` bytes)
4. Running another 60 steps, recording hash A
5. Building a *fresh* world, restoring from snapshot bytes
6. Running the same 60 steps, recording hash B
7. Comparing hash A vs B (position + rotation bit patterns of all dynamic bodies)

## Results

| Bodies | Joints | Snapshot size | Post-restore steps | Matched? |
|--------|--------|--------------|--------------------|----------|
| 1,000  | 900    | 105 KB       | 60                 | ✅ |
| 10,000 | 1,800  | 763 KB       | 60                 | ✅ |

Both runs produced bit-identical hashes.

## Usage

```sh
RUSTFLAGS="-C target-cpu=native" cargo run --release --package prototype-physics-serialize <body_count>
```
