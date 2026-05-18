use super::{blocker_test_support::*, *};

#[test]
fn controller_clips_failed_low_blocker_intent_after_sprint_reset() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = config.max_step_height + 0.14;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 5.5),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(1.2, obstacle_height, 0.55),
        Transform::from_xyz(0.0, obstacle_height * 0.5, 0.2),
    );

    app.update();
    for _ in 0..24 {
        test_input::set_input(&mut app, player, sprint_forward_command());
        app.update();
    }
    for _ in 0..60 {
        test_input::clear_input(&mut app, player);
        app.update();
    }
    assert!(
        app.world()
            .get::<FirstPersonMotorState>(player)
            .unwrap()
            .forward_speed
            .abs()
            < 0.01,
        "sprint warmup did not fully reset before blocker test"
    );

    let mut samples = Vec::with_capacity(160);
    for _ in 0..160 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[100..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "low blocker triggered stair climbing after sprint reset: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "low blocker caused left/right jitter after sprint reset: {samples:?}"
    );
    assert!(
        settled
            .iter()
            .all(|sample| sample.actual_forward_speed.abs() < 0.05),
        "low blocker kept actual forward motion after sprint reset: {samples:?}"
    );
    assert!(
        settled
            .iter()
            .all(|sample| (sample.intent_forward_speed - 5.0).abs() < 0.1),
        "failed low-blocker intent is not sustained at ground_speed (HPL2 does not clip intent on failed steps): {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_low_centered_blocker() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = config.max_step_height + 0.14;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, 1.2),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(config.body_radius * 2.0, obstacle_height, 0.55),
        Transform::from_xyz(0.0, obstacle_height * 0.5, 0.2),
    );

    app.update();
    let mut samples = Vec::with_capacity(121);
    for _ in 0..120 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[60..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "low centered blocker triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "low centered blocker caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "low centered blocker caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.006,
        "low centered blocker caused blocked-axis jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_thick_knee_wall() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = 0.5;
    let wall_half_depth = 2.0;
    let wall_center_z = 0.0;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, wall_center_z + wall_half_depth + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, obstacle_height, wall_half_depth),
        Transform::from_xyz(0.0, obstacle_height * 0.5, wall_center_z),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "thick knee wall triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "thick knee wall caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "thick knee wall caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "thick knee wall caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_chest_high_barrier() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = 0.9;
    let wall_half_depth = 2.0;
    let wall_center_z = 0.0;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, wall_center_z + wall_half_depth + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, obstacle_height, wall_half_depth),
        Transform::from_xyz(0.0, obstacle_height * 0.5, wall_center_z),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "chest-high barrier triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "chest-high barrier caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "chest-high barrier caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "chest-high barrier caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_waist_high_barrier() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let obstacle_height = 0.36; // exactly at max_step_height
    let wall_half_depth = 2.0;
    let wall_center_z = 0.0;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(0.0, half_height, wall_center_z + wall_half_depth + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 16.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, obstacle_height, wall_half_depth),
        Transform::from_xyz(0.0, obstacle_height * 0.5, wall_center_z),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "waist-high barrier at max_step_height triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "waist-high barrier caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "waist-high barrier caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "waist-high barrier caused forward/back jitter: {samples:?}"
    );
}
