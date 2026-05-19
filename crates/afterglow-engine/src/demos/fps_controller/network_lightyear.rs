use bevy::{app::PreUpdate, prelude::*};
use lightyear::{
    connection::client_of::SkipNetcode,
    crossbeam::CrossbeamIo,
    prelude::{
        server::{ClientOf, Started},
        *,
    },
};
use std::{collections::BTreeMap, time::Duration};

use crate::{
    controller::{FirstPersonMotorState, ReplayCommand},
    core::identity::StableEntityId,
    input::AfterglowAction,
    network::{AfterglowLightyearConfig, LightyearRole},
};

use super::{
    FPS_DEMO_PLAYER_ID, FPS_DEMO_REMOTE_PLAYER_ID, FpsDemoInputCommand, FpsDemoPlayerState,
    network_input::{first_person_command_state, motor_from_player_state},
    network_protocol::{FpsStateChannel, fps_demo_transport, register_fps_demo_lightyear_protocol},
};

pub(super) struct FpsDemoLocalLightyear {
    client_app: App,
    bot_app: App,
    server_app: App,
    client_link: Entity,
    bot_link: Entity,
    server_links: Vec<Entity>,
    server_avatars: BTreeMap<StableEntityId, Entity>,
    pub(super) ticks: u32,
}

