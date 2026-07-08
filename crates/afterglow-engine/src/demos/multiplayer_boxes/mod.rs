pub mod camera;
pub mod client;
pub mod movement;
pub mod network;
pub mod playground;
pub mod protocol;
pub mod rope;
pub mod rope_visual;
pub mod scene;
pub mod server;
#[cfg(test)]
pub mod tests;

use bevy::{app::ScheduleRunnerPlugin, prelude::*};
use std::{net::SocketAddr, time::Duration};

use crate::network::{
    connection::{
        AfterglowConnectionPlugin, ConnectionConfig, LocalIdentity, NetcodeConfig, ServerAddr,
        ServerListenAddr,
    },
    lightyear::{AfterglowLightyearConfig, AfterglowLightyearPlugin, LightyearRole},
};

#[derive(Clone)]
pub struct MultiplayerBoxesDemoConfig {
    pub player_name: String,
    pub host: bool,
    pub listen: String,
    pub connect: String,
}

// ---------------------------------------------------------------------------
// Server App (headless, MinimalPlugins)
// ---------------------------------------------------------------------------

pub fn run_multiplayer_boxes_server(listen: &str) {
    let listen_addr: SocketAddr = listen.parse().expect("invalid --listen address");

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        ))),
        bevy::transform::TransformPlugin,
    ));

    app.insert_resource(AfterglowLightyearConfig {
        role: LightyearRole::Server,
        tick_rate: 60,
        rebroadcast_inputs: false,
        ..Default::default()
    })
    .insert_resource(ConnectionConfig {
        require_auth: false,
        ..Default::default()
    })
    .insert_resource(ServerListenAddr(listen_addr))
    .insert_resource(LocalIdentity::unauthenticated(0));

    app.add_plugins((
        crate::core::AfterglowCorePlugin,
        AfterglowLightyearPlugin,
        AfterglowConnectionPlugin::server(NetcodeConfig {
            protocol_id: 42,
            private_key: [42u8; 32],
        }),
        crate::physics::AfterglowPhysicsPlugin,
        server::MultiplayerBoxesServerPlugin,
    ));

    app.run();
}

// ---------------------------------------------------------------------------
// Client App (DefaultPlugins with rendering)
// ---------------------------------------------------------------------------

pub fn run_multiplayer_boxes_client(connect: &str, player_name: &str) -> bevy::app::AppExit {
    let connect_addr: SocketAddr = connect.parse().expect("invalid --connect address");

    let trace_data = crate::perf_hud::setup_tracing();

    let mut app = App::new();
    crate::keep_windowed_runtime_unthrottled_when_unfocused(&mut app);

    app.insert_resource(trace_data)
        .insert_resource(AfterglowLightyearConfig {
            role: LightyearRole::Client,
            tick_rate: 60,
            rebroadcast_inputs: false,
            ..Default::default()
        })
        .insert_resource(ConnectionConfig {
            require_auth: false,
            ..Default::default()
        })
        .insert_resource(ServerAddr(connect_addr))
        .insert_resource(LocalIdentity::load_or_create_named(&player_name));

    app.add_plugins((
        crate::default_plugins(),
        crate::core::AfterglowCorePlugin,
        AfterglowLightyearPlugin,
        AfterglowConnectionPlugin::client(NetcodeConfig {
            protocol_id: 42,
            private_key: [42u8; 32],
        }),
        crate::physics::AfterglowPhysicsPlugin,
        client::MultiplayerBoxesClientPlugin,
    ));

    app.run()
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run_multiplayer_boxes_demo(config: MultiplayerBoxesDemoConfig) -> bevy::app::AppExit {
    if config.host {
        let listen_addr: SocketAddr = config.listen.parse().expect("invalid --listen address");

        // Spawn the server App in a detached thread.
        let server_listen = listen_addr.to_string();
        std::thread::Builder::new()
            .name("boxes-server".into())
            .spawn(move || {
                run_multiplayer_boxes_server(&server_listen);
            })
            .expect("failed to spawn server thread");

        // Give the server a moment to bind before client connects
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Connect to 127.0.0.1, not the listen address (which may be 0.0.0.0
        // and is not a valid connect target on Linux).
        let client_connect = if listen_addr.ip().is_unspecified() {
            format!("127.0.0.1:{}", listen_addr.port())
        } else {
            listen_addr.to_string()
        };
        run_multiplayer_boxes_client(&client_connect, &config.player_name)
    } else {
        run_multiplayer_boxes_client(&config.connect, &config.player_name)
    }
}
