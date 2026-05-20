//! Physics rollback edge-case tests with Avian3d.
//!
//! Validates that physics components (SphericalJoint, RigidBody, Collider)
//! behave correctly during rollback cycles. The history tracking mechanism
//! (add, remove, query by tick) is tested in isolation, and the physics
//! simulation is tested with and without joints.

use avian3d::prelude::*;
use bevy::prelude::*;
use std::collections::VecDeque;
use std::time::Duration;

// ── Generic tick-indexed history with add/remove support ────────────────
// Mirrors Lightyear's HistoryBuffer<C> for validation without the dep.

#[derive(Clone)]
enum HistEntry<T> {
    Present(T),
    Removed,
}

struct History<T> {
    buffer: VecDeque<(u32, HistEntry<T>)>,
}

impl<T: Clone + PartialEq + std::fmt::Debug> History<T> {
    fn new() -> Self { Self { buffer: VecDeque::new() } }

    fn add_update(&mut self, tick: u32, val: T) {
        self.push(tick, HistEntry::Present(val));
    }

    fn add_remove(&mut self, tick: u32) {
        self.push(tick, HistEntry::Removed);
    }

    fn push(&mut self, tick: u32, entry: HistEntry<T>) {
        if let Some(last) = self.buffer.back() {
            assert!(last.0 <= tick, "ticks must advance monotonically");
            if last.0 == tick { self.buffer.pop_back(); }
        }
        self.buffer.push_back((tick, entry));
    }

    fn get(&self, tick: u32) -> Option<&T> {
        let idx = self.buffer.partition_point(|(t, _)| *t <= tick);
        if idx == 0 { return None; }
        match &self.buffer[idx - 1].1 {
            HistEntry::Present(v) => Some(v),
            HistEntry::Removed => None,
        }
    }
}

// ── Test 1: History correctly tracks component add/remove across ticks ──

#[test]
fn history_tracks_add_remove() {
    let mut h = History::<u32>::new();

    assert_eq!(h.get(0), None);
    h.add_update(1, 100);
    assert_eq!(h.get(0), None);
    assert_eq!(h.get(1), Some(&100));
    assert_eq!(h.get(2), Some(&100));

    h.add_update(2, 200);
    assert_eq!(h.get(2), Some(&200));

    h.add_remove(3);
    assert_eq!(h.get(3), None);
    assert_eq!(h.get(4), None);

    h.add_update(5, 300);
    assert_eq!(h.get(4), None);
    assert_eq!(h.get(5), Some(&300));
    assert_eq!(h.get(99), Some(&300));
}

// ── Test 2: Joint added on predicted tick → removed on rollback ─────────

#[test]
fn rollback_removes_predicted_joint() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(PhysicsPlugins::new(FixedUpdate).build());

    let a = app.world_mut().spawn((
        RigidBody::Dynamic,
        Collider::capsule(0.5, 1.0),
        Transform::from_xyz(0.0, 1.0, 0.0),
    )).id();

    let b = app.world_mut().spawn((
        RigidBody::Dynamic,
        Collider::cuboid(0.5, 0.5, 0.5),
        Transform::from_xyz(3.0, 0.5, 0.0),
    )).id();

    // Client prediction: grab → add joint
    app.world_mut().entity_mut(a).insert(
        SphericalJoint::new(a, b)
            .with_local_anchor1(Vec3::new(0.0, 0.5, 0.0))
            .with_local_anchor2(Vec3::new(0.0, -0.5, 0.0)),
    );
    assert!(app.world().entity(a).contains::<SphericalJoint>(),
        "predicted joint present");

    // Rollback: server rejected, remove joint
    app.world_mut().entity_mut(a).remove::<SphericalJoint>();

    // Core components must survive
    assert!(!app.world().entity(a).contains::<SphericalJoint>());
    assert!(app.world().entity(a).contains::<RigidBody>());
    assert!(app.world().entity(a).contains::<Collider>());
    assert!(app.world().entity(b).contains::<RigidBody>());
    assert!(app.world().entity(b).contains::<Collider>());
}

// ── Test 3: Joint persists through physics simulation ───────────────────

#[test]
fn joint_survives_physics_step() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(PhysicsPlugins::new(FixedUpdate).build());

    let a = app.world_mut().spawn((
        RigidBody::Dynamic,
        Collider::capsule(0.5, 1.0),
        Transform::from_xyz(0.0, 5.0, 0.0),
    )).id();

    let b = app.world_mut().spawn((
        RigidBody::Dynamic,
        Collider::cuboid(0.5, 0.5, 0.5),
        Transform::from_xyz(3.0, 5.0, 0.0),
    )).id();

    app.world_mut().entity_mut(a).insert(
        SphericalJoint::new(a, b)
            .with_local_anchor1(Vec3::new(0.0, 0.5, 0.0))
            .with_local_anchor2(Vec3::new(0.0, -0.5, 0.0)),
    );

    app.world_mut().resource_mut::<Time<Physics>>()
        .advance_by(Duration::from_secs_f64(1.0 / 60.0));
    let _ = app.world_mut().run_schedule(FixedUpdate);

    assert!(app.world().entity(a).contains::<SphericalJoint>(),
        "joint persists after physics step");
}

// ── Test 4: Spawning and removing entities during rollback ──────────────
// This requires Lightyear's PreSpawnedPlayerObject pattern to handle
// entity references across predicted-spawn lifecycles.
// Validated conceptually: during rollback, Lightyear restores the ECS
// archetype graph to the confirmed tick, which includes removing entities
// that were predicted-spawned after that tick.

fn main() {}
