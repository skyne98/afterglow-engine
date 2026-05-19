pub mod avian {
    pub use avian3d::prelude::*;
}

use avian3d::prelude::{
    AngularVelocity, Collider, Gravity, LinearVelocity, PhysicsPlugins, RigidBody,
};
use bevy::prelude::*;

#[path = "physics_grabbing_spring.rs"]
mod physics_grabbing_spring;
pub use physics_grabbing_spring::*;

#[path = "physics_interactions.rs"]
mod physics_interactions;
pub use physics_interactions::*;

#[derive(Resource, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct AfterglowPhysicsConfig {
    pub gravity: Vec3,
}

impl Default for AfterglowPhysicsConfig {
    fn default() -> Self {
        Self {
            gravity: Vec3::Y * -9.81,
        }
    }
}

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub struct PhysicsBody {
    pub kind: PhysicsBodyKind,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub enum PhysicsBodyKind {
    #[default]
    Dynamic,
    Static,
    Kinematic,
}

#[derive(Component, Clone, Debug, PartialEq, Reflect)]
pub enum PhysicsCollider {
    Cuboid { size: Vec3 },
    Sphere { radius: f32 },
    Cylinder { radius: f32, height: f32 },
    Capsule { radius: f32, length: f32 },
    ConvexHull { points: Vec<Vec3> },
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct PhysicsVelocity {
    pub linear: Vec3,
    pub angular: Vec3,
}

pub struct AfterglowPhysicsPlugin;

impl Plugin for AfterglowPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default())
            .init_resource::<AfterglowPhysicsConfig>()
            .register_type::<AfterglowPhysicsConfig>()
            .register_type::<PhysicsBody>()
            .register_type::<PhysicsBodyKind>()
            .register_type::<PhysicsCollider>()
            .register_type::<PhysicsVelocity>()
            .add_systems(Startup, apply_physics_config)
            .add_systems(
                Update,
                (
                    apply_physics_config.run_if(resource_changed::<AfterglowPhysicsConfig>),
                    sync_physics_body_authoring,
                    sync_physics_collider_authoring,
                    sync_physics_velocity_authoring,
                )
                    .in_set(crate::core::schedule::AfterglowSet::Simulate),
            );
        physics_grabbing_spring::register_physics_grabbing_spring_api(app);
        physics_interactions::register_physics_interaction_api(app);
    }
}

impl PhysicsBody {
    pub const fn dynamic() -> Self {
        Self {
            kind: PhysicsBodyKind::Dynamic,
        }
    }

    pub const fn static_body() -> Self {
        Self {
            kind: PhysicsBodyKind::Static,
        }
    }

    pub const fn kinematic() -> Self {
        Self {
            kind: PhysicsBodyKind::Kinematic,
        }
    }

    /// Dynamic body with explicit mass in real-world kilograms.
    ///
    /// ```ignore
    /// commands.spawn((
    ///     PhysicsBody::with_mass(100.0),
    ///     PhysicsCollider::cuboid(Vec3::splat(0.5)),
    /// ));
    /// ```
    /// A 100 kg cube behaves like 100 kg: gravity pulls it with ~981 N,
    /// collisions require proportional force to move.
    pub fn with_mass(kg: f32) -> (Self, avian::Mass) {
        (Self::dynamic(), avian::Mass(kg))
    }

    /// Dynamic body with a real-world density.
    /// avian3d auto-computes mass from `ColliderDensity` + collider volume.
    ///
    /// ```ignore
    /// commands.spawn((
    ///     PhysicsBody::with_density(units::Density::STEEL),
    ///     PhysicsCollider::cuboid(Vec3::splat(1.0)),
    /// ));
    /// ```
    /// A 1 m³ steel cube gets `Mass(7800.0)` automatically.
    pub fn with_density(density: crate::units::Density) -> (Self, avian::ColliderDensity) {
        (Self::dynamic(), avian::ColliderDensity(density.0))
    }
}

// Convenience — a 1m³ steel block has a believable 7800 kg.
// A player-sized human-density capsule has ~70-80 kg.

impl From<PhysicsBodyKind> for RigidBody {
    fn from(kind: PhysicsBodyKind) -> Self {
        match kind {
            PhysicsBodyKind::Dynamic => RigidBody::Dynamic,
            PhysicsBodyKind::Static => RigidBody::Static,
            PhysicsBodyKind::Kinematic => RigidBody::Kinematic,
        }
    }
}

impl PhysicsCollider {
    pub const fn cuboid(size: Vec3) -> Self {
        Self::Cuboid { size }
    }

    pub const fn sphere(radius: f32) -> Self {
        Self::Sphere { radius }
    }

    pub const fn cylinder(radius: f32, height: f32) -> Self {
        Self::Cylinder { radius, height }
    }

