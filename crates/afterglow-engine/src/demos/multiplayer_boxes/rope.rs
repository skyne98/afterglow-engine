//! Rope/grab mechanic: press F to attach a distance joint to the nearest box,
//! press F again to release. Also highlights the nearest box.
//!
//! Architecture:
//! - `RopedTo { player_owner }` is a replicated, predicted component on the box
//!   entity. The client predicts adding/removing it; the server validates and
//!   replicates back.
//! - The `DistanceJoint` is a **derived local entity** — created by an observer
//!   when `RopedTo` is added, despawned when `RopedTo` is removed.
//! - `highlight_nearest_box` is a local-only visual system — no replication.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::Predicted;

use super::{protocol::*, scene::PlayerName};
use crate::{input::AfterglowAction, network::AfterglowNetworkContext};

/// Component marking a box as currently highlighted (nearest to local player).
#[derive(Component)]
pub struct Highlighted;

/// Tracks the locally-spawned joint entity so it can be despawned when
/// `RopedTo` is removed.
#[derive(Component)]
pub struct RopeJointEntity(pub Entity);

/// Release-edge memory for rope toggles. This still uses `ActionState`; it
/// reconstructs the release edge from the stable pressed state instead of
/// relying on Leafwing's frame-local `just_released` flag, which can be
/// observed more than once by remote/server-side gameplay.
#[derive(Component, Default)]
pub struct RopeToggleLatch {
    was_pressed: bool,
    pressed_frames: u8,
    cooldown_frames: u8,
}

const ROPE_TOGGLE_COOLDOWN_FRAMES: u8 = 12;
const ROPE_TOGGLE_MIN_PRESSED_FRAMES: u8 = 2;
const HIGHLIGHT_SWITCH_MARGIN: f32 = 0.35;

/// Toggle the rope on the nearest box when RopeToggle is released.
pub fn toggle_rope(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    players: Query<(
        Entity,
        &PlayerBox,
        &Transform,
        &ActionState<AfterglowAction>,
        Has<Predicted>,
        Option<&RopeToggleLatch>,
    )>,
    boxes: Query<(Entity, &KinematicBox, &Transform), Without<RopedTo>>,
    roped: Query<(Entity, &RopedTo)>,
) {
    let status = context.as_deref().map(|ctx| ctx.get_connection_status());
    let client_only = status.is_some_and(|status| status.is_client_only());
    if client_only {
        // Do not locally mutate replicated rope state on clients. Client-side
        // prediction of a binary toggle can race with server correction and
        // manifest as attach-then-immediate-detach. Clients still send the
        // RopeToggle ActionState; the authoritative side writes RopedTo.
        return;
    }
    let authority = status.is_some_and(|status| status.runs_authority());
    let local_member = status.and_then(|s| s.local_member_owner());

    let Some((entity, player_box, player_transform, action, _predicted, previous)) =
        players.iter().find(|(_, pb, _, _, predicted, _)| {
            let is_local =
                pb.owner == player_name.0 || local_member.as_deref() == Some(pb.owner.as_str());
            if !is_local {
                return false;
            }
            (!client_only || *predicted) && (!authority || !*predicted)
        })
    else {
        return;
    };

    let pressed = action.pressed(&AfterglowAction::RopeToggle);
    let (released, next_latch) = next_rope_latch(previous, pressed);
    commands.entity(entity).insert(next_latch);
    if !released {
        return;
    }

    // Use the PlayerBox.owner from the found entity — this is the canonical
    // identifier that on_roped_to_added, draw_ropes, and the other side's
    // PlayerBox.owner will all match against.
    let owner = player_box.owner.clone();

    apply_rope_toggle(
        commands,
        owner,
        player_transform.translation,
        &boxes,
        &roped,
    );
}

