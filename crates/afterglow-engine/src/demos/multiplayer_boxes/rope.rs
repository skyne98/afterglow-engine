//! Rope/grab mechanic: press F to attach a distance joint to the nearest box,
//! press F again to release. Also highlights the nearest box.
//!
//! Architecture:
//! - `RopedTo { player_owner }` is a replicated, predicted component on the box
//!   entity. The client predicts adding/removing it; the server validates and
//!   replicates back.
//! - `sync_rope_joints` runs locally on both client and server. When it sees a
//!   `RopedTo` component, it spawns a local `DistanceJoint` entity connecting
//!   the box to the owning player. When `RopedTo` is removed, the joint is
//!   despawned.
//! - Joints are **not** replicated — they're derived from the `RopedTo` state
//!   and created locally.
//! - `highlight_nearest_box` is a local-only visual system. It does NOT touch
//!   any replicated state — it only changes material color locally.
//! - `toggle_rope` uses `ActionState` for input — NEVER bypasses it.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;

use super::{protocol::*, scene::PlayerName};
use crate::{input::AfterglowAction, network::AfterglowNetworkContext};

/// Component marking a box as currently highlighted (nearest to local player).
#[derive(Component)]
pub struct Highlighted;

/// Toggle the rope on the nearest box when RopeToggle is pressed.
/// Runs in `Update` (not `FixedUpdate`) so `just_pressed` is available
/// before Lightyear's input delay pipeline overwrites `ActionState`.
pub fn toggle_rope(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    players: Query<(&PlayerBox, &Transform, &ActionState<AfterglowAction>)>,
    boxes: Query<(Entity, &KinematicBox, &Transform), Without<RopedTo>>,
    roped: Query<(Entity, &RopedTo)>,
) {
    let status = context.as_deref().map(|ctx| ctx.get_connection_status());
    let local_member = status.and_then(|s| s.local_member_owner());

    let Some((_, player_transform, action)) = players.iter().find(|(pb, _, _)| {
        pb.owner == player_name.0 || local_member.as_deref() == Some(pb.owner.as_str())
    }) else {
        return;
    };

    if !action.just_pressed(&AfterglowAction::RopeToggle) {
        return;
    }

    let owner = local_member
        .map(|s| s.to_string())
        .unwrap_or_else(|| player_name.0.clone());

    // If we already have a box roped, release it
    if let Some((entity, _)) = roped.iter().find(|(_, r)| r.player_owner == owner) {
        commands.entity(entity).remove::<RopedTo>();
        return;
    }

    // Find the nearest box within grab range
    let player_pos = player_transform.translation;
    let nearest = boxes.iter().min_by(|(_, _, a), (_, _, b)| {
        a.translation
            .distance(player_pos)
            .partial_cmp(&b.translation.distance(player_pos))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if let Some((entity, _, transform)) = nearest {
        if transform.translation.distance(player_pos) <= ROPE_GRAB_RANGE {
            commands.entity(entity).insert(RopedTo {
                player_owner: owner,
            });
        }
    }
}

/// Create/destroy distance joints based on the `RopedTo` component.
/// Runs locally on both client and server — joints are not replicated,
/// only the `RopedTo` marker is.
pub fn sync_rope_joints(
    mut commands: Commands,
    players: Query<(Entity, &PlayerBox)>,
    boxes: Query<(Entity, &RopedTo)>,
    existing_joints: Query<(Entity, &DistanceJoint, &RopeJoint)>,
) {
    // Remove joints for boxes that are no longer roped
    let roped_boxes: std::collections::HashSet<Entity> = boxes.iter().map(|(e, _)| e).collect();
    for (joint_entity, joint, _) in existing_joints.iter() {
        if !roped_boxes.contains(&joint.body2) {
            commands.entity(joint_entity).despawn();
        }
    }

    // Create joints for boxes that are roped but don't have a joint yet
    let jointed_boxes: std::collections::HashSet<Entity> =
        existing_joints.iter().map(|(_, j, _)| j.body2).collect();

    for (box_entity, roped) in boxes.iter() {
        if jointed_boxes.contains(&box_entity) {
            continue;
        }
        let Some(player_entity) = players
            .iter()
            .find(|(_, pb)| pb.owner == roped.player_owner)
            .map(|(e, _)| e)
        else {
            continue;
        };

        let joint = DistanceJoint::new(player_entity, box_entity)
            .with_limits(0.0, ROPE_MAX_DISTANCE)
            .with_compliance(ROPE_COMPLIANCE);
        commands.spawn((RopeJoint, joint));
    }
}

// ---------------------------------------------------------------------------
// Local-only visual system: highlight the nearest box
// ---------------------------------------------------------------------------

/// Highlight the nearest un-roped box to the local player.
/// This is a **local-only** system — it does NOT touch any replicated state.
/// It only changes the material color locally. Both client and host run it.
pub fn highlight_nearest_box(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    players: Query<(&PlayerBox, &Transform)>,
    boxes: Query<(Entity, &KinematicBox, &Transform), Without<RopedTo>>,
    highlighted: Query<Entity, With<Highlighted>>,
) {
    let status = context.as_deref().map(|ctx| ctx.get_connection_status());
    let local_member = status.and_then(|s| s.local_member_owner());

    let player_pos = players
        .iter()
        .find(|(pb, _)| {
            pb.owner == player_name.0 || local_member.as_deref() == Some(pb.owner.as_str())
        })
        .map(|(_, t)| t.translation);

    // Clear all existing highlights
    for entity in highlighted.iter() {
        commands.entity(entity).remove::<Highlighted>();
    }

    let Some(player_pos) = player_pos else {
        return;
    };

    // Find nearest un-roped box within grab range
    let nearest = boxes.iter().min_by(|(_, _, a), (_, _, b)| {
        a.translation
            .distance(player_pos)
            .partial_cmp(&b.translation.distance(player_pos))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if let Some((entity, _, transform)) = nearest {
        if transform.translation.distance(player_pos) <= ROPE_GRAB_RANGE {
            commands.entity(entity).insert(Highlighted);
        }
    }
}

/// Updates the material color of highlighted boxes. Local-only — does not
/// touch replicated state.
pub fn update_highlight_colors(
    mut materials: ResMut<Assets<StandardMaterial>>,
    boxes: Query<
        (
            &MeshMaterial3d<StandardMaterial>,
            &super::scene::BoxMaterial,
        ),
        Without<Highlighted>,
    >,
    highlighted: Query<
        (
            &MeshMaterial3d<StandardMaterial>,
            &super::scene::BoxMaterial,
        ),
        With<Highlighted>,
    >,
) {
    // Restore non-highlighted boxes to their base color
    for (mat_handle, box_mat) in boxes.iter() {
        if let Some(mat) = materials.get_mut(mat_handle) {
            mat.base_color = Color::hsla(box_mat.base_hue, 0.7, 0.5, 1.0);
            mat.emissive = LinearRgba::BLACK;
        }
    }

    // Make highlighted boxes glow brighter
    for (mat_handle, box_mat) in highlighted.iter() {
        if let Some(mat) = materials.get_mut(mat_handle) {
            mat.base_color = Color::hsla(box_mat.base_hue, 0.9, 0.7, 1.0);
            let glow = Color::hsla(box_mat.base_hue, 0.9, 0.5, 1.0).to_srgba();
            mat.emissive = LinearRgba::new(glow.red * 0.3, glow.green * 0.3, glow.blue * 0.3, 1.0);
        }
    }
}