impl FpsDemoLocalLightyear {
    pub(super) fn new() -> Self {
        let mut client_app = lightyear_app(true);
        let mut bot_app = lightyear_app(true);
        let mut server_app = lightyear_app(false);
        let server_entity = server_app
            .world_mut()
            .spawn((Server::default(), Started))
            .id();
        let (client_link, server_link_a) = spawn_link_pair(
            &mut client_app,
            &mut server_app,
            server_entity,
            PeerId::Local(1),
        );
        let (bot_link, server_link_b) = spawn_link_pair(
            &mut bot_app,
            &mut server_app,
            server_entity,
            PeerId::Local(2),
        );
        // The server runs collision-aware physics, so it needs the full demo scene.
        let mut server_world = server_app.world_mut();
        let wall = |w: &mut World, size: Vec3, pos: Vec3| {
            w.spawn((
                crate::physics::PhysicsBody::static_body(),
                crate::physics::PhysicsCollider::cuboid(size),
                Transform::from_translation(pos),
            ));
        };
        wall(
            &mut server_world,
            Vec3::new(28.0, 0.4, 28.0),
            Vec3::new(0.0, -0.2, 0.0),
        );
        wall(
            &mut server_world,
            Vec3::new(28.0, 3.0, 0.4),
            Vec3::new(0.0, 1.3, -14.0),
        );
        wall(
            &mut server_world,
            Vec3::new(28.0, 3.0, 0.4),
            Vec3::new(0.0, 1.3, 14.0),
        );
        wall(
            &mut server_world,
            Vec3::new(0.4, 3.0, 28.0),
            Vec3::new(-14.0, 1.3, 0.0),
        );
        wall(
            &mut server_world,
            Vec3::new(0.4, 3.0, 28.0),
            Vec3::new(14.0, 1.3, 0.0),
        );
        wall(
            &mut server_world,
            Vec3::new(1.5, 0.5, 3.0),
            Vec3::new(2.5, 0.25, -2.0),
        );

        let mut runner = Self {
            client_app,
            bot_app,
            server_app,
            client_link,
            bot_link,
            server_links: vec![server_link_a, server_link_b],
            server_avatars: BTreeMap::new(),
            ticks: 0,
        };

        let player_state = FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0));
        runner.ensure_server_avatar(
            FPS_DEMO_PLAYER_ID,
            player_state.clone(),
            motor_from_player_state(&player_state),
        );
        let remote_state = FpsDemoPlayerState::from_translation(Vec3::new(2.0, 0.95, 2.0));
        runner.ensure_server_avatar(
            FPS_DEMO_REMOTE_PLAYER_ID,
            remote_state.clone(),
            motor_from_player_state(&remote_state),
        );
        runner.client_app.update();
        runner.bot_app.update();
        runner.server_app.update();
        runner.client_app.update();
        runner.bot_app.update();
        runner.pump_once();
        runner.pump_once();
        runner
    }

    pub(super) fn send_player_input(&mut self, command: FpsDemoInputCommand) {
        send_command(&mut self.client_app, self.client_link, command);
    }

    pub(super) fn pump_once(&mut self) {
        self.send_bot_state();
        self.client_app.update();
        self.bot_app.update();
        self.server_app.world_mut().run_schedule(PreUpdate);
        self.apply_server_updates();
        self.server_app.update();
        self.sync_avatar_state_for_replication();
        self.client_app.update();
        self.bot_app.update();
        self.ticks = self.ticks.saturating_add(1);
    }

    pub(super) fn has_lightyear_links(&self) -> bool {
        let client = self.client_app.world().entity(self.client_link);
        let bot = self.bot_app.world().entity(self.bot_link);
        client.contains::<Connected>()
            && client.contains::<Linked>()
            && bot.contains::<Connected>()
            && bot.contains::<Linked>()
            && self.server_links.iter().all(|entity| {
                let link = self.server_app.world().entity(*entity);
                link.contains::<Connected>() && link.contains::<Linked>()
            })
    }

    pub(super) fn client_has_replicated_avatar(&mut self) -> bool {
        self.replicated_avatar_states()
            .iter()
            .any(|(id, _)| *id == FPS_DEMO_PLAYER_ID)
    }

    pub(super) fn replicated_avatar_states(&mut self) -> Vec<(StableEntityId, FpsDemoPlayerState)> {
        let world = self.client_app.world_mut();
        let mut query = world.query::<(&StableEntityId, &FpsDemoPlayerState)>();
        let mut states = query
            .iter(world)
            .map(|(id, state)| (*id, state.clone()))
            .collect::<Vec<_>>();
        states.sort_by_key(|(id, _)| *id);
        states
    }

    #[cfg(test)]
    pub(super) fn local_player_server_state(&self) -> Option<FpsDemoPlayerState> {
        self.server_avatar_state(FPS_DEMO_PLAYER_ID)
    }

    fn send_bot_state(&mut self) {
        let x = 2.0 + (self.ticks as f32 * 0.03).sin();
        let z = 2.0 + (self.ticks as f32 * 0.03).cos();
        send_command(
            &mut self.bot_app,
            self.bot_link,
            FpsDemoInputCommand {
                player: FPS_DEMO_REMOTE_PLAYER_ID,
                tick: self.ticks,
                move_axis: Vec2::new(x.sin(), z.cos()),
                look_axis: Vec2::ZERO,
                jump_held: false,
                crouch_held: false,
                sprint_held: false,
            },
        );
    }

    fn apply_server_updates(&mut self) {
        let mut updates = Vec::new();
        for link in &self.server_links {
            let received = self
                .server_app
                .world_mut()
                .entity_mut(*link)
                .get_mut::<MessageReceiver<FpsDemoInputCommand>>()
                .expect("server link should receive FPS updates")
                .receive()
                .collect::<Vec<_>>();
            updates.extend(received);
        }
        for update in updates {
            let default_state = FpsDemoPlayerState::from_translation(Vec3::new(0.0, 0.95, 4.0));
            let default_motor = motor_from_player_state(&default_state);
            self.ensure_server_avatar(update.player, default_state, default_motor);
            let entity = self.server_avatars[&update.player];
            let command_state = first_person_command_state(&update);
            self.server_app
                .world_mut()
                .entity_mut(entity)
                .insert(ReplayCommand(command_state));
        }
    }

    /// After server FixedUpdate + physics, sync avatar Transform + MotorState
    /// into FpsDemoPlayerState for Lightyear replication.
    fn sync_avatar_state_for_replication(&mut self) {
        let world = self.server_app.world_mut();
        let mut query = world.query::<(
            &StableEntityId,
            &Transform,
            &FirstPersonMotorState,
            &mut FpsDemoPlayerState,
        )>();
        for (_stable_id, transform, motor, mut state) in query.iter_mut(world) {
            let tick = state.authoritative_tick.max(1);
            *state = FpsDemoPlayerState::from_translation(transform.translation);
            state.yaw_milliradians = (motor.yaw * 1000.0).round() as i32;
            state.pitch_milliradians = (motor.pitch * 1000.0).round() as i32;
            state.authoritative_tick = tick;
        }
    }

    #[cfg(test)]
    fn server_avatar_state(&self, stable_id: StableEntityId) -> Option<FpsDemoPlayerState> {
        let entity = self.server_avatars.get(&stable_id)?;
        self.server_app
            .world()
            .get::<FpsDemoPlayerState>(*entity)
            .cloned()
    }

    fn ensure_server_avatar(
        &mut self,
        stable_id: StableEntityId,
        state: FpsDemoPlayerState,
        motor: FirstPersonMotorState,
    ) {
        let world = self.server_app.world_mut();
        let _ = self.server_avatars.entry(stable_id).or_insert_with(|| {
            world
                .spawn((
                    stable_id,
                    state.clone(),
                    motor,
                    crate::controller::FirstPersonController::new(),
                    crate::physics::PhysicsBody::kinematic(),
                    crate::physics::PhysicsCollider::cylinder(
                        crate::controller::FirstPersonControllerConfig::default().body_radius,
                        crate::controller::FirstPersonControllerConfig::default().standing_height,
                    ),
                    Transform::from_translation(state.to_translation()),
                    Replicate::to_clients(NetworkTarget::All),
                ))
                .id()
        });
    }
}

