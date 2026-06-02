//! Focused Lightyear + Avian3d physics prediction experiments.
//!
//! The full multi-client grab reconciliation regression lives in
//! `mock-rpg-network-tests`. This prototype keeps the smaller mechanism tests
//! close to the Lightyear Avian bridge, especially lag-compensated historical
//! collision queries.

#[cfg(test)]
use avian3d::prelude::*;
#[cfg(test)]
use bevy::prelude::*;
#[cfg(test)]
use lightyear::{core::time::PositiveTickDelta, prelude::*};
#[cfg(test)]
use lightyear_avian3d::prelude::*;
#[cfg(test)]
use std::{collections::VecDeque, time::Duration};

// ── History (mechanism validation) ─────────────────────────────────────

#[derive(Clone)]
#[cfg(test)]
enum HistEntry<T> {
    Present(T),
    Removed,
}
#[cfg(test)]
struct History<T>(VecDeque<(u32, HistEntry<T>)>);

#[cfg(test)]
impl<T: Clone + PartialEq + std::fmt::Debug> History<T> {
    fn new() -> Self {
        Self(VecDeque::new())
    }
    fn push(&mut self, tick: u32, entry: HistEntry<T>) {
        if let Some(last) = self.0.back() {
            assert!(last.0 <= tick);
            if last.0 == tick {
                self.0.pop_back();
            }
        }
        self.0.push_back((tick, entry));
    }
    fn add_update(&mut self, tick: u32, val: T) {
        self.push(tick, HistEntry::Present(val));
    }
    fn add_remove(&mut self, tick: u32) {
        self.push(tick, HistEntry::Removed);
    }
    fn get(&self, tick: u32) -> Option<&T> {
        // `partition_point` gives the first entry newer than the requested tick;
        // the previous entry is the historical state active at that tick.
        let idx = self.0.partition_point(|(t, _)| *t <= tick);
        if idx == 0 {
            return None;
        }
        match &self.0[idx - 1].1 {
            HistEntry::Present(v) => Some(v),
            HistEntry::Removed => None,
        }
    }
}

#[test]
fn history_tracks_add_remove() {
    let mut h = History::<u32>::new();
    assert_eq!(h.get(0), None);
    h.add_update(1, 100);
    assert_eq!(h.get(1), Some(&100));
    h.add_update(2, 200);
    assert_eq!(h.get(2), Some(&200));
    h.add_remove(3);
    assert_eq!(h.get(3), None);
    h.add_update(5, 300);
    assert_eq!(h.get(5), Some(&300));
}

#[test]
fn rollback_removes_predicted_joint() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(PhysicsPlugins::new(FixedUpdate).build());
    let a = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::capsule(0.5, 1.0),
            Transform::from_xyz(0.0, 1.0, 0.0),
        ))
        .id();
    let b = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::cuboid(0.5, 0.5, 0.5),
            Transform::from_xyz(3.0, 0.5, 0.0),
        ))
        .id();
    // Local rollback must be able to remove a predicted constraint without
    // disturbing the bodies/colliders that existed before prediction.
    app.world_mut().entity_mut(a).insert(
        SphericalJoint::new(a, b)
            .with_local_anchor1(Vec3::new(0.0, 0.5, 0.0))
            .with_local_anchor2(Vec3::new(0.0, -0.5, 0.0)),
    );
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
    let a = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::capsule(0.5, 1.0),
            Transform::from_xyz(0.0, 5.0, 0.0),
        ))
        .id();
    let b = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::cuboid(0.5, 0.5, 0.5),
            Transform::from_xyz(3.0, 5.0, 0.0),
        ))
        .id();
    // This guards the opposite edge: a predicted/authoritative joint should not
    // disappear merely because Avian stepped the physics schedule.
    app.world_mut().entity_mut(a).insert(
        SphericalJoint::new(a, b)
            .with_local_anchor1(Vec3::new(0.0, 0.5, 0.0))
            .with_local_anchor2(Vec3::new(0.0, -0.5, 0.0)),
    );
    app.world_mut()
        .resource_mut::<Time<Physics>>()
        .advance_by(Duration::from_secs_f64(1.0 / 60.0));
    let _ = app.world_mut().run_schedule(FixedUpdate);
    assert!(app.world().entity(a).contains::<SphericalJoint>());
}

#[derive(Resource, Default)]
#[cfg(test)]
struct LagRayReports {
    historical_hit: Option<Entity>,
    current_miss: bool,
}

#[test]
fn lag_compensation_ray_hits_historical_collider_tick() {
    let mut app = lag_compensation_app();
    let target = spawn_lag_compensated_cube(&mut app);

    // Current server tick becomes 3 after the final record. A delay of 3 queries
    // tick 0, where the cube is inside ray range; a delay of 1 queries tick 2,
    // where the cube has already moved outside the 10-unit ray.
    record_target_position(&mut app, target, 0, Vec3::new(5.0, 0.0, 0.0));
    record_target_position(&mut app, target, 1, Vec3::new(5.0, 0.0, 0.0));
    record_target_position(&mut app, target, 2, Vec3::new(20.0, 0.0, 0.0));
    record_target_position(&mut app, target, 3, Vec3::new(20.0, 0.0, 0.0));

    app.add_systems(Update, record_lag_compensation_ray_reports);
    app.world_mut().run_schedule(Update);

    let reports = app.world().resource::<LagRayReports>();
    assert_eq!(reports.historical_hit, Some(target));
    assert!(reports.current_miss);
}

