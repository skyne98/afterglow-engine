//! Lightyear `PreSpawned` regression coverage for predicted physics
//! interactions.
//!
//! This module intentionally models grab as a spawned constraint entity instead
//! of a flag on the grabbed body. Lightyear's `PreSpawned` matching works at
//! entity identity boundaries, so transient interactions that need client-side
//! responsiveness should become predicted entities with deterministic hashes.
//!
//! The scenario is adversarial enough to matter for twitch interactions: Client
//! A predicts against the world it can see, Client B has already changed the
//! server world at the same tick window, and the server is the only source of
//! truth.

use afterglow_engine::{
    core::identity::StableEntityId,
    network::{AfterglowLightyearConfig, LightyearRole},
};
use bevy::{
    app::{FixedPostUpdate, PostUpdate, PreUpdate},
    prelude::*,
};
use lightyear::{
    crossbeam::CrossbeamIo,
    prelude::{
        server::{ClientOf, Started},
        *,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

use crate::Vec3i;

const PLAYER_A: StableEntityId = StableEntityId::from_raw(101);
const PLAYER_B: StableEntityId = StableEntityId::from_raw(102);
const SPHERE: StableEntityId = StableEntityId::from_raw(201);
const BOX: StableEntityId = StableEntityId::from_raw(202);
const GRAB_POINT: Vec3i = Vec3i::new(5, 0, 0);

// A dedicated command channel keeps the test protocol close to production
// shape: input/intent moves client-to-server, while entity state is replicated
// back.
struct PhysicsCommandChannel;

#[derive(Clone, Copy, Component, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct GrabBody {
    shape: GrabShape,
    position: Vec3i,
    grabbable: bool,
}

// The predicted object is the relationship itself. That lets the client spawn
// immediate feedback while the server later decides whether the relationship is
// valid, rejected, or valid but pointing at a different target.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct GrabConstraint {
    player: StableEntityId,
    target: StableEntityId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum GrabShape {
    Sphere,
    Box,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct PhysicsCommand {
    player: StableEntityId,
    tick: u32,
    sequence: u64,
    action: PhysicsAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum PhysicsAction {
    SpawnBox { grabbable: bool },
    GrabAt { target: Vec3i },
}

#[derive(Default)]
struct AuthoritativeGrabWorld {
    bodies: BTreeMap<StableEntityId, GrabBody>,
    constraints: BTreeMap<u64, GrabConstraint>,
}

// Owns three isolated Bevy apps wired together with Lightyear Crossbeam links.
// Keeping server, Client A, and Client B as real apps catches schedule and
// component-registration mistakes that a pure data-structure test would miss.
struct PhysicsGrabNetwork {
    server: AuthoritativeGrabWorld,
    server_app: App,
    client_a: App,
    client_b: App,
    client_a_link: Entity,
    client_b_link: Entity,
    server_a_link: Entity,
    server_b_link: Entity,
    body_entities: BTreeMap<StableEntityId, Entity>,
    constraint_entities: BTreeMap<u64, Entity>,
}

impl PhysicsGrabNetwork {
    fn new() -> Self {
        let mut server_app = physics_app(LightyearRole::Server);
        let mut client_a = physics_app(LightyearRole::Client);
        let mut client_b = physics_app(LightyearRole::Client);
        let (client_a_io, server_a_io) = CrossbeamIo::new_pair();
        let (client_b_io, server_b_io) = CrossbeamIo::new_pair();
        let server_entity = server_app
            .world_mut()
            .spawn((Server::default(), Started))
            .id();
        let client_a_link = spawn_client_link(&mut client_a, PeerId::Local(1), client_a_io);
        let client_b_link = spawn_client_link(&mut client_b, PeerId::Local(2), client_b_io);
        let server_a_link = spawn_server_link(
            &mut server_app,
            server_entity,
            PeerId::Local(1),
            server_a_io,
        );
        let server_b_link = spawn_server_link(
            &mut server_app,
            server_entity,
            PeerId::Local(2),
            server_b_io,
        );

        client_a.update();
        client_b.update();
        server_app.update();
        client_a.update();
        client_b.update();

        let mut server = AuthoritativeGrabWorld::default();
        server.bodies.insert(
            SPHERE,
            GrabBody {
                shape: GrabShape::Sphere,
                position: GRAB_POINT,
                grabbable: true,
            },
        );

        let mut network = Self {
            server,
            server_app,
            client_a,
            client_b,
            client_a_link,
            client_b_link,
            server_a_link,
            server_b_link,
            body_entities: BTreeMap::new(),
            constraint_entities: BTreeMap::new(),
        };
        network.sync_server_to_clients();
        network
    }

    fn client_b_spawns_unseen_box(&mut self, grabbable: bool) {
        // Client B's command is delivered before Client A predicts, but Client A
        // does not receive the resulting replicated box yet. This is the core
        // stale-visibility case for client prediction.
        self.send_client_b(PhysicsCommand {
            player: PLAYER_B,
            tick: 0,
            sequence: 1,
            action: PhysicsAction::SpawnBox { grabbable },
        });
        self.deliver_client_commands();
    }

    fn client_a_predicts_and_sends_grab(&mut self) -> Entity {
        // Client A predicts against only its local replicated view. At this point
        // the sphere is visible and the box is not, so the predicted target is
        // intentionally wrong in the grabbable-box test.
        let predicted = predict_client_grab(
            &mut self.client_a,
            self.client_a_link,
            PLAYER_A,
            1,
            GRAB_POINT,
        );
        assert_eq!(
            client_constraints(&mut self.client_a),
            vec![GrabConstraint {
                player: PLAYER_A,
                target: SPHERE
            }]
        );
        assert!(
            self.client_a
                .world()
                .entity(predicted)
                .contains::<PreSpawned>()
        );
        self.send_client_a(PhysicsCommand {
            player: PLAYER_A,
            tick: 1,
            sequence: 1,
            action: PhysicsAction::GrabAt { target: GRAB_POINT },
        });
        self.deliver_client_commands();
        predicted
    }

    fn assert_client_a_matches_server_constraints(&mut self) {
        self.sync_server_to_clients();
        assert_eq!(
            client_constraints(&mut self.client_a),
            self.server.constraints()
        );
    }

    fn send_client_a(&mut self, command: PhysicsCommand) {
        send_command(&mut self.client_a, self.client_a_link, command);
    }

    fn send_client_b(&mut self, command: PhysicsCommand) {
        send_command(&mut self.client_b, self.client_b_link, command);
    }

    fn deliver_client_commands(&mut self) {
        self.client_a.world_mut().run_schedule(PostUpdate);
        self.client_b.world_mut().run_schedule(PostUpdate);
        self.server_app.world_mut().run_schedule(PreUpdate);
        let mut commands = receive_commands(&mut self.server_app, self.server_a_link);
        commands.extend(receive_commands(&mut self.server_app, self.server_b_link));
        // Server authority needs deterministic ordering when different clients
        // submit commands for the same tick. Stable IDs make the tie-breaker
        // independent of local Bevy entity allocation.
        commands.sort_by_key(|command| (command.tick, command.player, command.sequence));
        for command in commands {
            self.server.apply(command);
        }
    }

    fn sync_server_to_clients(&mut self) {
        sync_server_entities(
            &mut self.server_app,
            &mut self.body_entities,
            &mut self.constraint_entities,
            &self.server,
        );
        for _ in 0..2 {
            // Send replicated server state, receive it on both clients, then let
            // Lightyear prediction/confirmation systems match or update the
            // predicted entity in FixedPostUpdate.
            self.server_app.world_mut().run_schedule(PostUpdate);
            self.client_a.world_mut().run_schedule(PreUpdate);
            self.client_b.world_mut().run_schedule(PreUpdate);
            self.client_a.world_mut().run_schedule(FixedPostUpdate);
            self.client_b.world_mut().run_schedule(FixedPostUpdate);
        }
    }

    fn expire_client_a_prespawns(&mut self) {
        // Unmatched pre-spawned entities are not removed immediately. Lightyear
        // waits for the pre-spawn timeout so late confirmations can still match.
        for _ in 0..70 {
            self.client_a
                .world_mut()
                .resource_mut::<LocalTimeline>()
                .apply_delta(1);
            self.client_a.world_mut().run_schedule(PostUpdate);
        }
    }
}

impl AuthoritativeGrabWorld {
    fn apply(&mut self, command: PhysicsCommand) {
        match command.action {
            PhysicsAction::SpawnBox { grabbable } => {
                self.bodies.insert(
                    BOX,
                    GrabBody {
                        shape: GrabShape::Box,
                        position: GRAB_POINT,
                        grabbable,
                    },
                );
            }
            PhysicsAction::GrabAt { target } => {
                self.resolve_grab(command.player, command.sequence, target)
            }
        }
    }

    fn resolve_grab(&mut self, player: StableEntityId, sequence: u64, target: Vec3i) {
        // Prefer a box at the target to model Client B placing an unseen object
        // in front of the sphere. A grabbable box redirects the grab; a
        // non-grabbable box blocks it outright.
        let box_at_target = self.bodies.iter().find_map(|(stable, body)| {
            (body.shape == GrabShape::Box && body.position == target).then_some((*stable, *body))
        });
        let target = match box_at_target {
            Some((stable, body)) if body.grabbable => Some(stable),
            Some(_) => None,
            None => self.first_grabbable_at(target),
        };
        if let Some(target) = target {
            self.constraints.insert(
                grab_hash(player, sequence),
                GrabConstraint { player, target },
            );
        }
    }

    fn first_grabbable_at(&self, target: Vec3i) -> Option<StableEntityId> {
        self.bodies.iter().find_map(|(stable, body)| {
            (body.position == target && body.grabbable).then_some(*stable)
        })
    }

    fn constraints(&self) -> Vec<GrabConstraint> {
        self.constraints.values().copied().collect()
    }
}

fn physics_app(role: LightyearRole) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(AfterglowLightyearConfig { role, ..default() });
    add_crossbeam_lightyear_plugins(&mut app, role);
    app.init_resource::<PeerMetadata>();
    register_protocol(&mut app);
    app.add_systems(
        PreUpdate,
        reconcile_confirmed_constraints.after(ReplicationSystems::Receive),
    );
    app.finish();
    app.cleanup();
    app
}

fn reconcile_confirmed_constraints(
    mut query: Query<(&mut GrabConstraint, &Confirmed<GrabConstraint>), With<Predicted>>,
) {
    for (mut predicted, confirmed) in &mut query {
        *predicted = confirmed.0;
    }
}

fn add_crossbeam_lightyear_plugins(app: &mut App, role: LightyearRole) {
    let tick_duration = Duration::from_secs_f64(1.0 / 60.0);
    match role {
        LightyearRole::Client => app.add_plugins(
            lightyear::prelude::client::ClientPlugins { tick_duration }
                .build()
                .disable::<lightyear::prelude::client::NetcodeClientPlugin>(),
        ),
        LightyearRole::Server => app.add_plugins(
            lightyear::prelude::server::ServerPlugins { tick_duration }
                .build()
                .disable::<lightyear::prelude::server::NetcodeServerPlugin>(),
        ),
        LightyearRole::Host => unreachable!("physics grab tests use separate clients and server"),
    };
}

fn register_protocol(app: &mut App) {
    app.add_channel::<PhysicsCommandChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: Duration::ZERO,
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<PhysicsCommand>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_component::<StableEntityId>();
    // Bodies and constraints are prediction-enabled because client-visible state
    // is represented as predicted entities after Lightyear receives replication.
    app.register_component::<GrabBody>().add_prediction();
    app.register_component::<GrabConstraint>().add_prediction();
}

fn transport(app: &App, send: bool, receive: bool) -> Transport {
    let registry = app.world().resource::<ChannelRegistry>();
    let mut transport = Transport::default();
    if send {
        transport.add_sender_from_registry::<PhysicsCommandChannel>(registry);
    }
    if receive {
        transport.add_receiver_from_registry::<PhysicsCommandChannel>(registry);
    }
    transport.add_sender_from_registry::<MetadataChannel>(registry);
    transport.add_receiver_from_registry::<MetadataChannel>(registry);
    transport.add_sender_from_registry::<UpdatesChannel>(registry);
    transport.add_receiver_from_registry::<UpdatesChannel>(registry);
    transport
}

fn spawn_client_link(app: &mut App, local_id: PeerId, io: CrossbeamIo) -> Entity {
    let transport = transport(app, true, false);
    app.world_mut()
        .spawn((
            Client::default(),
            LocalId(local_id),
            RemoteId(PeerId::Server),
            Connected,
            Link::default(),
            Linked,
            io,
            transport,
            MessageManager::default(),
            ReplicationReceiver::default(),
            PredictionManager::default(),
            MessageSender::<PhysicsCommand>::default(),
        ))
        .id()
}

fn spawn_server_link(app: &mut App, server: Entity, remote_id: PeerId, io: CrossbeamIo) -> Entity {
    let transport = transport(app, false, true);
    app.world_mut()
        .spawn((
            LinkOf { server },
            ClientOf,
            LocalId(PeerId::Server),
            RemoteId(remote_id),
            Connected,
            Link::default(),
            Linked,
            io,
            transport,
            MessageManager::default(),
            ReplicationSender::new(Duration::ZERO, SendUpdatesMode::SinceLastAck, false),
            MessageReceiver::<PhysicsCommand>::default(),
        ))
        .id()
}

fn send_command(app: &mut App, link: Entity, command: PhysicsCommand) {
    app.world_mut()
        .entity_mut(link)
        .get_mut::<MessageSender<PhysicsCommand>>()
        .unwrap()
        .send::<PhysicsCommandChannel>(command);
}

fn receive_commands(app: &mut App, link: Entity) -> Vec<PhysicsCommand> {
    app.world_mut()
        .entity_mut(link)
        .get_mut::<MessageReceiver<PhysicsCommand>>()
        .unwrap()
        .receive()
        .collect()
}

fn sync_server_entities(
    app: &mut App,
    body_entities: &mut BTreeMap<StableEntityId, Entity>,
    constraint_entities: &mut BTreeMap<u64, Entity>,
    server: &AuthoritativeGrabWorld,
) {
    for (stable, body) in &server.bodies {
        let entity = *body_entities.entry(*stable).or_insert_with(|| {
            app.world_mut()
                .spawn((
                    *stable,
                    *body,
                    Replicate::to_clients(NetworkTarget::All),
                    PredictionTarget::to_clients(NetworkTarget::All),
                ))
                .id()
        });
        app.world_mut().entity_mut(entity).insert(*body);
    }
    constraint_entities.retain(|hash, entity| {
        let keep = server.constraints.contains_key(hash);
        if !keep && let Ok(entity) = app.world_mut().get_entity_mut(*entity) {
            entity.despawn();
        }
        keep
    });
    for (hash, constraint) in &server.constraints {
        let entity = *constraint_entities.entry(*hash).or_insert_with(|| {
            app.world_mut()
                .spawn((
                    *constraint,
                    // This hash must match the client's predicted hash. If it does,
                    // Lightyear keeps the predicted entity and attaches confirmation
                    // state instead of creating a duplicate constraint on the client.
                    PreSpawned::new(*hash),
                    Replicate::to_clients(NetworkTarget::All),
                    PredictionTarget::to_clients(NetworkTarget::All),
                ))
                .id()
        });
        app.world_mut().entity_mut(entity).insert(*constraint);
    }
}

fn predict_client_grab(
    app: &mut App,
    receiver: Entity,
    player: StableEntityId,
    sequence: u64,
    target: Vec3i,
) -> Entity {
    let target = visible_grabbable_at(app, target);
    app.world_mut()
        .spawn((
            GrabConstraint { player, target },
            // `for_receiver` scopes this client-side pre-spawn to the local link. A
            // server entity using the same hash can confirm it for this receiver.
            PreSpawned::new(grab_hash(player, sequence)).for_receiver(receiver),
        ))
        .id()
}

fn visible_grabbable_at(app: &mut App, target: Vec3i) -> StableEntityId {
    let mut query = app
        .world_mut()
        .query::<(&StableEntityId, &GrabBody, Option<&Predicted>)>();
    // The client can only predict from replicated/predicted state it has already
    // received. Authoritative-but-unseen entities intentionally do not appear.
    query
        .iter(app.world())
        .find_map(|(stable, body, predicted)| {
            (predicted.is_some() && body.position == target && body.grabbable).then_some(*stable)
        })
        .expect("client should have visible grabbable target")
}

fn client_constraints(app: &mut App) -> Vec<GrabConstraint> {
    let mut query = app
        .world_mut()
        .query::<(&GrabConstraint, Option<&Predicted>, Option<&PreSpawned>)>();
    let mut values = query
        .iter(app.world())
        .filter_map(|(constraint, predicted, prespawned)| {
            (predicted.is_some() || prespawned.is_some()).then_some(*constraint)
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|constraint| (constraint.player, constraint.target));
    values
}

fn client_bodies(app: &mut App) -> BTreeMap<StableEntityId, GrabBody> {
    let mut query = app
        .world_mut()
        .query::<(&StableEntityId, &GrabBody, Option<&Predicted>)>();
    query
        .iter(app.world())
        .filter_map(|(stable, body, predicted)| predicted.is_some().then_some((*stable, *body)))
        .collect()
}

fn grab_hash(player: StableEntityId, sequence: u64) -> u64 {
    0xA6AB_0000_0000_0000 ^ ((player.as_raw() as u64) << 16) ^ sequence
}

#[test]
fn client_grab_prespawn_matches_server_box_when_unseen_box_is_grabbable() {
    let mut network = PhysicsGrabNetwork::new();
    network.client_b_spawns_unseen_box(true);
    assert!(!client_bodies(&mut network.client_a).contains_key(&BOX));

    let predicted = network.client_a_predicts_and_sends_grab();
    network.assert_client_a_matches_server_constraints();

    assert_eq!(
        client_constraints(&mut network.client_a),
        vec![GrabConstraint {
            player: PLAYER_A,
            target: BOX
        }]
    );
    assert!(
        network.client_a.world().get_entity(predicted).is_ok(),
        "server confirmation should match the prespawned entity"
    );
    assert!(
        !network
            .client_a
            .world()
            .entity(predicted)
            .contains::<PreSpawned>()
    );
}

#[test]
fn client_grab_prespawn_is_cleaned_up_when_unseen_box_blocks() {
    let mut network = PhysicsGrabNetwork::new();
    network.client_b_spawns_unseen_box(false);
    assert!(!client_bodies(&mut network.client_a).contains_key(&BOX));

    let predicted = network.client_a_predicts_and_sends_grab();
    network.sync_server_to_clients();
    assert_eq!(network.server.constraints(), Vec::<GrabConstraint>::new());
    assert!(network.client_a.world().get_entity(predicted).is_ok());

    network.expire_client_a_prespawns();
    assert_eq!(
        client_constraints(&mut network.client_a),
        Vec::<GrabConstraint>::new()
    );
    assert!(
        network.client_a.world().get_entity(predicted).is_err(),
        "unmatched prespawned grab should be removed"
    );
}