fn send_command(app: &mut App, link: Entity, command: FpsDemoInputCommand) {
    app.world_mut()
        .entity_mut(link)
        .get_mut::<MessageSender<FpsDemoInputCommand>>()
        .expect("client link should send FPS updates")
        .send::<FpsStateChannel>(command);
}

fn spawn_link_pair(
    client_app: &mut App,
    server_app: &mut App,
    server_entity: Entity,
    client_id: PeerId,
) -> (Entity, Entity) {
    let (client_io, server_io) = CrossbeamIo::new_pair();
    let client_transport = fps_demo_transport(
        client_app.world().resource::<ChannelRegistry>(),
        true,
        false,
    );
    let server_transport = fps_demo_transport(
        server_app.world().resource::<ChannelRegistry>(),
        false,
        true,
    );
    let client_link = client_app
        .world_mut()
        .spawn((
            Client::default(),
            LocalId(client_id),
            RemoteId(PeerId::Server),
            Connected,
            Link::default(),
            Linked,
            client_io,
            client_transport,
            MessageManager::default(),
            ReplicationReceiver::default(),
            PredictionManager::default(),
            MessageSender::<FpsDemoInputCommand>::default(),
        ))
        .id();
    let server_link = server_app
        .world_mut()
        .spawn((
            LinkOf {
                server: server_entity,
            },
            ClientOf,
            SkipNetcode,
            LocalId(PeerId::Server),
            RemoteId(client_id),
            Connected,
            Link::default(),
            Linked,
            server_io,
            server_transport,
            MessageManager::default(),
            ReplicationSender::new(Duration::ZERO, SendUpdatesMode::SinceLastAck, false),
            MessageReceiver::<FpsDemoInputCommand>::default(),
        ))
        .id();
    (client_link, server_link)
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
    // Server sub-app needs collision-aware physics so its avatar states
    // don't diverge from the real controller path on clients.
    if !client {
        app.add_plugins((
            crate::core::AfterglowCorePlugin,
            crate::physics::AfterglowPhysicsPlugin,
            crate::controller::AfterglowFirstPersonControllerPlugin,
        ));
    }
    add_crossbeam_lightyear_plugins(&mut app, role);
    register_fps_demo_lightyear_protocol(&mut app);
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
    app.add_plugins(lightyear_inputs_leafwing::prelude::InputPlugin::<
        AfterglowAction,
    >::default());
}
