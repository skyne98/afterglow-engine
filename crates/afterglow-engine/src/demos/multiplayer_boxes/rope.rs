//! Rope/grab mechanic: release F to attach a rope entity to the nearest box,
//! release F again to request release. Also highlights the nearest free box.
//!
//! Architecture:
//! - `RopeLink { player_owner, box_id }` is an entity-backed rope state.
//! - Clients may spawn `RopeLink + PreSpawned(hash).for_receiver(client_link)`
//!   for immediate predicted feedback.
//! - The server validates the same `ActionState` release and confirms by
//!   spawning authoritative `RopeLink + PreSpawned(hash) + Replicate`.
//! - `DistanceJoint` is a derived local entity created when `RopeLink` appears
//!   and despawned when the link disappears.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

use super::{protocol::*, scene::PlayerName};
use crate::{
    input::AfterglowAction,
    network::{AfterglowNetworkContext, SessionLightyearLinks},
};

/// Component marking a box as currently highlighted (nearest to local player).
#[derive(Component)]
pub struct Highlighted;

/// Tracks the locally-spawned joint entity so it can be despawned when the
/// owning `RopeLink` entity is removed.
#[derive(Component)]
pub struct RopeJointEntity(pub Entity);

/// Release-edge memory for rope toggles. This still uses `ActionState`; it
/// reconstructs the release edge from stable pressed state instead of relying
/// on Leafwing's frame-local `just_released` flag.
#[derive(Component, Default)]
pub struct RopeToggleLatch {
    was_pressed: bool,
    pressed_frames: u8,
    cooldown_frames: u8,
}

const ROPE_TOGGLE_COOLDOWN_FRAMES: u8 = 12;
const ROPE_TOGGLE_MIN_PRESSED_FRAMES: u8 = 2;
const HIGHLIGHT_SWITCH_MARGIN: f32 = 0.35;

#[derive(Clone, Copy)]
enum RopeSpawnMode {
    Authoritative,
    ClientPredicted { client_link: Entity },
}

/// Deterministic hash used by Lightyear `PreSpawned` matching.
pub fn rope_link_hash(player_owner: &str, box_id: u32) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in player_owner.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= u64::from(box_id);
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    hash
}

