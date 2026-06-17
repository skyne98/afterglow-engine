pub mod camera;
pub mod movement;
pub mod network;
pub mod playground;
pub mod protocol;
pub mod rope;
pub mod scene;
#[cfg(test)]
pub mod tests;

use bevy::prelude::*;
use lightyear::prelude::{ReplicationSystems, RollbackSystems, client::input::InputSystems};
use std::net::SocketAddr;

use crate::network::{
    lightyear::{AfterglowLightyearConfig, LightyearRole},
    session::{
        AfterglowSessionState, NonSteamSessionClient, PlayerIdentity, ProviderEndpoint,
        SessionBackend, SessionCode, SessionRequest, SessionSearch, SessionStatus,
    },
};
use movement::DemoInput;
use network::register_demo_protocol;
use scene::{MemberToPlayer, PlayerName};

#[derive(Clone)]
pub struct MultiplayerBoxesDemoConfig {
    pub player_name: String,
    pub host: bool,
    pub listen: String,
    pub connect: String,
}

#[derive(Resource)]
pub struct ServerAddr(pub SocketAddr);

#[derive(Resource)]
pub struct LocalIdentity(pub PlayerIdentity);

#[allow(dead_code)]
#[derive(Resource, Default)]
enum ClientJoinState {
    #[default]
    Idle,
    Search,
    SearchSent,
    Join(SessionCode),
    Joining,
    Joined,
    Failed,
}

pub struct MultiplayerBoxesPlugin;

impl Plugin for MultiplayerBoxesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerName>()
            .init_resource::<DemoInput>()
            .init_resource::<ClientJoinState>()
            .init_resource::<MemberToPlayer>();

        register_demo_protocol(app);

        app.add_systems(
            Startup,
            (
                scene::spawn_arena,
                scene::spawn_host_player,
                scene::spawn_lights,
            )
                .run_if(|config: Res<AfterglowLightyearConfig>| {
                    matches!(config.role, LightyearRole::Host)
                }),
        );

        app.add_systems(
            Startup,
            (
                scene::spawn_client_arena_visuals,
                scene::spawn_lights,
                client_start_search,
            )
                .run_if(|config: Res<AfterglowLightyearConfig>| {
                    matches!(config.role, LightyearRole::Client)
                }),
        );

        app.add_systems(
            Update,
            movement::add_input_map_to_local_predicted_player
                .after(scene::attach_replicated_player_visuals),
        );
        app.add_systems(
            FixedPreUpdate,
            movement::collect_input.in_set(InputSystems::WriteClientInputs),
        );
        app.add_systems(
            PreUpdate,
            (
                scene::attach_predicted_player_physics,
                scene::attach_predicted_kinematic_physics,
            )
                .after(ReplicationSystems::Receive)
                .run_if(|config: Res<AfterglowLightyearConfig>| {
                    matches!(config.role, LightyearRole::Client)
                }),
        );
        app.add_systems(
            Update,
            (
                client_join_flow,
                scene::attach_replicated_player_visuals,
                scene::attach_replicated_kinematic_visuals,
            )
                .run_if(|config: Res<AfterglowLightyearConfig>| {
                    matches!(config.role, LightyearRole::Client)
                }),
        );
        app.add_systems(
            Update,
            camera::setup_camera
                .run_if(|cam: Query<&camera::DemoCamera>| cam.is_empty())
                .after(scene::attach_replicated_player_visuals),
        );
        app.add_systems(
            PostUpdate,
            camera::follow_camera_system
                .after(lightyear::frame_interpolation::FrameInterpolationSystems::Interpolate)
                .after(RollbackSystems::VisualCorrection),
        );
        app.add_systems(
            Update,
            (
                scene::spawn_player_on_member_joined,
                scene::despawn_player_on_member_left,
            )
                .run_if(|config: Res<AfterglowLightyearConfig>| {
                    matches!(config.role, LightyearRole::Host)
                }),
        );

        app.add_systems(
            FixedUpdate,
            movement::apply_movement
                .run_if(|config: Res<AfterglowLightyearConfig>| {
                    matches!(config.role, LightyearRole::Host | LightyearRole::Client)
                })
                .before(avian3d::schedule::PhysicsSystems::Prepare),
        );

        // Rope mechanic: reactive observers, no polling sync
        app.add_systems(
            PreUpdate,
            rope::toggle_rope
                .after(leafwing_input_manager::plugin::InputManagerSystem::Update)
                .run_if(|config: Res<AfterglowLightyearConfig>| {
                    matches!(config.role, LightyearRole::Host | LightyearRole::Client)
                }),
        );
        app.add_observer(rope::on_roped_to_added);
        app.add_observer(rope::on_roped_to_removed);

        // Local-only visual: highlight nearest box (runs on all sides in Update)
        app.add_systems(
            Update,
            (rope::highlight_nearest_box, rope::update_highlight_colors),
        );
    }
}

fn client_start_search(
    mut client: ResMut<NonSteamSessionClient>,
    server_addr: Res<ServerAddr>,
    mut state: ResMut<ClientJoinState>,
) {
    if !matches!(*state, ClientJoinState::Idle) {
        return;
    }
    let _ = client.send_request(
        &ProviderEndpoint::Udp(server_addr.0),
        &SessionRequest::Search(SessionSearch {
            backend: SessionBackend::NonSteam,
            provider: ProviderEndpoint::Udp(server_addr.0),
            metadata: [("name".into(), "multiplayer-boxes".into())].into(),
            require_open_slot: true,
            max_results: 16,
        }),
    );
    *state = ClientJoinState::Search;
}

fn client_join_flow(
    mut client: ResMut<NonSteamSessionClient>,
    server_addr: Res<ServerAddr>,
    status: Res<SessionStatus>,
    session_state: Res<AfterglowSessionState>,
    nonce: Res<crate::network::session::SessionIdentityNonce>,
    mut join_state: ResMut<ClientJoinState>,
) {
    match *join_state {
        ClientJoinState::Search | ClientJoinState::SearchSent => {
            if !status.last_search_results.is_empty() {
                let info = &status.last_search_results[0];
                let code = info.code.clone();
                let identity = PlayerIdentity::demo(&nonce.0, code.as_str(), 1);
                let _ = client.send_request(
                    &ProviderEndpoint::Udp(server_addr.0),
                    &SessionRequest::JoinByCode {
                        backend: SessionBackend::NonSteam,
                        code: code.clone(),
                        identity,
                        provider: ProviderEndpoint::Udp(server_addr.0),
                    },
                );
                *join_state = ClientJoinState::Join(code);
            } else {
                *join_state = ClientJoinState::SearchSent;
            }
        }
        ClientJoinState::Join(..) | ClientJoinState::Joining => {
            if session_state.is_in_session() {
                *join_state = ClientJoinState::Joined;
                bevy::log::info!("client joined session");
            } else {
                *join_state = ClientJoinState::Joining;
            }
        }
        ClientJoinState::Joined | ClientJoinState::Failed => {}
        ClientJoinState::Idle => {}
    }
}
