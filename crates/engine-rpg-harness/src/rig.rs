use afterglow_engine::core::identity::StableEntityId;
use bevy::{
    app::{FixedPreUpdate, PostUpdate},
    prelude::*,
    time::Time,
};
use lightyear::prelude::{
    server::{ClientOf, Start},
    *,
};
use std::{collections::HashMap, time::Duration};

mod setup;

const UDP_CONNECT_TIMEOUT_TICKS: u32 = 120;

#[derive(Clone, Copy, Debug, Default)]
pub enum TransportConfig {
    #[default]
    Crossbeam,
    Udp {
        /// `0` selects a process-local dynamic test port.
        server_port: u16,
    },
}

/// Infrastructure-only test rig for Lightyear + Bevy apps.
///
/// Owns server + N client apps, tick-level schedule stepping, and supports
/// both Crossbeam (in-memory) and UDP (real sockets) transports.
/// Does NOT own any game/RPG concepts — those go in a scenario helper on top.
///
/// Supports optional input delay: when `input_delay_ticks > 0`, actions queued
/// via `queue_action` are held until the target tick before being applied to
/// the server app. This lets scenario code simulate delayed input arrival
/// without server rewind.
pub struct LightyearTestRig {
    pub server_app: App,
    pub client_apps: Vec<App>,
    pub server_links: Vec<Entity>,
    pub client_links: Vec<Entity>,
    current_tick: u32,
    entity_map: HashMap<StableEntityId, Vec<Entity>>,
    pub input_delay_ticks: u32,
    pub tick_rate: u32,
    pending_inputs: Vec<(u32, u32, Box<dyn FnOnce(&mut App)>)>,
    /// Retention window in ticks. Inputs whose intended tick falls outside
    /// `[current_tick - retention_window_ticks, current_tick]` are dropped
    /// before delivery. 0 = no limit.
    retention_window_ticks: u32,
}

impl LightyearTestRig {
    /// Establish network connections.
    ///
    /// For Crossbeam transport this is a no-op (links are already connected).
    /// For UDP transport this triggers the netcode handshake and blocks until
    /// all clients are connected, then populates `server_links`. The UDP
    /// handshake budget is `UDP_CONNECT_TIMEOUT_TICKS` rig ticks.
    pub fn connect(&mut self) {
        if !self.server_links.is_empty() {
            return;
        }

        let server_entity = self
            .server_app
            .world_mut()
            .query_filtered::<Entity, With<server::NetcodeServer>>()
            .iter(self.server_app.world())
            .next()
            .expect("server entity with NetcodeServer not found");

        {
            let mut c = self.server_app.world_mut().commands();
            c.trigger(Start {
                entity: server_entity,
            });
        }

        for (i, &client_entity) in self.client_links.iter().enumerate() {
            let mut c = self.client_apps[i].world_mut().commands();
            c.trigger(Connect {
                entity: client_entity,
            });
        }

        let expected_links = self.client_apps.len();
        let mut server_links = Vec::new();
        for _ in 0..UDP_CONNECT_TIMEOUT_TICKS {
            self.advance(1);
            let mut query = self
                .server_app
                .world_mut()
                .query_filtered::<(Entity, &RemoteId), (With<ClientOf>, With<Connected>)>();
            let mut links: Vec<(Entity, u64)> = query
                .iter(self.server_app.world())
                .map(|(entity, remote)| {
                    let order = match &remote.0 {
                        PeerId::Entity(id)
                        | PeerId::Netcode(id)
                        | PeerId::Steam(id)
                        | PeerId::Local(id) => *id,
                        PeerId::Raw(addr) => addr.port() as u64,
                        PeerId::Server => 0,
                    };
                    (entity, order)
                })
                .collect();
            links.sort_by_key(|(_, order)| *order);
            server_links = links.into_iter().map(|(entity, _)| entity).collect();
            if server_links.len() == expected_links {
                break;
            }
        }

        assert_eq!(
            server_links.len(),
            expected_links,
            "UDP connect expected {expected_links} server links after handshake, got {}",
            server_links.len()
        );

        self.server_links = server_links;
    }

