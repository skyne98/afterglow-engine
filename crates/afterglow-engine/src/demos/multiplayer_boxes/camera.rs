use bevy::prelude::*;

use super::{
    protocol::*,
    scene::{LocalPlayerPresentation, PlayerName},
};
use crate::network::session::AfterglowSessionState;

const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 8.0, -6.0);
const LOOK_AHEAD: f32 = 2.0;
const FOLLOW_DAMPING: f32 = 10.0;

#[derive(Component)]
pub struct DemoCamera;

pub fn setup_camera(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    session_state: Option<Res<AfterglowSessionState>>,
    players: Query<(&PlayerBox, &Transform, Option<&LocalPlayerPresentation>)>,
    existing: Query<&DemoCamera>,
) {
    if !existing.is_empty() {
        return;
    }
    let member_owner = local_member_owner(session_state.as_deref());
    let pos = players
        .iter()
        .find(|(box_, _, _)| is_local_box(box_, &player_name, member_owner.as_deref()))
        .map(|(_, transform, presentation)| player_presentation_position(transform, presentation))
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
    session_state: Option<Res<AfterglowSessionState>>,
    mut cameras: Query<&mut Transform, With<DemoCamera>>,
    players: Query<(&PlayerBox, &Transform, Option<&LocalPlayerPresentation>), Without<DemoCamera>>,
) {
    let Ok(mut cam_tf) = cameras.single_mut() else {
        return;
    };
    let member_owner = local_member_owner(session_state.as_deref());
    let Some((_, transform, presentation)) = players
        .iter()
        .find(|(box_, _, _)| is_local_box(box_, &player_name, member_owner.as_deref()))
    else {
        return;
    };
    let target_pos = player_presentation_position(transform, presentation);
    let desired = target_pos + CAMERA_OFFSET;
    let look_at = target_pos + Vec3::new(0.0, 1.0, LOOK_AHEAD);
    let t = (FOLLOW_DAMPING * time.delta_secs()).min(1.0);
    cam_tf.translation = cam_tf.translation.lerp(desired, t);
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

fn player_presentation_position(
    transform: &Transform,
    presentation: Option<&LocalPlayerPresentation>,
) -> Vec3 {
    presentation.map_or(transform.translation, |presentation| {
        presentation.visual_translation
    })
}