/// Local/host toggle path. Clients only pre-spawn a `RopeLink`; authority
/// writes the authoritative replicated link.
pub fn toggle_rope(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    session_links: Option<Res<SessionLightyearLinks>>,
    players: Query<(
        Entity,
        &PlayerBox,
        &Transform,
        &ActionState<AfterglowAction>,
        Has<Predicted>,
        Option<&RopeToggleLatch>,
    )>,
    boxes: Query<(&KinematicBox, &Transform)>,
    links: Query<(Entity, &RopeLink)>,
) {
    let status = context.as_deref().map(|ctx| ctx.get_connection_status());
    let client_only = status.is_some_and(|status| status.is_client_only());
    let authority = status.is_some_and(|status| status.runs_authority());
    let local_member = status.and_then(|s| s.local_member_owner());

    let Some((entity, player_box, player_transform, action, _predicted, previous)) =
        players.iter().find(|(_, pb, _, _, predicted, _)| {
            let is_local =
                pb.owner == player_name.0 || local_member.as_deref() == Some(pb.owner.as_str());
            is_local && (!client_only || *predicted) && (!authority || !*predicted)
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

    let owner = player_box.owner.clone();
    let mode = if authority {
        RopeSpawnMode::Authoritative
    } else if let Some(client_link) = session_links.as_deref().and_then(|links| links.client_link) {
        RopeSpawnMode::ClientPredicted { client_link }
    } else {
        return;
    };

    apply_rope_toggle(
        commands,
        owner,
        player_transform.translation,
        &boxes,
        &links,
        mode,
    );
}

/// Server-authoritative path for remote-client rope toggles.
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
    boxes: Query<(&KinematicBox, &Transform)>,
    links: Query<(Entity, &RopeLink)>,
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
                &links,
                RopeSpawnMode::Authoritative,
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
    boxes: &Query<(&KinematicBox, &Transform)>,
    links: &Query<(Entity, &RopeLink)>,
    mode: RopeSpawnMode,
) {
    if let Some((entity, _)) = links.iter().find(|(_, link)| link.player_owner == owner) {
        if matches!(mode, RopeSpawnMode::Authoritative) {
            commands.entity(entity).despawn();
        }
        return;
    }

    let nearest = boxes
        .iter()
        .filter(|(box_data, _)| !box_has_link(box_data.id, links))
        .min_by(|(_, a), (_, b)| {
            a.translation
                .distance(player_pos)
                .partial_cmp(&b.translation.distance(player_pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    if let Some((box_data, transform)) = nearest {
        if transform.translation.distance(player_pos) <= ROPE_GRAB_RANGE {
            spawn_rope_link(commands, owner, box_data.id, mode);
        }
    }
}

fn spawn_rope_link(mut commands: Commands, player_owner: String, box_id: u32, mode: RopeSpawnMode) {
    let hash = rope_link_hash(&player_owner, box_id);
    let link = RopeLink {
        player_owner,
        box_id,
    };
    match mode {
        RopeSpawnMode::Authoritative => {
            commands.spawn((
                link,
                PreSpawned::new(hash),
                Replicate::to_clients(NetworkTarget::All),
                PredictionTarget::to_clients(NetworkTarget::All),
            ));
        }
        RopeSpawnMode::ClientPredicted { client_link } => {
            commands.spawn((link, PreSpawned::new(hash).for_receiver(client_link)));
        }
    }
}

fn box_has_link(box_id: u32, links: &Query<(Entity, &RopeLink)>) -> bool {
    links.iter().any(|(_, link)| link.box_id == box_id)
}

/// Observer: when `RopeLink` is added, find the owning player and target box,
/// then spawn a local `DistanceJoint` between them.
pub fn on_rope_link_added(
    trigger: On<Add, RopeLink>,
    links: Query<&RopeLink>,
    players: Query<(Entity, &PlayerBox), With<RigidBody>>,
    boxes: Query<(Entity, &KinematicBox), With<RigidBody>>,
    existing: Query<(), With<RopeJointEntity>>,
    mut commands: Commands,
) {
    if existing.get(trigger.entity).is_ok() {
        return;
    }
    let Ok(link) = links.get(trigger.entity) else {
        return;
    };
    let Some((player_entity, _)) = players.iter().find(|(_, pb)| pb.owner == link.player_owner)
    else {
        return;
    };
    let Some((box_entity, _)) = boxes.iter().find(|(_, b)| b.id == link.box_id) else {
        return;
    };

    let joint = DistanceJoint::new(player_entity, box_entity)
        .with_limits(0.0, ROPE_MAX_DISTANCE)
        .with_compliance(ROPE_COMPLIANCE);
    let joint_entity = commands.spawn((RopeJoint, joint)).id();
    commands
        .entity(trigger.entity)
        .insert(RopeJointEntity(joint_entity));
}

/// Observer: when `RopeLink` is removed, despawn its derived local joint.
pub fn on_rope_link_removed(
    trigger: On<Remove, RopeLink>,
    link_query: Query<&RopeJointEntity>,
    mut commands: Commands,
) {
    if let Ok(rope_joint) = link_query.get(trigger.entity) {
        commands.entity(rope_joint.0).despawn();
        // Do not remove RopeJointEntity from trigger.entity here. This observer
        // commonly runs while Lightyear is despawning an expired/unmatched
        // PreSpawned RopeLink, so the link entity may already be invalid by the
        // time queued commands apply.
    }
}

/// Highlight the nearest unlinked box to the local player.
pub fn highlight_nearest_box(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    players: Query<(&PlayerBox, &Transform)>,
    boxes: Query<(Entity, &KinematicBox, &Transform)>,
    links: Query<(Entity, &RopeLink)>,
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
            .filter(|(_, box_data, _)| !box_has_link(box_data.id, &links))
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
                .and_then(|(_, box_data, transform)| {
                    (!box_has_link(box_data.id, &links))
                        .then_some(transform.translation.distance(player_pos))
                })
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

/// Draws a line from each rope link to its owning player and target box.
pub fn draw_ropes(
    mut gizmos: Gizmos,
    players: Query<(&PlayerBox, &Transform)>,
    boxes: Query<(&KinematicBox, &Transform)>,
    links: Query<&RopeLink>,
) {
    for link in links.iter() {
        let Some((_, player_transform)) =
            players.iter().find(|(pb, _)| pb.owner == link.player_owner)
        else {
            continue;
        };
        let Some((_, box_transform)) = boxes.iter().find(|(b, _)| b.id == link.box_id) else {
            continue;
        };
        gizmos.line(
            player_transform.translation,
            box_transform.translation,
            Color::srgb(0.8, 0.6, 0.2),
        );
    }
}
