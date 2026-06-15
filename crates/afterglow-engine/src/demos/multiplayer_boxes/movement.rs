use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::*;

use super::protocol::*;

#[derive(Resource, Default)]
pub struct DemoInput(pub Vec2);

pub fn collect_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut input: ResMut<DemoInput>,
) {
    let mut dir = Vec2::ZERO;
    if keyboard.pressed(KeyCode::KeyW) || keyboard.pressed(KeyCode::ArrowUp) {
        dir.y -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyS) || keyboard.pressed(KeyCode::ArrowDown) {
        dir.y += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyA) || keyboard.pressed(KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyD) || keyboard.pressed(KeyCode::ArrowRight) {
        dir.x += 1.0;
    }
    input.0 = dir.clamp_length_max(1.0);
}

pub fn apply_movement(
    input: Res<DemoInput>,
    mut players: Query<&mut LinearVelocity, (With<PlayerBox>, Without<Predicted>)>,
) {
    let vel = Vec3::new(input.0.x, 0.0, input.0.y) * PLAYER_SPEED;
    for mut velocity in &mut players {
        velocity.0 = vel;
    }
}