    /// Spawn an entity on the server with Lightyear replication markers and
    /// immediately trigger replication so clients receive it.
    ///
    /// Returns the server Entity. Call `connect()` before this on UDP rigs so
    /// the target client links exist.
    pub fn spawn_replicated(&mut self, sid: StableEntityId, components: impl Bundle) -> Entity {
        assert!(
            !self.server_links.is_empty(),
            "spawn_replicated requires connected clients; call connect() before spawning UDP entities"
        );
        let entity = self
            .server_app
            .world_mut()
            .spawn((
                sid,
                components,
                Replicate::to_clients(NetworkTarget::All),
                PredictionTarget::to_clients(NetworkTarget::All),
            ))
            .id();
        self.server_app.world_mut().run_schedule(PostUpdate);
        for client in &mut self.client_apps {
            client.world_mut().run_schedule(PreUpdate);
        }
        entity
    }

    pub fn server_component<C: Component>(&self, entity: Entity) -> Option<&C> {
        self.server_app.world().get::<C>(entity)
    }

    pub fn client_component<C: Component>(&self, client_id: usize, entity: Entity) -> Option<&C> {
        self.client_apps[client_id].world().get::<C>(entity)
    }

    /// Find a client-world entity by its StableEntityId.
    /// Requires the engine Lightyear protocol helper to register
    /// `StableEntityId` for replication.
    pub fn find_client_entity(&mut self, client_id: usize, sid: StableEntityId) -> Option<Entity> {
        let mut query = self.client_apps[client_id]
            .world_mut()
            .query::<(Entity, &StableEntityId)>();
        query
            .iter(self.client_apps[client_id].world())
            .find_map(|(entity, id)| if *id == sid { Some(entity) } else { None })
    }

    /// Set input delay in milliseconds. Converts to ticks at the current tick
    /// rate. Only affects actions dispatched through `queue_action`.
    pub fn with_input_delay_ms(mut self, delay_ms: u32) -> Self {
        let frame_time_ms = 1000 / self.tick_rate.max(1);
        self.input_delay_ticks = delay_ms / frame_time_ms;
        self
    }

    /// Set retention window in ticks. Inputs whose intended tick falls outside
    /// `[current_tick - retention_window_ticks, current_tick]` are silently
    /// dropped before delivery. 0 (default) = no limit.
    pub fn with_retention_window_ticks(mut self, ticks: u32) -> Self {
        self.retention_window_ticks = ticks;
        self
    }

    /// Queue a closure to run on the server app at `tick + input_delay_ticks`.
    /// This allows scenarios to simulate delayed input arrival.
    pub fn queue_action(&mut self, tick: u32, action: impl FnOnce(&mut App) + 'static) {
        let deliver_at = tick
            .checked_add(self.input_delay_ticks)
            .expect("tick overflow");
        self.pending_inputs
            .push((tick, deliver_at, Box::new(action)));
    }

    /// Queue a closure with explicit intended-tick and deliver-at values.
    /// Bypasses the normal delay computation — useful for simulating late
    /// arrivals in retention-window tests.
    pub fn queue_action_at_deliver_tick(
        &mut self,
        intended_tick: u32,
        deliver_at: u32,
        action: impl FnOnce(&mut App) + 'static,
    ) {
        self.pending_inputs
            .push((intended_tick, deliver_at, Box::new(action)));
    }

    pub fn server_link(&self, client_id: usize) -> Entity {
        self.server_links[client_id]
    }

    pub fn client_link(&self, client_id: usize) -> Entity {
        self.client_links[client_id]
    }

    pub fn server_world(&self) -> &World {
        self.server_app.world()
    }

    pub fn server_world_mut(&mut self) -> &mut World {
        self.server_app.world_mut()
    }

    pub fn client_world(&self, client_id: usize) -> &World {
        self.client_apps[client_id].world()
    }

    pub fn client_world_mut(&mut self, client_id: usize) -> &mut World {
        self.client_apps[client_id].world_mut()
    }

    /// Register a mapping from `StableEntityId` to its local entity handles.
    /// The rig maintains this so scenario code can look up entities by stable
    /// ID without tracking spawn order manually.
    ///
    /// Vec layout: index 0 = server entity, index 1+ = client entities in
    /// order.
    pub fn register_entity(&mut self, sid: StableEntityId, entities: Vec<Entity>) {
        assert_eq!(
            entities.len(),
            1 + self.client_apps.len(),
            "register_entity needs 1 server + {} client entities, got {}",
            self.client_apps.len(),
            entities.len()
        );
        self.entity_map.insert(sid, entities);
    }