/// Server-authoritative path for remote-client rope toggles. Clients may
/// predict `RopedTo`, but the server must also apply the same toggle from the
/// replicated `ActionState`; otherwise Lightyear correction removes the
/// client's predicted rope immediately.
pub fn server_toggle_remote_ropes_from_inputs(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    players: Query<(
        Entity,
        &PlayerBox,
        &Transform,
        &ActionState<AfterglowAction>,
        Has<Predicted>,
        Option<&RopeToggleLatch>,
    )>,
    boxes: Query<(Entity, &KinematicBox, &Transform), Without<RopedTo>>,
    roped: Query<(Entity, &RopedTo)>,
) {
    let status = context.as_deref().map(|ctx| ctx.get_connection_status());
    if !status.is_some_and(|status| status.runs_authority()) {
        return;
    }
    let local_member = status.and_then(|status| status.local_member_owner());

    for (entity, player_box, transform, action, predicted, previous) in players.iter() {
        let is_local = player_box.owner == player_name.0
            || local_member.as_deref() == Some(player_box.owner.as_str());
        if is_local || predicted {
            continue;
        }

        let pressed = action.pressed(&AfterglowAction::RopeToggle);
        let (released, next_latch) = next_rope_latch(previous, pressed);
        if released {
            apply_rope_toggle(
                commands.reborrow(),
                player_box.owner.clone(),
                transform.translation,
                &boxes,
                &roped,
            );
        }
        commands.entity(entity).insert(next_latch);
    }
}

fn next_rope_latch(previous: Option<&RopeToggleLatch>, pressed: bool) -> (bool, RopeToggleLatch) {
    let was_pressed = previous.is_some_and(|previous| previous.was_pressed);
    let pressed_frames = previous.map_or(0, |previous| previous.pressed_frames);
    let cooldown = previous.map_or(0, |previous| previous.cooldown_frames);
    let armed_press = pressed_frames >= ROPE_TOGGLE_MIN_PRESSED_FRAMES;
    let released = was_pressed && !pressed && armed_press && cooldown == 0;
    let next_cooldown = if released {
        ROPE_TOGGLE_COOLDOWN_FRAMES
    } else {
        cooldown.saturating_sub(1)
    };
    let next_pressed_frames = if pressed {
        pressed_frames.saturating_add(1)
    } else {
        0
    };
    (
        released,
        RopeToggleLatch {
            was_pressed: pressed,
            pressed_frames: next_pressed_frames,
            cooldown_frames: next_cooldown,
        },
    )
}

