//! Rope/grab mechanic: press F to attach a distance joint to the nearest box,
//! press F again to release.
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
//!   and created locally. This avoids entity-mapping issues across the network.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;

use super::{protocol::*, scene::PlayerName};
use crate::{input::AfterglowAction, network::AfterglowNetworkContext};

/// Toggle the rope on the nearest box when RopeToggle is pressed.
/// Runs on both client (predicted) and server (authoritative).
pub fn toggle_rope(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    players: Query<(&PlayerBox, &Transform, &ActionState<AfterglowAction>)>,
    boxes: Query<(Entity, &KinematicBox, &Transform), Without<RopedTo>>,
    roped: Query<(Entity, &RopedTo)>,
) {
    let local_member = context
        .as_deref()
        .and_then(|ctx| ctx.get_connection_status().local_member_owner());
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
        // Find the player entity matching the owner
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
