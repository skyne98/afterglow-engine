//! Rope/grab mechanic: release F to attach a rope entity to the nearest box,
//! release F again to request release. Also highlights the nearest free box.
//!
//! Architecture:
//! - `RopeLink { rope_id, player_owner, target }` is an entity-backed rope
//!   state.
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

pub use super::rope_visual::{
    Highlighted, draw_ropes, highlight_nearest_box, update_highlight_colors,
};
use super::{network::RopeIntentChannel, protocol::*, scene::PlayerName};
use crate::{
    core::identity::{StableEntityId, StableIdAllocator},
    input::AfterglowAction,
    network::{AfterglowNetworkContext, SessionLightyearLinks},
};

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
pub(crate) const HIGHLIGHT_SWITCH_MARGIN: f32 = 0.35;

#[derive(Clone, Copy)]
enum RopeSpawnMode {
    Authoritative,
    ClientPredicted { client_link: Entity },
}

/// Deterministic hash used by Lightyear `PreSpawned` matching.
pub fn rope_link_hash(rope_id: StableEntityId) -> u64 {
    rope_id.as_hash64()
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
    boxes: Query<(&KinematicBox, &StableEntityId, &Transform)>,
    links: Query<(Entity, &RopeLink)>,
    stable_ids: Query<&StableEntityId>,
    mut allocator: ResMut<StableIdAllocator>,
    mut intent_senders: Query<&mut MessageSender<RopeIntent>>,
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

    apply_local_rope_toggle(
        commands,
        owner,
        player_transform.translation,
        &boxes,
        &links,
        &stable_ids,
        &mut allocator,
        mode,
        client_only.then_some(&mut intent_senders),
    );
}

