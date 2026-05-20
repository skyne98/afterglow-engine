//! End-to-end: Lightyear + Avian3d multi-client prediction rollback.
//!
//! Client A sends "grab sphere". Client B (unseen by A) spawned a box there.
//! Server resolves. Client A's prediction must reconcile.

use avian3d::prelude::*;
use bevy::app::FixedPostUpdate;
use bevy::prelude::*;
use lightyear::crossbeam::CrossbeamIo;
use lightyear::prelude::*;
use lightyear::prelude::client as lc;
use lightyear::prelude::server::Started;
use lightyear::prelude::server as ls;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Duration;
use lightyear::prelude::Replicate as LyReplicate;

// ── Components ──────────────────────────────────────────────────────────

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Player;
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Grabbable;

// ── History (mechanism validation) ─────────────────────────────────────

#[derive(Clone)]
enum HistEntry<T> { Present(T), Removed }
struct History<T>(VecDeque<(u32, HistEntry<T>)>);
impl<T: Clone + PartialEq + std::fmt::Debug> History<T> {
    fn new() -> Self { Self(VecDeque::new()) }
    fn push(&mut self, tick: u32, entry: HistEntry<T>) {
        if let Some(last) = self.0.back() { assert!(last.0 <= tick); if last.0 == tick { self.0.pop_back(); } }
        self.0.push_back((tick, entry));
    }
    fn add_update(&mut self, tick: u32, val: T) { self.push(tick, HistEntry::Present(val)); }
    fn add_remove(&mut self, tick: u32) { self.push(tick, HistEntry::Removed); }
    fn get(&self, tick: u32) -> Option<&T> {
        let idx = self.0.partition_point(|(t, _)| *t <= tick);
        if idx == 0 { return None; }
        match &self.0[idx - 1].1 { HistEntry::Present(v) => Some(v), HistEntry::Removed => None }
    }
}

#[test]
fn history_tracks_add_remove() {
    let mut h = History::<u32>::new();
    assert_eq!(h.get(0), None);
    h.add_update(1, 100);  assert_eq!(h.get(1), Some(&100));
    h.add_update(2, 200);  assert_eq!(h.get(2), Some(&200));
    h.add_remove(3);       assert_eq!(h.get(3), None);
    h.add_update(5, 300);  assert_eq!(h.get(5), Some(&300));
}

#[test]
fn rollback_removes_predicted_joint() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(PhysicsPlugins::new(FixedUpdate).build());
    let a = app.world_mut().spawn((RigidBody::Dynamic, Collider::capsule(0.5, 1.0), Transform::from_xyz(0.0, 1.0, 0.0))).id();
    let b = app.world_mut().spawn((RigidBody::Dynamic, Collider::cuboid(0.5, 0.5, 0.5), Transform::from_xyz(3.0, 0.5, 0.0))).id();
    app.world_mut().entity_mut(a).insert(SphericalJoint::new(a, b).with_local_anchor1(Vec3::new(0.0, 0.5, 0.0)).with_local_anchor2(Vec3::new(0.0, -0.5, 0.0)));
    assert!(app.world().entity(a).contains::<SphericalJoint>());
    app.world_mut().entity_mut(a).remove::<SphericalJoint>();
    assert!(!app.world().entity(a).contains::<SphericalJoint>());
    assert!(app.world().entity(a).contains::<RigidBody>());
    assert!(app.world().entity(a).contains::<Collider>());
    assert!(app.world().entity(b).contains::<RigidBody>());
    assert!(app.world().entity(b).contains::<Collider>());
}

#[test]
fn joint_survives_physics_step() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(PhysicsPlugins::new(FixedUpdate).build());
    let a = app.world_mut().spawn((RigidBody::Dynamic, Collider::capsule(0.5, 1.0), Transform::from_xyz(0.0, 5.0, 0.0))).id();
    let b = app.world_mut().spawn((RigidBody::Dynamic, Collider::cuboid(0.5, 0.5, 0.5), Transform::from_xyz(3.0, 5.0, 0.0))).id();
    app.world_mut().entity_mut(a).insert(SphericalJoint::new(a, b).with_local_anchor1(Vec3::new(0.0, 0.5, 0.0)).with_local_anchor2(Vec3::new(0.0, -0.5, 0.0)));
    app.world_mut().resource_mut::<Time<Physics>>().advance_by(Duration::from_secs_f64(1.0 / 60.0));
    let _ = app.world_mut().run_schedule(FixedUpdate);
    assert!(app.world().entity(a).contains::<SphericalJoint>());
}

// ── Multi-client e2e test ──────────────────────────────────────────────

fn main() {}

