use avian3d::prelude::*;
use bevy::prelude::*;

use super::protocol::*;
use super::scene::PlayerName;

const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 8.0, -6.0);
const LOOK_AHEAD: f32 = 2.0;
const FOLLOW_DAMPING: f32 = 10.0;

#[derive(Component)]
pub struct DemoCamera;

pub fn setup_camera(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    players: Query<(&PlayerBox, &Position)>,
    existing: Query<&DemoCamera>,
) {
    if !existing.is_empty() {
        return;
    }
    let pos = players
        .iter()
        .find(|(box_, _)| box_.owner == player_name.0)
        .map(|(_, p)| p.0)
        .unwrap_or(Vec3::ZERO);
    commands.spawn((
        DemoCamera,
        Camera3d::default(),
        Msaa::Off,
        Transform::from_translation(pos + CAMERA_OFFSET).looking_at(pos, Vec3::Y),
    ));
}

pub fn follow_camera_system(
    time: Res<Time>,
    player_name: Res<PlayerName>,
    mut cameras: Query<&mut Transform, With<DemoCamera>>,
    players: Query<(&PlayerBox, &Position)>,
) {
    let Ok(mut cam_tf) = cameras.single_mut() else {
        return;
    };
    let Some((_, pos)) = players.iter().find(|(box_, _)| box_.owner == player_name.0) else {
        return;
    };
    let target_pos = pos.0;
    let desired = target_pos + CAMERA_OFFSET;
    let look_at = target_pos + Vec3::new(0.0, 1.0, LOOK_AHEAD);
    let t = (FOLLOW_DAMPING * time.delta_secs()).min(1.0);
    cam_tf.translation = cam_tf.translation.lerp(desired, t);
    cam_tf.look_at(look_at, Vec3::Y);
}
