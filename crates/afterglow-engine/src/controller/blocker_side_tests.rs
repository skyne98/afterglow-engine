use super::{blocker_test_support::*, *};

#[test]
fn controller_does_not_jitter_sideways_against_low_blocker() {
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
            Transform::from_xyz(-0.2, half_height, 0.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(8.0, 0.2, 10.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(0.55, obstacle_height, 3.0),
        Transform::from_xyz(0.55, obstacle_height * 0.5, 0.0),
    );

    app.update();
    let mut samples = Vec::with_capacity(121);
    for _ in 0..120 {
        test_input::set_input(&mut app, player, move_right_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[60..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "low side blocker triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.006,
        "low side blocker caused blocked-axis jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.y) < 0.003,
        "low side blocker caused vertical jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.003,
        "low side blocker caused left/right jitter along the wall: {samples:?}"
    );
    assert!(
        settled
            .iter()
            .all(|sample| sample.actual_side_speed.abs() < 0.05),
        "low side blocker kept reporting side motion for camera/presentation: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_tallest_stair_side() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let step_height = 0.72;
    let step_half_width = 1.1;
    let side_face_x = -step_half_width;
    let step_z = -0.25;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(side_face_x - 2.0, half_height, step_z),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(28.0, 0.4, 28.0),
        Transform::from_xyz(0.0, -0.2, 0.0),
    );
    spawn_static_box(
        &mut app,
        Vec3::new(2.2, step_height, 0.55),
        Transform::from_xyz(0.0, step_height * 0.5, step_z),
    );

    app.update();
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        test_input::set_input(&mut app, player, move_right_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "tallest stair side face triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.006,
        "tallest stair side face caused blocked-axis jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.003,
        "tallest stair side face caused left/right jitter along the wall: {samples:?}"
    );
}

#[test]
fn centered_blocker_exact_stillness_check() {
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
    let mut samples = Vec::with_capacity(180);
    for _ in 0..180 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    let first = settled[0];
    assert!(
        settled.iter().all(|s| s.position == first.position),
        "centered blocker not exactly still: converged on ({}, {}, {})",
        first.position.x,
        first.position.y,
        first.position.z
    );
}

#[test]
fn offcenter_accent_box_exact_stillness_check() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let box_size = Vec3::new(1.5, 0.5, 3.0);
    let box_translation = Vec3::new(2.5, 0.25, -2.0);
    let box_front_z = box_translation.z + box_size.z * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(box_translation.x - 0.05, half_height, box_front_z + 2.0),
        ))
        .id();
    spawn_static_box(
        &mut app,
        Vec3::new(28.0, 0.4, 28.0),
        Transform::from_xyz(0.0, -0.2, 0.0),
    );
    spawn_static_box(
        &mut app,
        box_size,
        Transform::from_translation(box_translation),
    );

    app.update();
    let mut samples = Vec::with_capacity(300);
    for _ in 0..300 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[200..];
    let first = settled[0];
    assert!(
        settled.iter().all(|s| s.position == first.position),
        "off-center box not exactly still: converged on ({}, {}, {})",
        first.position.x,
        first.position.y,
        first.position.z
    );
}
