use super::{blocker_test_support::*, *};

#[test]
fn controller_does_not_jitter_against_demo_accent_box_centered() {
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
            Transform::from_xyz(box_translation.x, half_height, box_front_z + 2.0),
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
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "demo accent box centered triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "demo accent box centered caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "demo accent box centered caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_demo_accent_box() {
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
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "demo accent box triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "demo accent box caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "demo accent box caused forward/back jitter: {samples:?}"
    );
}

#[test]
fn controller_does_not_jitter_against_demo_barrier() {
    let mut app = app();
    let config = FirstPersonControllerConfig::default();
    let half_height = config.height(ControllerStance::Standing) * 0.5;
    let box_size = Vec3::new(2.2, 0.45, 0.55);
    let box_translation = Vec3::new(-4.0, 0.225, -0.5);
    let box_front_z = box_translation.z + box_size.z * 0.5;
    let player = app
        .world_mut()
        .spawn((
            FirstPersonController {
                config: config.clone(),
            },
            Transform::from_xyz(box_translation.x, half_height, box_front_z + 2.0),
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
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        test_input::set_input(&mut app, player, move_forward_command());
        app.update();
        samples.push(sample(&app, player));
    }

    let settled = &samples[120..];
    assert!(
        settled.iter().all(|sample| !sample.climbing),
        "demo barrier triggered stair climbing: {samples:?}"
    );
    assert!(
        range(settled, |position| position.x) < 0.003,
        "demo barrier caused left/right jitter: {samples:?}"
    );
    assert!(
        range(settled, |position| position.z) < 0.01,
        "demo barrier caused forward/back jitter: {samples:?}"
    );
}
