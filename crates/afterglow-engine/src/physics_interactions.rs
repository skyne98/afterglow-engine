use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    core::{identity::StableEntityId, schedule::AfterglowSet},
    network::{NetworkTransformInterpolationBuffer, NetworkTransformSample},
};

use super::physics_grabbing_spring::apply_grab_spring_forces;

#[derive(Resource, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct PhysicsInteractionConfig {
    pub max_grab_distance: f32,
}

#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub struct PhysicsInteractionTick(pub u32);

#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct PhysicsBreakable {
    pub health: f32,
    pub impact_threshold: f32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct PhysicsKinematicRemote {
    pub interpolation_delay: u32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct PhysicsGrabbed {
    pub grabbed_by: StableEntityId,
    pub link_distance: f32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct PhysicsGrabbedState {
    pub grabbed_by: StableEntityId,
    pub link_distance: f32,
    pub authoritative_tick: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Message, Reflect)]
pub struct PhysicsImpactEvent {
    pub entity: Entity,
    pub other: Entity,
    pub relative_speed: f32,
    pub contact_point: Vec3,
    pub contact_normal: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Message, Reflect)]
pub struct PhysicsBreakEvent {
    pub entity: Entity,
    pub stable_id: StableEntityId,
}

#[derive(Clone, Debug, PartialEq, Message, Serialize, Deserialize)]
pub struct PhysicsGrabCommand {
    pub player: StableEntityId,
    pub tick: u32,
    pub target: StableEntityId,
}

#[derive(Clone, Debug, PartialEq, Message, Serialize, Deserialize)]
pub struct PhysicsReleaseCommand {
    pub player: StableEntityId,
    pub tick: u32,
    pub target: StableEntityId,
}

type PhysicsInteractionBody<'a> = (
    Entity,
    &'a StableEntityId,
    &'a Transform,
    Option<&'a PhysicsGrabbed>,
);

impl Default for PhysicsInteractionConfig {
    fn default() -> Self {
        Self {
            max_grab_distance: 3.0,
        }
    }
}

impl PhysicsBreakable {
    pub const fn new(health: f32, impact_threshold: f32) -> Self {
        Self {
            health,
            impact_threshold,
        }
    }
}

impl PhysicsKinematicRemote {
    pub const fn new(interpolation_delay: u32) -> Self {
        Self {
            interpolation_delay,
        }
    }
}

pub(crate) fn register_physics_interaction_api(app: &mut App) {
    app.init_resource::<PhysicsInteractionConfig>()
        .init_resource::<PhysicsInteractionTick>()
        .register_type::<PhysicsInteractionConfig>()
        .register_type::<PhysicsInteractionTick>()
        .register_type::<PhysicsBreakable>()
        .register_type::<PhysicsKinematicRemote>()
        .register_type::<PhysicsGrabbed>()
        .register_type::<PhysicsGrabbedState>()
        .add_message::<PhysicsImpactEvent>()
        .add_message::<PhysicsBreakEvent>()
        .add_message::<PhysicsGrabCommand>()
        .add_message::<PhysicsReleaseCommand>()
        .add_systems(
            FixedUpdate,
            (
                advance_physics_interaction_tick,
                apply_physics_grab_release_commands,
                apply_grab_spring_forces,
                resolve_physics_breakable_impacts,
                sync_kinematic_remote_observers,
            )
                .chain()
                .in_set(AfterglowSet::Simulate),
        );
}

fn advance_physics_interaction_tick(mut tick: ResMut<PhysicsInteractionTick>) {
    tick.0 = tick.0.saturating_add(1);
}

fn apply_physics_grab_release_commands(
    mut commands: Commands,
    config: Res<PhysicsInteractionConfig>,
    tick: Res<PhysicsInteractionTick>,
    mut grabs: MessageReader<PhysicsGrabCommand>,
    mut releases: MessageReader<PhysicsReleaseCommand>,
    bodies: Query<PhysicsInteractionBody>,
) {
    for grab in grabs.read() {
        let Some(player_position) = body_position(&bodies, grab.player) else {
            continue;
        };
        let Some((target, target_position, grabbed)) = body_target(&bodies, grab.target) else {
            continue;
        };
        if grabbed.is_some_and(|grabbed| grabbed.grabbed_by != grab.player) {
            continue;
        }
        let link_distance = player_position.distance(target_position);
        if link_distance > config.max_grab_distance.max(0.0) {
            continue;
        }
        let grabbed = PhysicsGrabbed {
            grabbed_by: grab.player,
            link_distance,
        };
        commands.entity(target).insert((
            grabbed,
            PhysicsGrabbedState {
                grabbed_by: grab.player,
                link_distance,
                authoritative_tick: tick.0.max(grab.tick),
            },
        ));
    }

    for release in releases.read() {
        let Some((target, _, grabbed)) = body_target(&bodies, release.target) else {
            continue;
        };
        if grabbed.is_some_and(|grabbed| grabbed.grabbed_by == release.player) {
            commands
                .entity(target)
                .remove::<(PhysicsGrabbed, PhysicsGrabbedState)>();
        }
    }
}