fn apply_rope_toggle(
    mut commands: Commands,
    owner: String,
    player_pos: Vec3,
    boxes: &Query<(Entity, &KinematicBox, &Transform), Without<RopedTo>>,
    roped: &Query<(Entity, &RopedTo)>,
) {
    // If we already have a box roped, release it.
    if let Some((entity, _)) = roped.iter().find(|(_, r)| r.player_owner == owner) {
        commands.entity(entity).remove::<RopedTo>();
        return;
    }

    // Find the nearest box within grab range.
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

/// Observer: when `RopedTo` is added to a box, find the owning player and
/// spawn a `DistanceJoint` between them. Both bodies must have `RigidBody`.
pub fn on_roped_to_added(
    trigger: On<Add, RopedTo>,
    roped: Query<&RopedTo>,
    players: Query<(Entity, &PlayerBox), With<RigidBody>>,
    boxes: Query<(), (With<RigidBody>, Without<RopeJointEntity>)>,
    mut commands: Commands,
) {
    let Ok(roped_to) = roped.get(trigger.entity) else {
        return;
    };
    if boxes.get(trigger.entity).is_err() {
        return;
    }
    let Some((player_entity, _)) = players
        .iter()
        .find(|(_, pb)| pb.owner == roped_to.player_owner)
    else {
        return;
    };

    let joint = DistanceJoint::new(player_entity, trigger.entity)
        .with_limits(0.0, ROPE_MAX_DISTANCE)
        .with_compliance(ROPE_COMPLIANCE);
    let joint_entity = commands.spawn((RopeJoint, joint)).id();
    commands
        .entity(trigger.entity)
        .insert(RopeJointEntity(joint_entity));
}

/// Observer: when `RopedTo` is removed from a box, despawn the joint.
pub fn on_roped_to_removed(
    trigger: On<Remove, RopedTo>,
    box_query: Query<&RopeJointEntity>,
    mut commands: Commands,
) {
    if let Ok(rope_joint) = box_query.get(trigger.entity) {
        commands.entity(rope_joint.0).despawn();
        commands.entity(trigger.entity).remove::<RopeJointEntity>();
    }
}

// ---------------------------------------------------------------------------
// Local-only visual system: highlight the nearest box
// ---------------------------------------------------------------------------

/// Highlight the nearest un-roped box to the local player.
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

    let current: Vec<Entity> = highlighted.iter().collect();
    let current_entity = current.first().copied();
    let desired = player_pos.and_then(|player_pos| {
        let nearest = boxes
            .iter()
            .min_by(|(_, _, a), (_, _, b)| {
                a.translation
                    .distance(player_pos)
                    .partial_cmp(&b.translation.distance(player_pos))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .and_then(|(entity, _, transform)| {
                let distance = transform.translation.distance(player_pos);
                (distance <= ROPE_GRAB_RANGE).then_some((entity, distance))
            });

        let current_distance = current_entity.and_then(|entity| {
            boxes
                .get(entity)
                .ok()
                .map(|(_, _, transform)| transform.translation.distance(player_pos))
                .filter(|distance| *distance <= ROPE_GRAB_RANGE)
        });

        match (current_entity, current_distance, nearest) {
            (Some(current), Some(current_distance), Some((nearest, nearest_distance)))
                if current != nearest
                    && nearest_distance + HIGHLIGHT_SWITCH_MARGIN < current_distance =>
            {
                Some(nearest)
            }
            (Some(current), Some(_), _) => Some(current),
            (_, _, Some((nearest, _))) => Some(nearest),
            _ => None,
        }
    });

    let current: Vec<Entity> = highlighted.iter().collect();
    for entity in current
        .iter()
        .copied()
        .filter(|entity| Some(*entity) != desired)
    {
        commands.entity(entity).remove::<Highlighted>();
    }
    if let Some(entity) = desired {
        if !current.contains(&entity) {
            commands.entity(entity).insert(Highlighted);
        }
    }
}

/// Updates the material color of highlighted boxes. Local-only.
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
    for (mat_handle, box_mat) in boxes.iter() {
        if let Some(mat) = materials.get_mut(mat_handle) {
            mat.base_color = Color::hsla(box_mat.base_hue, 0.7, 0.5, 1.0);
            mat.emissive = LinearRgba::BLACK;
        }
    }

    for (mat_handle, box_mat) in highlighted.iter() {
        if let Some(mat) = materials.get_mut(mat_handle) {
            mat.base_color = Color::hsla(box_mat.base_hue, 0.9, 0.7, 1.0);
            let glow = Color::hsla(box_mat.base_hue, 0.9, 0.5, 1.0).to_srgba();
            mat.emissive = LinearRgba::new(glow.red * 0.3, glow.green * 0.3, glow.blue * 0.3, 1.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Local-only visual system: draw the rope
// ---------------------------------------------------------------------------

/// Draws a line from each roped box to its owning player. Local-only.
pub fn draw_ropes(
    mut gizmos: Gizmos,
    players: Query<(&PlayerBox, &Transform)>,
    roped: Query<(&RopedTo, &Transform)>,
) {
    for (roped_to, box_transform) in roped.iter() {
        let Some((_, player_transform)) = players
            .iter()
            .find(|(pb, _)| pb.owner == roped_to.player_owner)
        else {
            continue;
        };
        gizmos.line(
            player_transform.translation,
            box_transform.translation,
            Color::srgb(0.8, 0.6, 0.2),
        );
    }
}
