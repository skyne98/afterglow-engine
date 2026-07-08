//! Rope/grab mechanic driven only by Lightyear/Leafwing input.

use std::collections::HashSet;

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::*;

use super::protocol::*;
pub use super::rope_visual::{
    Highlighted, draw_ropes, highlight_nearest_box, update_highlight_colors,
};
use crate::{
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::connection::{ClientSpawned, LocalPlayerId},
};

/// Tracks the locally-spawned joint entity so it can be despawned when the
/// owning `RopeLink` entity is removed or prediction-disables.
#[derive(Component)]
pub struct RopeJointEntity(pub Entity);

/// Marks a rope link that has queued authoritative/predicted despawn this tick.
/// `sync_rope_joints` must not recreate a derived joint for it before the
/// deferred despawn command is applied.
#[derive(Component)]
pub struct RopeJointDetachPending;

/// Client-local prediction state for rope releases that have already been
/// requested locally but may not have been observed through authoritative
/// despawn replication yet.
///
/// Lightyear's predicted despawn path hides the local entity with
/// `PredictionDisable`, but rollback or delayed confirmed snapshots can
/// temporarily restore the active server rope before the authoritative despawn
/// arrives. Suppressing the deterministic rope id keeps visuals and joints
/// hidden until the player explicitly attaches that same rope again.
#[derive(Resource, Default, Debug)]
pub struct LocallyReleasedRopes {
    rope_ids: HashSet<StableEntityId>,
}

impl LocallyReleasedRopes {
    pub(crate) fn suppress(&mut self, rope_id: StableEntityId) {
        self.rope_ids.insert(rope_id);
    }

    pub(crate) fn allow_again(&mut self, rope_id: StableEntityId) {
        self.rope_ids.remove(&rope_id);
    }