fn resolve_physics_breakable_impacts(
    mut impacts: MessageReader<PhysicsImpactEvent>,
    mut breaks: MessageWriter<PhysicsBreakEvent>,
    mut breakables: Query<(Entity, &mut PhysicsBreakable, Option<&StableEntityId>)>,
) {
    for impact in impacts.read() {
        apply_impact_to_breakable(
            impact.entity,
            impact.relative_speed,
            &mut breakables,
            &mut breaks,
        );
        if impact.other != impact.entity {
            apply_impact_to_breakable(
                impact.other,
                impact.relative_speed,
                &mut breakables,
                &mut breaks,
            );
        }
    }
}

fn sync_kinematic_remote_observers(
    mut commands: Commands,
    tick: Res<PhysicsInteractionTick>,
    mut observers: Query<(
        Entity,
        &PhysicsKinematicRemote,
        &Transform,
        Option<&mut NetworkTransformInterpolationBuffer>,
    )>,
) {
    for (entity, remote, transform, interpolation) in &mut observers {
        let sample = NetworkTransformSample::new(tick.0, transform.translation, transform.rotation);
        if let Some(mut interpolation) = interpolation {
            interpolation.delay_ticks = remote.interpolation_delay;
            interpolation.push_sample(sample);
            continue;
        }
        let mut interpolation = NetworkTransformInterpolationBuffer::with_sample(sample);
        interpolation.delay_ticks = remote.interpolation_delay;
        commands.entity(entity).insert(interpolation);
    }
}

fn body_position(
    bodies: &Query<PhysicsInteractionBody>,
    stable_id: StableEntityId,
) -> Option<Vec3> {
    bodies
        .iter()
        .find_map(|(_, id, transform, _)| (*id == stable_id).then_some(transform.translation))
}

fn body_target(
    bodies: &Query<PhysicsInteractionBody>,
    stable_id: StableEntityId,
) -> Option<(Entity, Vec3, Option<PhysicsGrabbed>)> {
    bodies.iter().find_map(|(entity, id, transform, grabbed)| {
        (*id == stable_id).then_some((entity, transform.translation, grabbed.copied()))
    })
}

