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
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use super::{harness::NetworkedRpg, model::*};
use crate::Vec3i;

struct RpgInputChannel;

pub struct LightyearNetworkedRpg {
    server: NetworkedRpg,
    client_app: App,
    server_app: App,
    client_link: Entity,
    server_link: Entity,
    replicated: BTreeMap<StableEntityId, Entity>,
    pending: Vec<(u32, ClientInput)>,
    received_inputs: usize,
    current_tick: u32,
}

impl LightyearNetworkedRpg {
    pub fn new(retention_ticks: u32) -> Self {
        let mut client_app = lightyear_app(true);
        let mut server_app = lightyear_app(false);
        let (client_io, server_io) = CrossbeamIo::new_pair();
        let client_transport = transport(&client_app, true, false);
        let server_transport = transport(&server_app, false, true);

        let client_link = client_app
            .world_mut()
            .spawn((
                Client::default(),
                LocalId(PeerId::Local(1)),
                RemoteId(PeerId::Server),
                Connected,
                Link::default(),
                Linked,
                client_io,
                client_transport,
                MessageManager::default(),
                ReplicationReceiver::default(),
                PredictionManager::default(),
                MessageSender::<ClientInput>::default(),
            ))
            .id();
        let server_entity = server_app
            .world_mut()
            .spawn((Server::default(), Started))
            .id();
        let server_link = server_app
            .world_mut()
            .spawn((
                LinkOf {
                    server: server_entity,
                },
                ClientOf,
                LocalId(PeerId::Server),
                RemoteId(PeerId::Local(1)),
                Connected,
                Link::default(),
                Linked,
                server_io,
                server_transport,
                MessageManager::default(),
                ReplicationSender::new(Duration::ZERO, SendUpdatesMode::SinceLastAck, false),
                MessageReceiver::<ClientInput>::default(),
            ))
            .id();
        client_app.update();
        server_app.update();
        client_app.update();

        Self {
            server: NetworkedRpg::new(retention_ticks),
            client_app,
            server_app,
            client_link,
            server_link,
            replicated: BTreeMap::new(),
            pending: Vec::new(),
            received_inputs: 0,
            current_tick: 0,
        }
    }

    pub fn send(&mut self, input: ClientInput, latency_ticks: u32) {
        self.pending
            .push((input.tick.saturating_add(latency_ticks), input));
        self.pending
            .sort_by_key(|(send_tick, input)| (*send_tick, input.player, input.sequence));
    }

    pub fn advance_to(&mut self, target_tick: u32) {
        for tick in self.current_tick.saturating_add(1)..=target_tick {
            self.flush_scheduled_inputs(tick);
            self.pump_lightyear_inputs();
            self.server.advance_to(tick);
            self.sync_replicated_world();
            self.pump_lightyear_replication();
            self.pump_lightyear_replication();
            self.current_tick = tick;
        }
    }

    pub fn hp(&mut self, stable_id: StableEntityId) -> i32 {
        self.server.hp(stable_id)
    }

    pub fn death_markers_for(&mut self, victim: StableEntityId) -> usize {
        self.server.death_markers_for(victim)
    }

    pub fn corpses_for(&mut self, victim: StableEntityId) -> usize {
        self.server.corpses_for(victim)
    }

    pub fn loot_for(&mut self, owner: StableEntityId) -> usize {
        self.server.loot_for(owner)
    }

    pub fn inventory_food(&mut self, player: StableEntityId) -> u32 {
        self.server.inventory_food(player)
    }

    pub fn position(&mut self, stable_id: StableEntityId) -> Vec3i {
        self.server.position(stable_id)
    }

    pub fn facts(&self) -> &[CombatFact] {
        self.server.facts()
    }

    pub fn corrections(&self) -> &[Correction] {
        self.server.corrections()
    }

    pub fn rejected(&self) -> &[RejectedInput] {
        self.server.rejected()
    }

    pub fn has_afterglow_network_resources(&self) -> bool {
        self.server.has_afterglow_network_resources()
    }

    pub fn has_lightyear_links(&self) -> bool {
        let client = self.client_app.world().entity(self.client_link);
        let server = self.server_app.world().entity(self.server_link);
        client.contains::<Connected>()
            && client.contains::<Linked>()
            && server.contains::<Connected>()
            && server.contains::<Linked>()
    }

    pub fn received_lightyear_inputs(&self) -> usize {
        self.received_inputs
    }

    pub fn client_afterglow_lightyear_role(&self) -> Option<LightyearRole> {
        self.client_app
            .world()
            .get_resource::<AfterglowLightyearConfig>()
            .map(|config| config.role)
    }

    pub fn server_afterglow_lightyear_role(&self) -> Option<LightyearRole> {
        self.server_app
            .world()
            .get_resource::<AfterglowLightyearConfig>()
            .map(|config| config.role)
    }