    pub(crate) fn contains(&self, rope_id: StableEntityId) -> bool {
        self.rope_ids.contains(&rope_id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rope_ids.is_empty()
    }
}

pub(crate) const HIGHLIGHT_SWITCH_MARGIN: f32 = 0.35;
const ROPE_ID_NAMESPACE: u128 = 0xA6_u128 << 120;
const AUTHORITATIVE_GRAB_RANGE_SLACK: f32 = 0.35;

#[derive(Clone, Copy)]
enum RopeSpawnMode {
    Authoritative { server: Option<Entity> },
    ClientPredicted { client_link: Entity },
}

/// Deterministic hash used by Lightyear `PreSpawned` matching.
pub fn rope_link_hash(rope_id: StableEntityId) -> u64 {
    rope_id.as_hash64()
}

/// Deterministic rope id for an input-derived attach.
pub fn rope_id_for_input(owner: &str, target: StableEntityId) -> StableEntityId {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    hash = fnv_mix(hash, target.as_raw() as u64);
    hash = fnv_mix(hash, (target.as_raw() >> 64) as u64);
    for byte in owner.as_bytes() {
        hash = fnv_mix(hash, u64::from(*byte));
    }
    let raw = ROPE_ID_NAMESPACE | u128::from(hash);
    StableEntityId::new(raw.max(1))
}

fn fnv_mix(mut hash: u64, value: u64) -> u64 {
    for byte in value.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

pub(crate) fn rope_grab_distance(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

/// Shared fixed-tick rope simulation. The server runs on non-predicted
/// entities (authoritative); the client runs on predicted copies (local
/// prediction). No explicit role check required — `Has<Predicted>` and the
/// presence of `ClientSpawned` distinguish the two cases.
pub fn toggle_rope(
    mut commands: Commands,
    players: Query<(
        Entity,
        &PlayerBox,
        &Transform,
        &ActionState<AfterglowAction>,
        Has<Predicted>,
    )>,
    boxes: Query<(&KinematicBox, &StableEntityId, &Transform)>,
    links: Query<(Entity, &RopeLink, Has<PredictionDisable>)>,
    joint_entities: Query<&RopeJointEntity>,
    client_entities: Query<Entity, With<ClientSpawned>>,
    server_entities: Query<
        Entity,
        (
            With<lightyear::prelude::server::Server>,
            With<lightyear::prelude::server::Started>,
        ),
    >,
    mut locally_released: Option<ResMut<LocallyReleasedRopes>>,
) {
    let client_link = client_entities.iter().next();
    let server_entity = server_entities.single().ok();

    for (_, player_box, player_transform, action, predicted) in &players {
        if client_link.is_some() && !predicted {
            continue;
        }
        if !action.just_released(&AfterglowAction::RopeToggle) {
            continue;
        }

        let mode = if !predicted {
            // Server-side: authoritative spawn
            RopeSpawnMode::Authoritative {
                server: server_entity,
            }
        } else {
            // Client-side: predicted spawn (only works if we have a client link)
            let Some(client_link) = client_link else {
                continue;
            };
            RopeSpawnMode::ClientPredicted { client_link }
        };

        apply_rope_toggle(
            commands.reborrow(),
            player_box.owner.clone(),
            player_transform.translation,
            &boxes,
            &links,
            &joint_entities,
            locally_released.as_deref_mut(),
            mode,
        );
    }
}

fn apply_rope_toggle(
    mut commands: Commands,
    owner: String,
    player_pos: Vec3,
    boxes: &Query<(&KinematicBox, &StableEntityId, &Transform)>,
    links: &Query<(Entity, &RopeLink, Has<PredictionDisable>)>,
    joint_entities: &Query<&RopeJointEntity>,
    mut locally_released: Option<&mut LocallyReleasedRopes>,
    mode: RopeSpawnMode,
) {
    if let Some((entity, link, _)) = links
        .iter()
        .find(|(_, link, disabled)| !*disabled && link.player_owner == owner)
        .or_else(|| links.iter().find(|(_, link, _)| link.player_owner == owner))
    {
        match mode {
            RopeSpawnMode::Authoritative { .. } => {
                commands.entity(entity).insert(RopeJointDetachPending);
                remove_rope_joint(&mut commands, entity, joint_entities);
                commands.entity(entity).despawn();
            }
            RopeSpawnMode::ClientPredicted { .. } => {
                if let Some(locally_released) = locally_released.as_deref_mut() {
                    locally_released.suppress(link.rope_id);
                }
                commands.entity(entity).insert(RopeJointDetachPending);
                remove_rope_joint(&mut commands, entity, joint_entities);
                commands.entity(entity).prediction_despawn();
            }
        }
        return;
    }

    let nearest = boxes
        .iter()
        .filter(|(_, stable_id, _)| !box_has_active_link(**stable_id, links))
        .min_by(|(_, _, a), (_, _, b)| {
            rope_grab_distance(a.translation, player_pos)
                .partial_cmp(&rope_grab_distance(b.translation, player_pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    if let Some((_, target, transform)) = nearest {
        let grab_range = match mode {
            RopeSpawnMode::Authoritative { .. } => ROPE_GRAB_RANGE + AUTHORITATIVE_GRAB_RANGE_SLACK,
            RopeSpawnMode::ClientPredicted { .. } => ROPE_GRAB_RANGE,
        };
        if rope_grab_distance(transform.translation, player_pos) <= grab_range {
            let rope_id = rope_id_for_input(&owner, *target);
            if rope_entity_exists(rope_id, links) {
                return;
            }
            if let RopeSpawnMode::ClientPredicted { .. } = mode {
                if let Some(locally_released) = locally_released.as_deref_mut() {
                    locally_released.allow_again(rope_id);
                }
            }
            spawn_rope_link(commands, rope_id, owner, *target, mode);
        }
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
        RopeSpawnMode::Authoritative { server } => {
            commands.spawn((
                link,
                rope_id,
                PreSpawned::new(hash),
                replicate_all_clients(server),
                predict_all_clients(server),
            ));
        }
        RopeSpawnMode::ClientPredicted { client_link } => {
            commands.spawn((
                link,
                rope_id,
                PreSpawned::new(hash).for_receiver(client_link),
            ));
        }
    }
}

fn replicate_all_clients(server: Option<Entity>) -> Replicate {
    match server {
        Some(server) => Replicate::new(ReplicationMode::Server(server, NetworkTarget::All)),
        None => Replicate::to_clients(NetworkTarget::All),
    }
}

fn predict_all_clients(server: Option<Entity>) -> PredictionTarget {
    match server {
        Some(server) => PredictionTarget::new(ReplicationMode::Server(server, NetworkTarget::All)),
        None => PredictionTarget::to_clients(NetworkTarget::All),
    }
}

fn box_has_active_link(
    target: StableEntityId,
    links: &Query<(Entity, &RopeLink, Has<PredictionDisable>)>,
) -> bool {
    links
        .iter()
        .any(|(_, link, disabled)| !disabled && link.target == target)
}

fn rope_entity_exists(
    rope_id: StableEntityId,
    links: &Query<(Entity, &RopeLink, Has<PredictionDisable>)>,
) -> bool {
    links.iter().any(|(_, link, _)| link.rope_id == rope_id)
}

fn remove_rope_joint(
    commands: &mut Commands,
    entity: Entity,
    joint_entities: &Query<&RopeJointEntity>,
) {
    if let Ok(rope_joint) = joint_entities.get(entity) {
        commands.entity(rope_joint.0).try_despawn();
        commands.entity(entity).remove::<RopeJointEntity>();
    }
}

/// Re-hide locally released ropes if delayed confirmed snapshots briefly make
/// them active again before the authoritative despawn is observed.
pub fn suppress_locally_released_rope_reappearances(
    mut commands: Commands,
    locally_released: Option<Res<LocallyReleasedRopes>>,
    local_player_id: Option<Res<LocalPlayerId>>,
    links: Query<(Entity, &RopeLink, Has<PredictionDisable>)>,
    joint_entities: Query<&RopeJointEntity>,
) {
    let Some(locally_released) = locally_released else {
        return;
    };
    if locally_released.is_empty() {
        return;
    }
    let Some(local_owner) = local_player_id.as_deref().map(|id| id.0.to_string()) else {
        return;
    };

    for (entity, link, disabled) in &links {
        if link.player_owner != local_owner || !locally_released.contains(link.rope_id) {
            continue;
        }
        commands.entity(entity).insert(RopeJointDetachPending);
        remove_rope_joint(&mut commands, entity, &joint_entities);
        if !disabled {
            commands.entity(entity).insert(PredictionDisable);
        }
    }
}

#[doc(hidden)]
#[derive(Default)]
pub struct LocalRopeReleaseEdge {
    was_pressed: bool,
}

/// Hide an already-active local rope immediately when the physical rope key is
/// released, even if Lightyear input-delay/rollback means the buffered
/// `ActionState::just_released` edge will be replayed later.
///
/// This does not create ropes and does not send a parallel command. The normal
/// Lightyear/Leafwing input buffer still delivers the release to the server;
/// this system is only local prediction/presentation suppression for the rope
/// that is already active on this client.
pub fn hide_local_rope_on_physical_release(
    mut commands: Commands,
    keyboard: Option<Res<ButtonInput<KeyCode>>>,
    local_player_id: Option<Res<LocalPlayerId>>,
    mut locally_released: Option<ResMut<LocallyReleasedRopes>>,
    links: Query<(Entity, Ref<RopeLink>, Has<PredictionDisable>)>,
    joint_entities: Query<&RopeJointEntity>,
    rollback: Query<(), With<Rollback>>,
    mut edge: Local<LocalRopeReleaseEdge>,
) {
    if rollback.iter().next().is_some() {
        return;
    }
    let pressed = keyboard.is_some_and(|keyboard| keyboard.pressed(KeyCode::KeyF));
    let released = edge.was_pressed && !pressed;
    edge.was_pressed = pressed;
    if !released {
        return;
    }
    let Some(local_owner) = local_player_id.as_deref().map(|id| id.0.to_string()) else {
        return;
    };

    for (entity, link, disabled) in &links {
        if disabled || link.is_added() || link.player_owner != local_owner {
            continue;
        }
        if let Some(locally_released) = locally_released.as_deref_mut() {
            locally_released.suppress(link.rope_id);
        }
        commands
            .entity(entity)
            .insert((RopeJointDetachPending, PredictionDisable));
        remove_rope_joint(&mut commands, entity, &joint_entities);
    }
}

/// Keep derived local Avian joints in sync with active rope state.
///
/// Server/host creates authoritative joints. Clients create joints only for the
/// locally owned predicted rope; remote/interpolated ropes are visual-only.
pub fn sync_rope_joints(
    mut commands: Commands,
    links: Query<(
        Entity,
        &RopeLink,
        Option<&RopeJointEntity>,
        Has<PredictionDisable>,
        Has<RopeJointDetachPending>,
    )>,
    players: Query<(Entity, &PlayerBox, Has<Predicted>), With<RigidBody>>,
    boxes: Query<(Entity, &StableEntityId), (With<KinematicBox>, With<RigidBody>)>,
    rope_joint_entities: Query<Entity, With<RopeJoint>>,
    client_entities: Query<Entity, With<ClientSpawned>>,
    local_player_id: Option<Res<LocalPlayerId>>,
) {
    let client_link = client_entities.iter().next();
    let is_client = client_link.is_some();
    let local_owner = local_player_id.as_deref().map(|id| id.0.to_string());
    let mut retained_joint_entities = HashSet::new();

    for (entity, link, joint, disabled, detach_pending) in &links {
        let should_drive = rope_should_drive_physics(
            link,
            is_client,
            local_owner.as_deref(),
            joint,
            disabled || detach_pending,
        );

        if !should_drive {
            if let Some(joint) = joint {
                commands.entity(joint.0).try_despawn();
                commands.entity(entity).remove::<RopeJointEntity>();
            }
            continue;
        }
        let Some((player_entity, _, _)) = players
            .iter()
            .find(|(_, pb, predicted)| pb.owner == link.player_owner && (is_client == *predicted))
        else {
            if let Some(joint) = joint {
                commands.entity(joint.0).try_despawn();
                commands.entity(entity).remove::<RopeJointEntity>();
            }
            continue;
        };
        let Some((box_entity, _)) = boxes.iter().find(|(_, id)| **id == link.target) else {
            if let Some(joint) = joint {
                commands.entity(joint.0).try_despawn();
                commands.entity(entity).remove::<RopeJointEntity>();
            }
            continue;
        };
        if let Some(joint) = joint {
            if rope_joint_entities.get(joint.0).is_ok() {
                retained_joint_entities.insert(joint.0);
                continue;
            }
            commands.entity(entity).remove::<RopeJointEntity>();
        }

        let joint_entity = commands
            .spawn((
                RopeJoint,
                DistanceJoint::new(player_entity, box_entity)
                    .with_limits(0.0, ROPE_MAX_DISTANCE)
                    .with_compliance(ROPE_COMPLIANCE),
            ))
            .id();
        commands
            .entity(entity)
            .insert(RopeJointEntity(joint_entity));
    }

    for joint_entity in &rope_joint_entities {
        if !retained_joint_entities.contains(&joint_entity) {
            commands.entity(joint_entity).try_despawn();
        }
    }
}

fn rope_should_drive_physics(
    link: &RopeLink,
    is_client: bool,
    local_owner: Option<&str>,
    _joint: Option<&RopeJointEntity>,
    disabled: bool,
) -> bool {
    if disabled {
        return false;
    }
    if is_client && local_owner != Some(link.player_owner.as_str()) {
        return false;
    }
    // Server: drive authoritative ropes against non-predicted player bodies.
    // Owning client: drive the locally predicted rope against predicted bodies.
    // Non-owning clients keep replicated RopeLink data visual-only and follow
    // confirmed player/box poses instead of simulating divergent constraints.
    true
}

/// Observer: when `RopeLink` is removed, despawn its derived local joint.
pub fn on_rope_link_removed(
    trigger: On<Remove, RopeLink>,
    link_query: Query<&RopeJointEntity>,
    mut commands: Commands,
) {
    if let Ok(rope_joint) = link_query.get(trigger.entity) {
        commands.entity(rope_joint.0).try_despawn();
    }
}