fn apply_impact_to_breakable(
    entity: Entity,
    relative_speed: f32,
    breakables: &mut Query<(Entity, &mut PhysicsBreakable, Option<&StableEntityId>)>,
    breaks: &mut MessageWriter<PhysicsBreakEvent>,
) {
    if !relative_speed.is_finite() || relative_speed <= 0.0 {
        return;
    }
    let Ok((entity, mut breakable, stable_id)) = breakables.get_mut(entity) else {
        return;
    };
    if breakable.health <= 0.0 || relative_speed < breakable.impact_threshold.max(0.0) {
        return;
    }
    breakable.health = (breakable.health - relative_speed).max(0.0);
    if breakable.health == 0.0 {
        breaks.write(PhysicsBreakEvent {
            entity,
            stable_id: stable_id.copied().unwrap_or(StableEntityId::INVALID),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AfterglowCorePlugin;

    const PLAYER: StableEntityId = StableEntityId::from_raw(91);
    const TARGET: StableEntityId = StableEntityId::from_raw(92);

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AfterglowCorePlugin,
            super::super::AfterglowPhysicsPlugin,
        ));
        app.finish();
        app.cleanup();
        app
    }

    #[test]
    fn physics_plugin_registers_interaction_messages_and_resources() {
        let app = app();

        assert!(app.world().contains_resource::<PhysicsInteractionConfig>());
        assert!(app.world().contains_resource::<PhysicsInteractionTick>());
        assert!(
            app.world()
                .contains_resource::<Messages<PhysicsImpactEvent>>()
        );
        assert!(
            app.world()
                .contains_resource::<Messages<PhysicsBreakEvent>>()
        );
        assert!(
            app.world()
                .contains_resource::<Messages<PhysicsGrabCommand>>()
        );
        assert!(
            app.world()
                .contains_resource::<Messages<PhysicsReleaseCommand>>()
        );
    }

    #[test]
    fn breakable_ignores_impacts_below_threshold() {
        let mut app = app();
        let entity = app
            .world_mut()
            .spawn((
                TARGET,
                PhysicsBreakable::new(5.0, 2.0),
                Transform::default(),
            ))
            .id();
        write_impact(&mut app, entity, 1.0);

        app.world_mut().run_schedule(FixedUpdate);

        assert_eq!(
            app.world().get::<PhysicsBreakable>(entity).unwrap().health,
            5.0
        );
        assert!(
            app.world_mut()
                .resource_mut::<Messages<PhysicsBreakEvent>>()
                .drain()
                .next()
                .is_none()
        );
    }

    #[test]
    fn breakable_emits_break_once_when_health_reaches_zero() {
        let mut app = app();
        let entity = app
            .world_mut()
            .spawn((
                TARGET,
                PhysicsBreakable::new(3.0, 1.0),
                Transform::default(),
            ))
            .id();
        write_impact(&mut app, entity, 3.5);
        write_impact(&mut app, entity, 3.5);

        app.world_mut().run_schedule(FixedUpdate);

        assert_eq!(
            app.world().get::<PhysicsBreakable>(entity).unwrap().health,
            0.0
        );
        let breaks = app
            .world_mut()
            .resource_mut::<Messages<PhysicsBreakEvent>>()
            .drain()
            .collect::<Vec<_>>();
        assert_eq!(breaks.len(), 1);
        assert_eq!(breaks[0].stable_id, TARGET);
    }

    #[test]
    fn grab_command_creates_authoritative_grabbed_state() {
        let mut app = app();
        spawn_grab_pair(&mut app, 1.5);
        write_grab(&mut app, 7);

        app.world_mut().run_schedule(FixedUpdate);

        let target = entity_with_id(&mut app, TARGET);
        let grabbed = app.world().get::<PhysicsGrabbed>(target).unwrap();
        let state = app.world().get::<PhysicsGrabbedState>(target).unwrap();
        assert_eq!(grabbed.grabbed_by, PLAYER);
        assert_eq!(state.grabbed_by, PLAYER);
        assert_eq!(state.authoritative_tick, 7);
        assert!((state.link_distance - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn out_of_range_grab_is_rejected() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<PhysicsInteractionConfig>()
            .max_grab_distance = 1.0;
        spawn_grab_pair(&mut app, 2.0);
        write_grab(&mut app, 1);

        app.world_mut().run_schedule(FixedUpdate);

        let target = entity_with_id(&mut app, TARGET);
        assert!(app.world().get::<PhysicsGrabbed>(target).is_none());
        assert!(app.world().get::<PhysicsGrabbedState>(target).is_none());
    }

    #[test]
    fn release_command_removes_matching_grabbed_state() {
        let mut app = app();
        spawn_grab_pair(&mut app, 1.0);
        write_grab(&mut app, 1);
        app.world_mut().run_schedule(FixedUpdate);
        write_release(&mut app, 2);
        app.world_mut().run_schedule(FixedUpdate);

        let target = entity_with_id(&mut app, TARGET);
        assert!(app.world().get::<PhysicsGrabbed>(target).is_none());
        assert!(app.world().get::<PhysicsGrabbedState>(target).is_none());
    }

    #[test]
    fn kinematic_remote_writes_interpolation_sample_each_fixed_tick() {
        let mut app = app();
        let entity = app
            .world_mut()
            .spawn((
                PhysicsKinematicRemote::new(4),
                Transform::from_xyz(2.0, 3.0, 4.0),
            ))
            .id();

        app.world_mut().run_schedule(FixedUpdate);

        let buffer = app
            .world()
            .get::<NetworkTransformInterpolationBuffer>(entity)
            .unwrap();
        assert_eq!(buffer.delay_ticks, 4);
        // After one FixedUpdate, the buffer should have 1 sample (index 0).
        assert_eq!(
            buffer.sample_at(0).unwrap().translation,
            Vec3::new(2.0, 3.0, 4.0)
        );
    }

    fn spawn_grab_pair(app: &mut App, distance: f32) {
        app.world_mut()
            .spawn((PLAYER, Transform::from_xyz(0.0, 0.0, 0.0)));
        app.world_mut()
            .spawn((TARGET, Transform::from_xyz(distance, 0.0, 0.0)));
    }

    fn entity_with_id(app: &mut App, stable_id: StableEntityId) -> Entity {
        app.world_mut()
            .query::<(Entity, &StableEntityId)>()
            .iter(app.world())
            .find_map(|(entity, id)| (*id == stable_id).then_some(entity))
            .unwrap()
    }

    fn write_impact(app: &mut App, entity: Entity, relative_speed: f32) {
        app.world_mut()
            .resource_mut::<Messages<PhysicsImpactEvent>>()
            .write(PhysicsImpactEvent {
                entity,
                other: entity,
                relative_speed,
                contact_point: Vec3::ZERO,
                contact_normal: Vec3::Y,
            });
    }

    fn write_grab(app: &mut App, tick: u32) {
        app.world_mut()
            .resource_mut::<Messages<PhysicsGrabCommand>>()
            .write(PhysicsGrabCommand {
                player: PLAYER,
                tick,
                target: TARGET,
            });
    }

    fn write_release(app: &mut App, tick: u32) {
        app.world_mut()
            .resource_mut::<Messages<PhysicsReleaseCommand>>()
            .write(PhysicsReleaseCommand {
                player: PLAYER,
                tick,
                target: TARGET,
            });
    }
}