    pub const fn capsule(radius: f32, length: f32) -> Self {
        Self::Capsule { radius, length }
    }

    pub fn convex_hull(points: Vec<Vec3>) -> Self {
        Self::ConvexHull { points }
    }

    fn to_avian(&self) -> Collider {
        match self {
            Self::Cuboid { size } => Collider::cuboid(size.x, size.y, size.z),
            Self::Sphere { radius } => Collider::sphere(*radius),
            Self::Cylinder { radius, height } => Collider::cylinder(*radius, *height),
            Self::Capsule { radius, length } => Collider::capsule(*radius, *length),
            Self::ConvexHull { points } => Collider::convex_hull(points.clone())
                .expect("PhysicsCollider::ConvexHull requires at least one valid 3D hull"),
        }
    }
}

impl PhysicsVelocity {
    pub const fn linear(linear: Vec3) -> Self {
        Self {
            linear,
            angular: Vec3::ZERO,
        }
    }

    pub const fn new(linear: Vec3, angular: Vec3) -> Self {
        Self { linear, angular }
    }
}

fn apply_physics_config(config: Res<AfterglowPhysicsConfig>, mut gravity: ResMut<Gravity>) {
    gravity.0 = config.gravity;
}

fn sync_physics_body_authoring(
    mut commands: Commands,
    bodies: Query<(Entity, &PhysicsBody), Changed<PhysicsBody>>,
) {
    for (entity, body) in &bodies {
        commands.entity(entity).insert(RigidBody::from(body.kind));
    }
}

fn sync_physics_collider_authoring(
    mut commands: Commands,
    colliders: Query<(Entity, &PhysicsCollider), Changed<PhysicsCollider>>,
) {
    for (entity, collider) in &colliders {
        commands.entity(entity).insert(collider.to_avian());
    }
}

