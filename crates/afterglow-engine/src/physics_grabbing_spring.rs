use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;

use crate::{core::identity::StableEntityId, physics::PhysicsGrabbed};

#[derive(Resource, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct PhysicsGrabSpringConfig {
    pub stiffness: f32,
    pub damping: f32,
    pub max_acceleration: f32,
}

impl Default for PhysicsGrabSpringConfig {
    fn default() -> Self {
        Self {
            stiffness: 40.0,
            damping: 8.0,
            max_acceleration: 80.0,
        }
    }
}

pub(crate) fn register_physics_grabbing_spring_api(app: &mut App) {
    app.init_resource::<PhysicsGrabSpringConfig>()
        .register_type::<PhysicsGrabSpringConfig>();
}

type GrabSpringBody<'a> = (
    &'a StableEntityId,
    &'a Transform,
    Option<&'a LinearVelocity>,
);
type GrabSpringTarget<'a> = (
    Entity,
    &'a PhysicsGrabbed,
    &'a Transform,
    Option<&'a mut LinearVelocity>,
);

pub(crate) fn apply_grab_spring_forces(
    mut commands: Commands,
    config: Res<PhysicsGrabSpringConfig>,
    fixed_time: Res<Time<Fixed>>,
    mut bodies: ParamSet<(Query<GrabSpringBody>, Query<GrabSpringTarget>)>,
) {
    let delta_seconds = fixed_time.timestep().as_secs_f32();
    if !delta_seconds.is_finite() || delta_seconds <= 0.0 {
        return;
    }

    let body_states = bodies
        .p0()
        .iter()
        .map(|(id, transform, velocity)| {
            (
                *id,
                transform.translation,
                velocity.map_or(Vec3::ZERO, |v| v.0),
            )
        })
        .collect::<Vec<_>>();

    for (entity, grabbed, transform, velocity) in &mut bodies.p1() {
        let Some((_, grabber_position, grabber_velocity)) = body_states
            .iter()
            .find(|(id, _, _)| *id == grabbed.grabbed_by)
        else {
            continue;
        };
        let target_velocity = velocity.as_ref().map_or(Vec3::ZERO, |v| v.0);
        let delta_velocity = spring_acceleration(
            &config,
            *grabber_position,
            *grabber_velocity,
            transform.translation,
            target_velocity,
            grabbed.link_distance,
        ) * delta_seconds;

        if !is_finite_vec3(delta_velocity) || delta_velocity == Vec3::ZERO {
            continue;
        }
        if let Some(mut velocity) = velocity {
            velocity.0 += delta_velocity;
        } else {
            commands
                .entity(entity)
                .insert(LinearVelocity(delta_velocity));
        }
    }
}

fn spring_acceleration(
    config: &PhysicsGrabSpringConfig,
    grabber_position: Vec3,
    grabber_velocity: Vec3,
    target_position: Vec3,
    target_velocity: Vec3,
    link_distance: f32,
) -> Vec3 {
    let stiffness = non_negative_finite(config.stiffness);
    let damping = non_negative_finite(config.damping);
    let max_acceleration = non_negative_finite(config.max_acceleration);
    if max_acceleration == 0.0 || (stiffness == 0.0 && damping == 0.0) {
        return Vec3::ZERO;
    }
    if !link_distance.is_finite() {
        return Vec3::ZERO;
    }

    let offset = target_position - grabber_position;
    if !is_finite_vec3(offset) {
        return Vec3::ZERO;
    }
    let distance = offset.length();
    if !distance.is_finite() || distance <= f32::EPSILON {
        return Vec3::ZERO;
    }

    let direction = offset / distance;
    let radial_speed = if is_finite_vec3(target_velocity) && is_finite_vec3(grabber_velocity) {
        (target_velocity - grabber_velocity).dot(direction)
    } else {
        0.0
    };
    let link_distance = link_distance.max(0.0);
    let acceleration =
        direction * (-stiffness * (distance - link_distance) - damping * radial_speed);
    if !is_finite_vec3(acceleration) {
        return Vec3::ZERO;
    }
    acceleration.clamp_length_max(max_acceleration)
}

