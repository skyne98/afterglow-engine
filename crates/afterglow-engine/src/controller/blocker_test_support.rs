use super::*;
use crate::{
    core::AfterglowCorePlugin,
    input::AfterglowAction,
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use leafwing_input_manager::action_state::ActionState;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub(super) struct Sample {
    pub(super) position: Vec3,
    pub(super) climbing: bool,
    pub(super) intent_forward_speed: f32,
    pub(super) actual_forward_speed: f32,
    pub(super) actual_side_speed: f32,
}

pub(super) fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AfterglowCorePlugin,
        AfterglowPhysicsPlugin,
        AfterglowFirstPersonControllerPlugin,
    ));
    app.finish();
    app.cleanup();
    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0));
    app
}

pub(super) fn move_right_command() -> ActionState<AfterglowAction> {
    test_input::command(&[("move.x", 1.0)], &[])
}

pub(super) fn move_forward_command() -> ActionState<AfterglowAction> {
    test_input::command(&[("move.y", 1.0)], &[])
}

pub(super) fn sprint_forward_command() -> ActionState<AfterglowAction> {
    test_input::command(&[("move.y", 1.0)], &[AfterglowAction::Sprint])
}

pub(super) fn spawn_static_box(app: &mut App, size: Vec3, transform: Transform) {
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(size),
        transform,
    ));
}

pub(super) fn sample(app: &App, player: Entity) -> Sample {
    let position = app.world().get::<Transform>(player).unwrap().translation;
    let state = app.world().get::<FirstPersonMotorState>(player).unwrap();
    let local_speed = body::local_speeds_from_velocity(state);
    Sample {
        position,
        climbing: state.climbing,
        intent_forward_speed: state.forward_speed,
        actual_forward_speed: local_speed.x,
        actual_side_speed: local_speed.y,
    }
}

pub(super) fn range(samples: &[Sample], axis: fn(Vec3) -> f32) -> f32 {
    let min = samples
        .iter()
        .map(|sample| axis(sample.position))
        .fold(f32::INFINITY, f32::min);
    let max = samples
        .iter()
        .map(|sample| axis(sample.position))
        .fold(f32::NEG_INFINITY, f32::max);
    max - min
}
