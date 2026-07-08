use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::action_state::ActionState;
use lightyear::prelude::{NetworkTarget, PredictionTarget, Replicate, ReplicationMode};
use lightyear_inputs_leafwing::prelude::LeafwingBuffer;

use super::{
    movement::apply_movement,
    protocol::{PLAYER_SIZE, PlayerBox, RopeLink},
    rope::{on_rope_link_removed, sync_rope_joints, toggle_rope},
    scene::{spawn_arena, spawn_lights},
};
use crate::{
    core::identity::AutoStableEntityId,
    input::AfterglowAction,
    network::connection::{ConnectionEvent, ConnectionEventKind, PlayerOwned},
};

/// Server-side plugin for the multiplayer boxes demo.
///
/// Listens for [`ConnectionEvent`] to spawn/despawn player entities, runs
/// physics simulation, and handles rope mechanics.
pub struct MultiplayerBoxesServerPlugin;

impl Plugin for MultiplayerBoxesServerPlugin {
    fn build(&self, app: &mut App) {
        super::network::register_demo_protocol(app);

        app.init_resource::<PendingPlayerConnections>();
        app.add_systems(Startup, (spawn_arena, spawn_lights));
        app.add_observer(queue_player_on_connected);
        app.add_observer(despawn_player_on_disconnected);
        app.add_systems(PostUpdate, spawn_pending_connected_players);
        app.add_systems(
            FixedUpdate,
            (apply_movement, toggle_rope, sync_rope_joints)
                .chain()
                .before(avian3d::schedule::PhysicsSystems::Prepare),
        );
        app.add_observer(on_rope_link_removed);
    }
}

#[derive(Resource, Default)]
pub(super) struct PendingPlayerConnections(Vec<crate::network::PlayerId>);

fn queue_player_on_connected(
    trigger: On<ConnectionEvent>,
    mut pending: ResMut<PendingPlayerConnections>,
) {
    let ConnectionEventKind::Connected = trigger.event().kind else {
        return;
    };
    let player_id = trigger.event().player_id;
    if !pending.0.contains(&player_id) {
        pending.0.push(player_id);
    }
}

fn spawn_pending_connected_players(
    mut commands: Commands,
    mut pending: ResMut<PendingPlayerConnections>,
    server_entities: Query<&PlayerBox>,
    lightyear_servers: Query<
        Entity,
        (
            With<lightyear::prelude::server::Server>,
            With<lightyear::prelude::server::Started>,
        ),
    >,
) {
    let lightyear_server = lightyear_servers.single().ok();
    let queued = pending.0.drain(..).collect::<Vec<_>>();
    let mut spawned_this_frame = 0usize;
    for player_id in queued {
        let owner = player_id.to_string();
        if server_entities.iter().any(|player| player.owner == owner) {
            continue;
        }
        let idx = (server_entities.iter().count() + spawned_this_frame) as f32;
        spawned_this_frame += 1;
        let pos = Vec3::new(5.0 + idx * 2.0, PLAYER_SIZE, 0.0);

        commands.spawn((
            PlayerBox { owner },
            RigidBody::Dynamic,
            Collider::cuboid(PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0, PLAYER_SIZE * 2.0),
            Position::from(pos),
            Rotation::default(),
            LinearVelocity::ZERO,
            Transform::from_translation(pos),
            ActionState::<AfterglowAction>::default(),
            LeafwingBuffer::<AfterglowAction>::default(),
            LockedAxes::ROTATION_LOCKED,
            AutoStableEntityId,
            PlayerOwned::from_player_id(player_id),
            replicate_all_clients(lightyear_server),
            predict_all_clients(lightyear_server),
        ));
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

pub(super) fn despawn_player_on_disconnected(
    trigger: On<ConnectionEvent>,
    mut commands: Commands,
    mut pending: Option<ResMut<PendingPlayerConnections>>,
    players: Query<(Entity, &PlayerBox)>,
    rope_links: Query<(Entity, &RopeLink)>,
) {
    let ConnectionEventKind::Disconnected { .. } = trigger.event().kind else {
        return;
    };
    let player_id = trigger.event().player_id;
    if let Some(pending) = pending.as_mut() {
        pending.0.retain(|queued| *queued != player_id);
    }
    let owner_str = player_id.to_string();
    for (entity, player) in &players {
        if player.owner == owner_str {
            commands.entity(entity).despawn();
        }
    }
    for (entity, link) in &rope_links {
        if link.player_owner == owner_str {
            commands.entity(entity).despawn();
        }
    }
}