fn sync_physics_velocity_authoring(
    mut commands: Commands,
    velocities: Query<(Entity, &PhysicsVelocity), Changed<PhysicsVelocity>>,
) {
    for (entity, velocity) in &velocities {
        commands.entity(entity).insert((
            LinearVelocity(velocity.linear),
            AngularVelocity(velocity.angular),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::{
            AfterglowCorePlugin,
            identity::{ChunkId, ChunkMembership, Persistent, StableEntityId},
        },
        persistence::AfterglowPersistencePlugin,
        world::{
            AfterglowWorldPlugin,
            lifecycle::{ChunkLifecycle, ChunkLifecycleRequests, ChunkLifecycleState},
        },
    };
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    const TEST_CHUNK: ChunkId = ChunkId::from_raw(77);
    const TEST_BODY: StableEntityId = StableEntityId::from_raw(7_700);

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AfterglowCorePlugin, AfterglowPhysicsPlugin));
        app.finish();
        app.cleanup();
        app
    }

    #[test]
    fn physics_plugin_registers_avian_and_engine_resources() {
        let app = app();

        assert!(app.world().contains_resource::<AfterglowPhysicsConfig>());
        assert!(app.world().contains_resource::<Gravity>());
    }

    #[test]
    fn physics_authoring_components_insert_avian_components() {
        let mut app = app();
        let entity = app
            .world_mut()
            .spawn((
                PhysicsBody::dynamic(),
                PhysicsCollider::sphere(0.5),
                PhysicsVelocity::linear(Vec3::X),
            ))
            .id();

        app.update();

        assert_eq!(
            app.world().get::<RigidBody>(entity),
            Some(&RigidBody::Dynamic)
        );
        assert!(app.world().get::<Collider>(entity).is_some());
        assert_eq!(
            app.world().get::<LinearVelocity>(entity),
            Some(&LinearVelocity(Vec3::X))
        );
    }

    #[test]
    fn physics_body_authoring_changes_replace_avian_body_kind() {
        let mut app = app();
        let entity = app.world_mut().spawn(PhysicsBody::dynamic()).id();

        app.update();
        assert_eq!(
            app.world().get::<RigidBody>(entity),
            Some(&RigidBody::Dynamic)
        );

        app.world_mut()
            .entity_mut(entity)
            .insert(PhysicsBody::static_body());
        app.update();

        assert_eq!(
            app.world().get::<RigidBody>(entity),
            Some(&RigidBody::Static)
        );
    }

    #[test]
    fn physics_velocity_authoring_changes_replace_avian_velocity() {
        let mut app = app();
        let entity = app.world_mut().spawn(PhysicsVelocity::linear(Vec3::X)).id();

        app.update();
        assert_eq!(
            app.world().get::<LinearVelocity>(entity),
            Some(&LinearVelocity(Vec3::X))
        );

        app.world_mut()
            .entity_mut(entity)
            .insert(PhysicsVelocity::new(Vec3::Y * 2.0, Vec3::Z * 3.0));
        app.update();

        assert_eq!(
            app.world().get::<LinearVelocity>(entity),
            Some(&LinearVelocity(Vec3::Y * 2.0))
        );
        assert_eq!(
            app.world().get::<AngularVelocity>(entity),
            Some(&AngularVelocity(Vec3::Z * 3.0))
        );
    }

    #[test]
    fn supported_collider_authoring_variants_insert_avian_colliders() {
        let mut app = app();
        let cuboid = app
            .world_mut()
            .spawn(PhysicsCollider::cuboid(Vec3::new(1.0, 2.0, 3.0)))
            .id();
        let sphere = app.world_mut().spawn(PhysicsCollider::sphere(0.5)).id();
        let cylinder = app
            .world_mut()
            .spawn(PhysicsCollider::cylinder(0.25, 1.8))
            .id();
        let capsule = app
            .world_mut()
            .spawn(PhysicsCollider::capsule(0.25, 1.5))
            .id();
        let hull = app
            .world_mut()
            .spawn(PhysicsCollider::convex_hull(vec![
                Vec3::new(-0.5, 0.0, -0.5),
                Vec3::new(0.5, 0.0, -0.5),
                Vec3::new(0.0, 0.0, 0.5),
                Vec3::new(0.0, 1.0, 0.0),
            ]))
            .id();

        app.update();

        assert!(app.world().get::<Collider>(cuboid).is_some());
        assert!(app.world().get::<Collider>(sphere).is_some());
        assert!(app.world().get::<Collider>(cylinder).is_some());
        assert!(app.world().get::<Collider>(capsule).is_some());
        assert!(app.world().get::<Collider>(hull).is_some());
    }

    #[test]
    fn physics_config_updates_avian_gravity() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<AfterglowPhysicsConfig>()
            .gravity = Vec3::new(0.0, -3.0, 0.0);

        app.update();

        assert_eq!(
            app.world().resource::<Gravity>().0,
            Vec3::new(0.0, -3.0, 0.0)
        );
    }

    #[test]
    fn dynamic_body_moves_under_gravity() {
        let mut app = app();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
        let entity = app
            .world_mut()
            .spawn((
                PhysicsBody::dynamic(),
                PhysicsCollider::sphere(0.5),
                Transform::from_xyz(0.0, 2.0, 0.0),
            ))
            .id();

        for _ in 0..20 {
            app.update();
        }

        assert!(app.world().get::<Transform>(entity).unwrap().translation.y < 2.0);
    }

    #[test]
    fn static_body_does_not_fall_under_gravity() {
        let mut app = app();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
        let entity = app
            .world_mut()
            .spawn((
                PhysicsBody::static_body(),
                PhysicsCollider::sphere(0.5),
                Transform::from_xyz(0.0, 2.0, 0.0),
            ))
            .id();

        for _ in 0..20 {
            app.update();
        }

        assert_eq!(
            app.world().get::<Transform>(entity).unwrap().translation.y,
            2.0
        );
    }

    #[test]
    fn runtime_plugins_register_physics() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::AfterglowRuntimePlugins));
        app.finish();
        app.cleanup();

        assert!(app.world().contains_resource::<AfterglowPhysicsConfig>());
        assert!(app.world().contains_resource::<Gravity>());
    }

    #[test]
    fn chunk_unload_despawns_physics_entities() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AfterglowCorePlugin,
            AfterglowPersistencePlugin,
            AfterglowWorldPlugin,
            AfterglowPhysicsPlugin,
        ));
        app.finish();
        app.cleanup();
        app.world_mut().spawn((
            Persistent,
            TEST_BODY,
            ChunkMembership::new(TEST_CHUNK),
            PhysicsBody::dynamic(),
            PhysicsCollider::sphere(0.5),
            Transform::from_xyz(0.0, 1.0, 0.0),
        ));

        app.world_mut()
            .resource_mut::<ChunkLifecycleRequests>()
            .request_load(TEST_CHUNK)
            .unwrap();
        app.update();
        app.world_mut()
            .resource_mut::<ChunkLifecycleRequests>()
            .request_spawned(TEST_CHUNK)
            .unwrap();
        app.update();
        app.world_mut()
            .resource_mut::<ChunkLifecycleRequests>()
            .request_unload(TEST_CHUNK)
            .unwrap();
        app.update();

        assert_eq!(
            app.world().resource::<ChunkLifecycle>().state(TEST_CHUNK),
            ChunkLifecycleState::Unloaded
        );
        assert!(
            app.world()
                .resource::<crate::core::identity::StableEntityRegistry>()
                .entity(TEST_BODY)
                .is_none()
        );
    }
}