fn non_negative_finite(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn is_finite_vec3(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::AfterglowCorePlugin,
        physics::{AfterglowPhysicsPlugin, PhysicsGrabCommand},
    };

    const DT: f32 = 1.0 / 60.0;
    const PLAYER: StableEntityId = StableEntityId::from_raw(301);
    const TARGET: StableEntityId = StableEntityId::from_raw(302);

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AfterglowCorePlugin, AfterglowPhysicsPlugin));
        app.finish();
        app.cleanup();
        app
    }

    #[test]
    fn physics_plugin_registers_grab_spring_config() {
        let app = app();

        assert!(app.world().contains_resource::<PhysicsGrabSpringConfig>());
    }

    #[test]
    fn spring_holds_object_at_link_distance() {
        let mut app = app();
        let target = spawn_grabbed_pair(&mut app, 1.0, 1.0);

        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().get::<LinearVelocity>(target).is_none());
    }

    #[test]
    fn spring_pulls_stretched_object_toward_player() {
        let mut app = app();
        let target = spawn_grabbed_pair(&mut app, 2.0, 1.0);

        app.world_mut().run_schedule(FixedUpdate);

        let velocity = app.world().get::<LinearVelocity>(target).unwrap().0;
        assert!(velocity.x < 0.0);
        assert_eq!(velocity.y, 0.0);
        assert_eq!(velocity.z, 0.0);
    }

    #[test]
    fn spring_pushes_compressed_object_away_from_player() {
        let mut app = app();
        let target = spawn_grabbed_pair(&mut app, 0.5, 1.0);

        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().get::<LinearVelocity>(target).unwrap().0.x > 0.0);
    }

    #[test]
    fn damping_opposes_radial_motion() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<PhysicsGrabSpringConfig>()
            .stiffness = 0.0;
        let target = spawn_grabbed_pair(&mut app, 2.0, 1.0);
        app.world_mut()
            .entity_mut(target)
            .insert(LinearVelocity(Vec3::X * 3.0));

        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().get::<LinearVelocity>(target).unwrap().0.x < 3.0);
    }

    #[test]
    fn max_acceleration_clamps_spring_velocity_delta() {
        let mut app = app();
        *app.world_mut().resource_mut::<PhysicsGrabSpringConfig>() = PhysicsGrabSpringConfig {
            stiffness: 10_000.0,
            damping: 0.0,
            max_acceleration: 6.0,
        };
        let target = spawn_grabbed_pair(&mut app, 10.0, 1.0);

        app.world_mut().run_schedule(FixedUpdate);

        let speed = app
            .world()
            .get::<LinearVelocity>(target)
            .unwrap()
            .0
            .length();
        assert!((speed - 6.0 * DT).abs() <= f32::EPSILON);
    }

    #[test]
    fn missing_grabber_produces_no_spring_velocity() {
        let mut app = app();
        let target = app
            .world_mut()
            .spawn((
                TARGET,
                Transform::from_xyz(2.0, 0.0, 0.0),
                PhysicsGrabbed {
                    grabbed_by: PLAYER,
                    link_distance: 1.0,
                },
            ))
            .id();

        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().get::<LinearVelocity>(target).is_none());
    }

    #[test]
    fn invalid_spring_values_do_not_create_velocity() {
        let mut app = app();
        *app.world_mut().resource_mut::<PhysicsGrabSpringConfig>() = PhysicsGrabSpringConfig {
            stiffness: f32::INFINITY,
            damping: f32::NAN,
            max_acceleration: 80.0,
        };
        let target = spawn_grabbed_pair(&mut app, 2.0, 1.0);

        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().get::<LinearVelocity>(target).is_none());
    }

    #[test]
    fn zero_distance_grab_is_ignored_safely() {
        let mut app = app();
        let target = spawn_grabbed_pair(&mut app, 0.0, 1.0);

        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().get::<LinearVelocity>(target).is_none());
    }

    #[test]
    fn command_created_grab_is_sprung_on_same_fixed_tick() {
        let mut app = app();
        app.world_mut()
            .spawn((PLAYER, Transform::from_xyz(0.0, 0.0, 0.0)));
        let target = app
            .world_mut()
            .spawn((TARGET, Transform::from_xyz(1.0, 0.0, 0.0)))
            .id();
        app.world_mut()
            .resource_mut::<Messages<PhysicsGrabCommand>>()
            .write(PhysicsGrabCommand {
                player: PLAYER,
                tick: 1,
                target: TARGET,
            });
        app.world_mut()
            .entity_mut(target)
            .insert(LinearVelocity(Vec3::X * 3.0));

        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().get::<LinearVelocity>(target).unwrap().0.x < 3.0);
    }

    fn spawn_grabbed_pair(app: &mut App, target_distance: f32, link_distance: f32) -> Entity {
        app.world_mut()
            .spawn((PLAYER, Transform::from_xyz(0.0, 0.0, 0.0)));
        app.world_mut()
            .spawn((
                TARGET,
                Transform::from_xyz(target_distance, 0.0, 0.0),
                PhysicsGrabbed {
                    grabbed_by: PLAYER,
                    link_distance,
                },
            ))
            .id()
    }
}