    pub fn client_predicted_combatant(&mut self, stable_id: StableEntityId) -> Option<Combatant> {
        let mut query =
            self.client_app
                .world_mut()
                .query::<(&StableEntityId, &Combatant, Option<&Predicted>)>();
        query
            .iter(self.client_app.world())
            .find_map(|(id, combatant, predicted)| {
                (*id == stable_id && predicted.is_some()).then_some(combatant.clone())
            })
    }

    pub fn client_confirmed_combatant(&mut self, stable_id: StableEntityId) -> Option<Combatant> {
        let mut query =
            self.client_app
                .world_mut()
                .query::<(&StableEntityId, &Confirmed<Combatant>, Option<&Predicted>)>();
        query
            .iter(self.client_app.world())
            .find_map(|(id, combatant, predicted)| {
                (*id == stable_id && predicted.is_some()).then_some(combatant.0.clone())
            })
    }

    pub fn client_predicted_inventory_food(&mut self, stable_id: StableEntityId) -> Option<u32> {
        let mut query =
            self.client_app
                .world_mut()
                .query::<(&StableEntityId, &Inventory, Option<&Predicted>)>();
        query
            .iter(self.client_app.world())
            .find_map(|(id, inventory, predicted)| {
                (*id == stable_id && predicted.is_some()).then_some(inventory.food)
            })
    }

    pub fn client_confirmed_inventory_food(&mut self, stable_id: StableEntityId) -> Option<u32> {
        let mut query =
            self.client_app
                .world_mut()
                .query::<(&StableEntityId, &Confirmed<Inventory>, Option<&Predicted>)>();
        query
            .iter(self.client_app.world())
            .find_map(|(id, inventory, predicted)| {
                (*id == stable_id && predicted.is_some()).then_some(inventory.0.food)
            })
    }

    pub fn client_has_replicated(&mut self, stable_id: StableEntityId) -> bool {
        let mut query = self.client_app.world_mut().query::<&StableEntityId>();
        query
            .iter(self.client_app.world())
            .any(|id| *id == stable_id)
    }

    pub fn client_prediction_history_len<T: Component + Clone>(
        &mut self,
        stable_id: StableEntityId,
    ) -> usize {
        let mut query = self.client_app.world_mut().query::<(
            &StableEntityId,
            Option<&PredictionHistory<T>>,
            Option<&Predicted>,
        )>();
        query
            .iter(self.client_app.world())
            .find_map(|(id, history, predicted)| {
                (*id == stable_id && predicted.is_some()).then_some(history.map_or(0, |h| h.len()))
            })
            .unwrap_or_default()
    }

    fn flush_scheduled_inputs(&mut self, tick: u32) {
        let mut pending = Vec::new();
        let mut ready = Vec::new();
        for (send_tick, input) in self.pending.drain(..) {
            if send_tick <= tick {
                ready.push(input);
            } else {
                pending.push((send_tick, input));
            }
        }
        self.pending = pending;
        for input in ready {
            self.client_app
                .world_mut()
                .entity_mut(self.client_link)
                .get_mut::<MessageSender<ClientInput>>()
                .expect("client link should send RPG inputs")
                .send::<RpgInputChannel>(input);
        }
    }

    fn pump_lightyear_inputs(&mut self) {
        self.client_app.world_mut().run_schedule(PostUpdate);
        self.server_app.world_mut().run_schedule(PreUpdate);
        let inputs = self
            .server_app
            .world_mut()
            .entity_mut(self.server_link)
            .get_mut::<MessageReceiver<ClientInput>>()
            .expect("server link should receive RPG inputs")
            .receive()
            .collect::<Vec<_>>();
        for input in inputs {
            self.received_inputs = self.received_inputs.saturating_add(1);
            self.server.receive_network_input(input);
        }
    }

    fn pump_lightyear_replication(&mut self) {
        self.server_app.world_mut().run_schedule(PostUpdate);
        self.client_app.world_mut().run_schedule(PreUpdate);
        self.client_app.world_mut().run_schedule(FixedPostUpdate);
    }

    fn sync_replicated_world(&mut self) {
        let snapshot = self.server.snapshot();
        let desired = desired_stable_ids(&snapshot);
        let world = self.server_app.world_mut();
        self.replicated.retain(|stable, entity| {
            if desired.contains(stable) {
                true
            } else {
                if let Ok(entity_mut) = world.get_entity_mut(*entity) {
                    entity_mut.despawn();
                }
                false
            }
        });
        for stable in &desired {
            self.replicated.entry(*stable).or_insert_with(|| {
                world
                    .spawn((
                        *stable,
                        Replicate::to_clients(NetworkTarget::All),
                        PredictionTarget::to_clients(NetworkTarget::All),
                    ))
                    .id()
            });
        }
        sync_components(world, &self.replicated, snapshot);
        world.flush();
    }
}

