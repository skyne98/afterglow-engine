use super::*;
use crate::{
    core::AfterglowCorePlugin,
    physics::{AfterglowPhysicsPlugin, PhysicsBody, PhysicsCollider},
};
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

fn app_with_dt(seconds: f64) -> App {
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
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(seconds));
    app
}

fn spawn_floor(app: &mut App) {
    app.world_mut().spawn((
        PhysicsBody::static_body(),
        PhysicsCollider::cuboid(Vec3::new(200.0, 0.2, 200.0)),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
}

#[test]
fn plugin_authors_empty_effect_stack_for_controller() {
    let mut app = app_with_dt(1.0 / 60.0);
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController::new(),
            Transform::from_xyz(0.0, 0.9, 0.0),
        ))
        .id();

    app.update();

    assert!(app.world().get::<FirstPersonEffectStack>(player).is_some());
}

#[test]
fn speed_and_jump_effects_modify_config_and_expire_on_fixed_ticks() {
    let base = FirstPersonControllerConfig::default();
    let mut stack = FirstPersonEffectStack::default();
    stack.push(FirstPersonEffect::speed_multiplier(0.5, 2));
    stack.push(FirstPersonEffect::jump_multiplier(1.5, 2));

    let affected = stack.effective_config(&base);
    assert_eq!(affected.ground_speed, base.ground_speed * 0.5);
    assert_eq!(affected.sprint_speed, base.sprint_speed * 0.5);
    assert_eq!(affected.jump_speed, base.jump_speed * 1.5);

    stack.tick_fixed();
    assert!(!stack.is_empty());
    stack.tick_fixed();
    assert!(stack.is_empty());
    assert_eq!(stack.effective_config(&base), base);
}

#[test]
fn speed_multipliers_stack_multiplicatively_and_ignore_invalid_values() {
    let base = FirstPersonControllerConfig::default();
    let mut stack = FirstPersonEffectStack::default();
    stack.push(FirstPersonEffect::speed_multiplier(0.5, 4));
    stack.push(FirstPersonEffect::speed_multiplier(0.25, 4));
    stack.push(FirstPersonEffect::speed_multiplier(f32::NAN, 4));

    let affected = stack.effective_config(&base);
    assert_eq!(affected.ground_speed, base.ground_speed * 0.125);
}

#[test]
fn look_multiplier_affects_render_rate_look_without_fixed_tick() {
    let mut app = app_with_dt(1.0 / 60.0);
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, config.height(ControllerStance::Standing) * 0.5, 0.0),
        ))
        .id();
    app.update();
    app.world_mut()
        .get_mut::<FirstPersonEffectStack>(player)
        .unwrap()
        .push(FirstPersonEffect::look_multiplier(0.25, 10));

    *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
        TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 240.0));
    test_input::set_input(
        &mut app,
        player,
        test_input::command(&[("look.x", 16.0)], &[]),
    );
    app.update();

    let motor = app.world().get::<FirstPersonMotorState>(player).unwrap();
    assert!((motor.yaw + 16.0 * config.look_sensitivity.x * 0.25).abs() < 0.001);
}

#[test]
fn root_speed_effect_blocks_new_horizontal_movement_until_expired() {
    let mut app = app_with_dt(1.0 / 60.0);
    let config = FirstPersonControllerConfig::default();
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, config.height(ControllerStance::Standing) * 0.5, 0.0),
        ))
        .id();
    spawn_floor(&mut app);
    app.update();
    app.world_mut()
        .get_mut::<FirstPersonEffectStack>(player)
        .unwrap()
        .push(FirstPersonEffect::speed_multiplier(0.0, 1));

    test_input::set_input(
        &mut app,
        player,
        test_input::command(&[("move.y", 1.0)], &[]),
    );
    let before = app.world().get::<Transform>(player).unwrap().translation;
    app.update();
    let rooted = app.world().get::<Transform>(player).unwrap().translation;
    app.update();
    let released = app.world().get::<Transform>(player).unwrap().translation;

    assert!((rooted.z - before.z).abs() < 0.001);
    assert!(released.z < rooted.z - 0.001);
}