/// Server-authoritative path for client-selected rope intents. The input edge
/// still comes from `ActionState` on the client; this message carries the
/// selected world target separately so ActionState remains pure input.
pub fn server_apply_rope_intents(
    mut commands: Commands,
    context: Option<Res<AfterglowNetworkContext>>,
    mut receivers: Query<(&RemoteId, &mut MessageReceiver<RopeIntent>)>,
    players: Query<(&PlayerBox, &Transform)>,
    boxes: Query<(&KinematicBox, &StableEntityId, &Transform)>,
    links: Query<(Entity, &RopeLink)>,
    stable_ids: Query<&StableEntityId>,
    mut allocator: ResMut<StableIdAllocator>,
) {
    let status = context.as_deref().map(|ctx| ctx.get_connection_status());
    if !status.is_some_and(|status| status.runs_authority()) {
        return;
    }

    for (remote_id, mut receiver) in receivers.iter_mut() {
        let Some(owner) = owner_from_remote_id(remote_id) else {
            continue;
        };
        for intent in receiver.receive() {
            apply_authoritative_rope_intent(
                commands.reborrow(),
                owner.clone(),
                intent,
                &players,
                &boxes,
                &links,
                &stable_ids,
                &mut allocator,
            );
        }
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

fn apply_local_rope_toggle(
    mut commands: Commands,
    owner: String,
    player_pos: Vec3,
    boxes: &Query<(&KinematicBox, &StableEntityId, &Transform)>,
    links: &Query<(Entity, &RopeLink)>,
    stable_ids: &Query<&StableEntityId>,
    allocator: &mut StableIdAllocator,
    mode: RopeSpawnMode,
    intent_senders: Option<&mut Query<&mut MessageSender<RopeIntent>>>,
) {
    if let Some((entity, link)) = links.iter().find(|(_, link)| link.player_owner == owner) {
        match mode {
            RopeSpawnMode::Authoritative => commands.entity(entity).despawn(),
            RopeSpawnMode::ClientPredicted { .. } => {
                if let Some(senders) = intent_senders {
                    let sent = send_rope_intent(
                        senders,
                        RopeIntent {
                            op: RopeIntentOp::Detach,
                            rope_id: link.rope_id,
                            target: None,
                        },
                    );
                    if sent {
                        // Predict detach locally too. Otherwise the client's own
                        // pre-spawned/confirmed RopeLink keeps blocking retries
                        // until the authoritative despawn comes back.
                        commands.entity(entity).try_despawn();
                    }
                }
            }
        }
        return;
    }

    let nearest = boxes
        .iter()
        .filter(|(_, stable_id, _)| !box_has_link(**stable_id, links))
        .min_by(|(_, _, a), (_, _, b)| {
            a.translation
                .distance(player_pos)
                .partial_cmp(&b.translation.distance(player_pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    if let Some((_, target, transform)) = nearest {
        if transform.translation.distance(player_pos) <= ROPE_GRAB_RANGE {
            let rope_id = allocate_rope_id(allocator, stable_ids);
            if matches!(mode, RopeSpawnMode::ClientPredicted { .. }) {
                let Some(senders) = intent_senders else {
                    return;
                };
                let sent = send_rope_intent(
                    senders,
                    RopeIntent {
                        op: RopeIntentOp::Attach,
                        rope_id,
                        target: Some(*target),
                    },
                );
                if !sent {
                    return;
                }
            }
            spawn_rope_link(commands, rope_id, owner, *target, mode);
        }
    }
}

fn send_rope_intent(
    senders: &mut Query<&mut MessageSender<RopeIntent>>,
    intent: RopeIntent,
) -> bool {
    let mut sent = false;
    for mut sender in senders.iter_mut() {
        sender.send::<RopeIntentChannel>(intent.clone());
        sent = true;
    }
    sent
}

pub(crate) fn apply_authoritative_rope_intent(
    mut commands: Commands,
    owner: String,
    intent: RopeIntent,
    players: &Query<(&PlayerBox, &Transform)>,
    boxes: &Query<(&KinematicBox, &StableEntityId, &Transform)>,
    links: &Query<(Entity, &RopeLink)>,
    stable_ids: &Query<&StableEntityId>,
    allocator: &mut StableIdAllocator,
) {
    match intent.op {
        RopeIntentOp::Detach => {
            if let Some((entity, _)) = links
                .iter()
                .find(|(_, link)| link.player_owner == owner && link.rope_id == intent.rope_id)
            {
                commands.entity(entity).despawn();
            }
        }
        RopeIntentOp::Attach => {
            if !intent.rope_id.is_valid()
                || stable_ids.iter().any(|id| *id == intent.rope_id)
                || links.iter().any(|(_, link)| link.player_owner == owner)
            {
                return;
            }
            let Some(target) = intent.target else {
                return;
            };
            if box_has_link(target, links) {
                return;
            }
            let Some((_, player_transform)) = players.iter().find(|(pb, _)| pb.owner == owner)
            else {
                return;
            };
            let Some((_, _, box_transform)) = boxes.iter().find(|(_, id, _)| **id == target) else {
                return;
            };
            if box_transform
                .translation
                .distance(player_transform.translation)
                <= ROPE_GRAB_RANGE
            {
                allocator.reserve_at_least(intent.rope_id.as_raw().saturating_add(1));
                spawn_rope_link(
                    commands,
                    intent.rope_id,
                    owner,
                    target,
                    RopeSpawnMode::Authoritative,
                );
            }
        }
    }
}

fn owner_from_remote_id(remote_id: &RemoteId) -> Option<String> {
    match remote_id.0 {
        PeerId::Netcode(id) => Some(id.to_string()),
        PeerId::Local(id) => Some(id.to_string()),
        _ => None,
    }
}

fn spawn_rope_link(
    mut commands: Commands,
    rope_id: StableEntityId,
    player_owner: String,
    target: StableEntityId,
    mode: RopeSpawnMode,
) {
    let hash = rope_link_hash(rope_id);
    let link = RopeLink {
        rope_id,
        player_owner,
        target,
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

fn allocate_rope_id(
    allocator: &mut StableIdAllocator,
    stable_ids: &Query<&StableEntityId>,
) -> StableEntityId {
    let reserved = stable_ids.iter().copied().collect();
    allocator.allocate_excluding(&reserved)
}

fn box_has_link(target: StableEntityId, links: &Query<(Entity, &RopeLink)>) -> bool {
    links.iter().any(|(_, link)| link.target == target)
}

/// Observer: when `RopeLink` is added, find the owning player and target box,
/// then spawn a local `DistanceJoint` between them.
pub fn on_rope_link_added(
    trigger: On<Add, RopeLink>,
    links: Query<&RopeLink>,
    players: Query<(Entity, &PlayerBox), With<RigidBody>>,
    boxes: Query<(Entity, &StableEntityId), (With<KinematicBox>, With<RigidBody>)>,
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
    let Some((box_entity, _)) = boxes.iter().find(|(_, id)| **id == link.target) else {
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
        // The same predicted/confirmed RopeLink can be removed through multiple
        // Lightyear correction/expiration paths. The derived joint may already
        // be gone by the time this deferred command applies, so use the silent
        // variant to avoid noisy invalid-entity warnings.
        commands.entity(rope_joint.0).try_despawn();
        // Do not remove RopeJointEntity from trigger.entity here. This observer
        // commonly runs while Lightyear is despawning an expired/unmatched
        // PreSpawned RopeLink, so the link entity may already be invalid by the
        // time queued commands apply.
    }
}