#[test]
fn lag_compensation_interpolates_between_historical_ticks() {
    let mut app = lag_compensation_app();
    let target = spawn_lag_compensated_cube(&mut app);

    // With current tick 1 and delay 0.5, Lightyear Avian interpolates the cube
    // center halfway between x=5 and x=9. The cuboid half-extent is 0.5, so the
    // ray should hit the near face at approximately 6.5 units.
    record_target_position(&mut app, target, 0, Vec3::new(5.0, 0.0, 0.0));
    record_target_position(&mut app, target, 1, Vec3::new(9.0, 0.0, 0.0));

    app.insert_resource(InterpolatedLagRayReport::default());
    app.add_systems(Update, record_interpolated_lag_ray_report);
    app.world_mut().run_schedule(Update);

    let report = app.world().resource::<InterpolatedLagRayReport>();
    assert_eq!(report.hit, Some(target));
    assert!((report.distance.unwrap() - 6.5).abs() < 0.05);
}

#[derive(Resource, Default)]
#[cfg(test)]
struct InterpolatedLagRayReport {
    hit: Option<Entity>,
    distance: Option<f32>,
}

#[cfg(test)]
fn lag_compensation_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(PhysicsPlugins::new(FixedUpdate).build());
    app.add_plugins(LagCompensationPlugin);
    // LagCompensationSpatialQuery derives the historical query tick from the
    // local timeline and the supplied InterpolationDelay.
    app.init_resource::<LocalTimeline>();
    app.insert_resource(LagRayReports::default());
    app.world_mut().spawn(Server::default());
    app.finish();
    app.cleanup();
    app
}

#[cfg(test)]
fn spawn_lag_compensated_cube(app: &mut App) -> Entity {
    let entity = app
        .world_mut()
        .spawn((
            RigidBody::Static,
            Collider::cuboid(1.0, 1.0, 1.0),
            Position(Vec3::new(5.0, 0.0, 0.0)),
            Rotation::IDENTITY,
            CollisionLayers::default(),
            // This marker owns the per-entity collider/pose history used by
            // LagCompensationSpatialQuery.
            LagCompensationHistory::default(),
        ))
        .id();
    app.world_mut().flush();
    entity
}

#[cfg(test)]
fn record_target_position(app: &mut App, entity: Entity, tick: u16, position: Vec3) {
    let current = app.world().resource::<LocalTimeline>().tick().0;
    // Move the timeline to the sample tick before running PhysicsSchedule so the
    // lag-compensation plugin records the pose at the intended historical tick.
    app.world_mut()
        .resource_mut::<LocalTimeline>()
        .apply_delta(tick as i16 - current as i16);
    app.world_mut()
        .entity_mut(entity)
        .insert(Position(position));
    app.world_mut().run_schedule(PhysicsSchedule);
}

#[cfg(test)]
fn record_lag_compensation_ray_reports(
    lag_query: LagCompensationSpatialQuery,
    mut reports: ResMut<LagRayReports>,
) {
    let mut historical_filter = SpatialQueryFilter::default();
    reports.historical_hit = lag_query
        .cast_ray(
            // Current tick is 3, so this asks for the collider state at tick 0.
            InterpolationDelay {
                delay: PositiveTickDelta::lit("3"),
            },
            Vec3::ZERO,
            Dir3::X,
            10.0,
            true,
            &mut historical_filter,
        )
        .map(|hit| hit.entity);

    let mut current_filter = SpatialQueryFilter::default();
    reports.current_miss = lag_query
        .cast_ray(
            // Current tick is 3, so this asks for tick 2, where the target moved.
            InterpolationDelay {
                delay: PositiveTickDelta::lit("1"),
            },
            Vec3::ZERO,
            Dir3::X,
            10.0,
            true,
            &mut current_filter,
        )
        .is_none();
}

#[cfg(test)]
fn record_interpolated_lag_ray_report(
    lag_query: LagCompensationSpatialQuery,
    mut report: ResMut<InterpolatedLagRayReport>,
) {
    let mut filter = SpatialQueryFilter::default();
    let hit = lag_query.cast_ray(
        // Fractional delays verify interpolation instead of only exact tick lookup.
        InterpolationDelay {
            delay: PositiveTickDelta::lit("0.5"),
        },
        Vec3::ZERO,
        Dir3::X,
        10.0,
        true,
        &mut filter,
    );
    report.hit = hit.map(|hit| hit.entity);
    report.distance = hit.map(|hit| hit.distance);
}

// ── Multi-client e2e test ──────────────────────────────────────────────

fn main() {}
