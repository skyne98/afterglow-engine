use bevy::prelude::*;

use super::{protocol::*, scene::PlayerName};
use crate::network::session::AfterglowSessionState;

const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 8.0, -6.0);
const LOOK_AHEAD: f32 = 2.0;
/// Exponential decay constant for camera follow (higher = snappier).
const FOLLOW_LAMBDA: f32 = 12.0;

#[derive(Component)]
pub struct DemoCamera;

pub fn setup_camera(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    session_state: Option<Res<AfterglowSessionState>>,
    players: Query<(&PlayerBox, &Transform)>,
    existing: Query<&DemoCamera>,
) {
    if !existing.is_empty() {
        return;
    }
    let member_owner = local_member_owner(session_state.as_deref());
    let pos = players
        .iter()
        .find(|(box_, _)| is_local_box(box_, &player_name, member_owner.as_deref()))
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
    player_name: Res<PlayerName>,
    session_state: Option<Res<AfterglowSessionState>>,
    mut cameras: Query<&mut Transform, With<DemoCamera>>,
    players: Query<(&PlayerBox, &Transform), Without<DemoCamera>>,
) {
    let Ok(mut cam_tf) = cameras.single_mut() else {
        return;
    };
    let member_owner = local_member_owner(session_state.as_deref());
    let Some((_, transform)) = players
        .iter()
        .find(|(box_, _)| is_local_box(box_, &player_name, member_owner.as_deref()))
    else {
        return;
    };
    let target_pos = transform.translation;
    let desired = target_pos + CAMERA_OFFSET;
    let look_at = target_pos + Vec3::new(0.0, 1.0, LOOK_AHEAD);
    // Frame-rate-independent exponential decay: same behaviour at 30/60/144fps.
    let alpha = 1.0 - (-FOLLOW_LAMBDA * time.delta_secs()).exp();
    cam_tf.translation = cam_tf.translation.lerp(desired, alpha);
    cam_tf.look_at(look_at, Vec3::Y);
}

fn local_member_owner(session_state: Option<&AfterglowSessionState>) -> Option<String> {
    session_state
        .map(|state| state.local_member_id)
        .filter(|member| member.is_valid())
        .map(|member| member.as_raw().to_string())
}

fn is_local_box(box_: &PlayerBox, player_name: &PlayerName, member_owner: Option<&str>) -> bool {
    box_.owner == player_name.0 || member_owner == Some(box_.owner.as_str())
}
