use bevy::prelude::*;

use super::protocol::*;
use crate::network::connection::LocalPlayerId;
use lightyear::prelude::Predicted;

const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 8.0, -6.0);
const LOOK_AHEAD: f32 = 2.0;
const FOLLOW_LAMBDA: f32 = 12.0;

#[derive(Component)]
pub struct DemoCamera;

pub fn setup_camera(
    mut commands: Commands,
    local_player_id: Option<Res<LocalPlayerId>>,
    players: Query<(&PlayerBox, &Transform), With<Predicted>>,
    existing: Query<&DemoCamera>,
) {
    if !existing.is_empty() {
        return;
    }
    let local_str = local_player_id.as_deref().map(|id| id.0.to_string());
    let pos = players
        .iter()
        .find(|(box_, _)| local_str.as_deref() == Some(box_.owner.as_str()))
        .map(|(_, transform)| transform.translation)
        .unwrap_or(Vec3::ZERO);
    commands.spawn((
        DemoCamera,
        Camera3d::default(),
        Msaa::Off,
        Transform::from_translation(pos + CAMERA_OFFSET).looking_at(pos, Vec3::Y),
    ));
}

/// Runs in `PostUpdate` after Lightyear's `VisualCorrection` and
/// `FrameInterpolation` so the camera reads the fully smoothed body Transform.
pub fn follow_camera_system(
    time: Res<Time>,
    local_player_id: Option<Res<LocalPlayerId>>,
    mut cameras: Query<&mut Transform, With<DemoCamera>>,
    players: Query<(&PlayerBox, &Transform), (With<Predicted>, Without<DemoCamera>)>,
) {
    let Ok(mut cam_tf) = cameras.single_mut() else {
        return;
    };
    let local_str = local_player_id.as_deref().map(|id| id.0.to_string());
    let Some((_, transform)) = players
        .iter()
        .find(|(box_, _)| local_str.as_deref() == Some(box_.owner.as_str()))
    else {
        return;
    };
    let target_pos = transform.translation;
    let desired = target_pos + CAMERA_OFFSET;
    let look_at = target_pos + Vec3::new(0.0, 1.0, LOOK_AHEAD);
    let alpha = 1.0 - (-FOLLOW_LAMBDA * time.delta_secs()).exp();
    cam_tf.translation = cam_tf.translation.lerp(desired, alpha);
    cam_tf.look_at(look_at, Vec3::Y);
}