    /// Look up the server-side Entity for a StableEntityId.
    /// Panics if not registered (call `register_entity` after spawning).
    pub fn server_entity(&self, sid: StableEntityId) -> Entity {
        self.entity_map
            .get(&sid)
            .and_then(|entities| entities.first())
            .copied()
            .expect("StableEntityId not registered. Call register_entity after spawning.")
    }

    /// Look up a client-side Entity for a StableEntityId.
    /// `entities` stores server entity first, then client entities in order.
    /// Index 0 = server, index 1+ = clients. Returns the client entity at
    /// `client_id` offset (0 = first client, etc.).
    pub fn client_entity(&self, sid: StableEntityId, client_id: usize) -> Entity {
        self.entity_map
            .get(&sid)
            .and_then(|entities| entities.get(1 + client_id))
            .copied()
            .unwrap_or_else(|| {
                panic!("StableEntityId {sid:?} or client {client_id} not registered")
            })
    }

    /// Advance the simulation by `delta` ticks.
    ///
    /// Per-tick schedule order:
    ///   1. Clients: PreUpdate (receive replication)
    ///   2. Clients: FixedFirst (advance Lightyear tick)
    ///   3. Clients: FixedPreUpdate (Leafwing input buffering, user input
    ///      writes)
    ///   4. Clients: FixedUpdate (predict physics/controller)
    ///   5. Clients: PostUpdate (send messages)
    ///   6. Server:  PreUpdate (receive client messages)
    ///   7. Server:  drain pending inputs (queued via `queue_action`) whose
    ///      delivery tick <= current tick
    ///   8. Server:  FixedFirst (advance Lightyear tick)
    ///   9. Server:  FixedPreUpdate (server-side input state update)
    ///  10. Server:  FixedUpdate (authoritative simulation)
    ///  11. Server:  FixedPostUpdate (authoritative output / Lightyear hooks)
    ///  12. Server:  PostUpdate (send replication)
    ///  13. Clients: FixedPostUpdate (reconcile prediction vs confirmed)
    pub fn advance(&mut self, delta: u32) {
        for _ in 0..delta {
            self.current_tick = self.current_tick.checked_add(1).expect("tick overflow");

            for client in &mut self.client_apps {
                advance_real_time(client);
                client.world_mut().run_schedule(PreUpdate);
                client.world_mut().run_schedule(FixedFirst);
                let _ = client.world_mut().try_run_schedule(FixedPreUpdate);
                client.world_mut().run_schedule(FixedUpdate);
                client.world_mut().run_schedule(PostUpdate);
            }

            advance_real_time(&mut self.server_app);
            self.server_app.world_mut().run_schedule(PreUpdate);

            let tick = self.current_tick;

            // Stale-input rejection: drop inputs whose intended tick is outside
            // the retention window
            if self.retention_window_ticks > 0 {
                let window_start = tick.saturating_sub(self.retention_window_ticks);
                self.pending_inputs
                    .retain(|(intended, _, _)| *intended >= window_start);
            }

            let mut i = 0;
            while i < self.pending_inputs.len() {
                if self.pending_inputs[i].1 <= tick {
                    let (_intended, _deliver_at, action) = self.pending_inputs.swap_remove(i);
                    action(&mut self.server_app);
                } else {
                    i += 1;
                }
            }

            self.server_app.world_mut().run_schedule(FixedFirst);
            let _ = self.server_app.world_mut().try_run_schedule(FixedPreUpdate);
            self.server_app.world_mut().run_schedule(FixedUpdate);
            self.server_app.world_mut().run_schedule(FixedPostUpdate);
            self.server_app.world_mut().run_schedule(PostUpdate);

            for client in &mut self.client_apps {
                client.world_mut().run_schedule(FixedPostUpdate);
            }
        }
    }

    /// Absolute tick advance. No-op if target is at or behind current tick.
    /// This method only advances forward; callers that need earlier state must
    /// manage their own snapshot/restore cycle externally.
    pub fn advance_to(&mut self, target: u32) {
        if target > self.current_tick {
            self.advance(target - self.current_tick);
        }
    }

    pub fn current_tick(&self) -> u32 {
        self.current_tick
    }
}

fn advance_real_time(app: &mut App) {
    let tick_duration = Duration::from_secs_f64(1.0 / 60.0);
    if let Some(mut time) = app.world_mut().get_resource_mut::<Time<Real>>() {
        time.advance_by(tick_duration);
    }
}