fn desired_stable_ids(snapshot: &CombatSnapshot) -> BTreeSet<StableEntityId> {
    snapshot
        .combatants
        .iter()
        .map(|(id, _)| *id)
        .chain(snapshot.inventories.iter().map(|(id, _)| *id))
        .chain(snapshot.projectiles.iter().map(|(id, _)| *id))
        .chain(snapshot.death_markers.iter().map(|(id, _)| *id))
        .chain(snapshot.corpses.iter().map(|(id, _)| *id))
        .chain(snapshot.loot.iter().map(|(id, _)| *id))
        .collect()
}

fn sync_components(
    world: &mut World,
    entities: &BTreeMap<StableEntityId, Entity>,
    snapshot: CombatSnapshot,
) {
    sync_component_set(world, entities, snapshot.combatants);
    sync_component_set(world, entities, snapshot.inventories);
    sync_component_set(world, entities, snapshot.projectiles);
    sync_component_set(world, entities, snapshot.death_markers);
    sync_component_set(world, entities, snapshot.corpses);
    sync_component_set(world, entities, snapshot.loot);
}

fn sync_component_set<T>(
    world: &mut World,
    entities: &BTreeMap<StableEntityId, Entity>,
    components: Vec<(StableEntityId, T)>,
) where
    T: Component,
{
    let present = components
        .iter()
        .map(|(stable, _)| *stable)
        .collect::<BTreeSet<_>>();
    for (stable, entity) in entities {
        if !present.contains(stable)
            && let Ok(mut entity_mut) = world.get_entity_mut(*entity)
        {
            entity_mut.remove::<T>();
        }
    }
    for (stable, component) in components {
        world.entity_mut(entities[&stable]).insert(component);
    }
}

fn lightyear_app(client: bool) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let role = if client {
        LightyearRole::Client
    } else {
        LightyearRole::Server
    };
    app.insert_resource(AfterglowLightyearConfig { role, ..default() });
    add_crossbeam_lightyear_plugins(&mut app, role);
    app.init_resource::<PeerMetadata>();
    register_protocol(&mut app);
    app.finish();
    app.cleanup();
    app
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
        LightyearRole::Host => app.add_plugins((
            lightyear::prelude::server::ServerPlugins { tick_duration }
                .build()
                .disable::<lightyear::prelude::server::NetcodeServerPlugin>(),
            lightyear::prelude::client::ClientPlugins { tick_duration }
                .build()
                .disable::<lightyear::prelude::client::NetcodeClientPlugin>(),
        )),
    };
}

fn register_protocol(app: &mut App) {
    app.add_channel::<RpgInputChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
        send_frequency: Duration::ZERO,
        priority: 1.0,
    })
    .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<ClientInput>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_component::<StableEntityId>();
    app.register_component::<Combatant>().add_prediction();
    app.register_component::<Inventory>().add_prediction();
    app.register_component::<Projectile>().add_prediction();
    app.register_component::<DeathMarker>();
    app.register_component::<Corpse>();
    app.register_component::<Loot>().add_prediction();
}

fn transport(app: &App, send: bool, receive: bool) -> Transport {
    let registry = app.world().resource::<ChannelRegistry>();
    let mut transport = Transport::default();
    if send {
        transport.add_sender_from_registry::<RpgInputChannel>(registry);
    }
    if receive {
        transport.add_receiver_from_registry::<RpgInputChannel>(registry);
    }
    transport.add_sender_from_registry::<MetadataChannel>(registry);
    transport.add_receiver_from_registry::<MetadataChannel>(registry);
    transport.add_sender_from_registry::<UpdatesChannel>(registry);
    transport.add_receiver_from_registry::<UpdatesChannel>(registry);
    transport
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_components_removes_stale_components_from_retained_entities() {
        let stable = StableEntityId::from_raw(99);
        let mut world = World::new();
        let entity = world
            .spawn((
                stable,
                Combatant::new(100, Vec3i::ZERO),
                Inventory { food: 1 },
            ))
            .id();
        let entities = BTreeMap::from([(stable, entity)]);
        let snapshot = CombatSnapshot {
            combatants: Vec::new(),
            inventories: vec![(stable, Inventory { food: 2 })],
            projectiles: Vec::new(),
            death_markers: Vec::new(),
            corpses: Vec::new(),
            loot: Vec::new(),
            log: CombatLog::default(),
        };

        sync_components(&mut world, &entities, snapshot);

        assert!(world.get::<Combatant>(entity).is_none());
        assert_eq!(world.get::<Inventory>(entity).unwrap().food, 2);
    }
}
