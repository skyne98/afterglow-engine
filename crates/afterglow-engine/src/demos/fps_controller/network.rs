use bevy::prelude::*;

#[cfg(feature = "lightyear")]
use crate::controller::{FirstPersonMotorState, PredictionErrorSmoothing};
use crate::{
    console::{ConsoleNetworkRequest, ConsoleNetworkState},
    core::{
        identity::{Replicated, StableEntityId},
        schedule::AfterglowSet,
    },
    network::{AfterglowLightyearConfig, LightyearLinkConditioner, LightyearRole},
};

use super::FpsDemoPlayer;

#[cfg(feature = "lightyear")]
#[path = "network_input.rs"]
mod network_input;
#[cfg(all(feature = "lightyear", not(target_family = "wasm")))]
#[path = "network_native.rs"]
mod network_native;
#[cfg(feature = "lightyear")]
#[path = "network_protocol.rs"]
mod network_protocol;
#[path = "network_types.rs"]
mod network_types;
#[path = "network_visuals.rs"]
mod network_visuals;

#[cfg(feature = "lightyear")]
pub(crate) use network_input::FpsDemoInputCommand;
#[cfg(feature = "lightyear")]
use network_input::FpsDemoPredictionBuffer;
#[cfg(feature = "lightyear")]
use network_input::fps_demo_input_command;
#[cfg(all(feature = "lightyear", not(target_family = "wasm")))]
use network_native::{
    FpsDemoNativeServerAvatar, FpsServerHasProcessedInput, update_native_lightyear,
};
pub use network_types::{
    FpsDemoConnectionState, FpsDemoLaunchMode, FpsDemoNetworkConfig, FpsDemoNetworkStatus,
    FpsDemoPlayerState, FpsDemoRemoteAvatar,
};

pub const FPS_DEMO_PLAYER_ID: StableEntityId = StableEntityId::from_raw(10_000_001);
#[cfg_attr(not(feature = "lightyear"), allow(dead_code))]
pub const FPS_DEMO_REMOTE_PLAYER_ID: StableEntityId = StableEntityId::from_raw(10_000_002);
#[cfg(feature = "lightyear")]
const OWNED_PLAYER_CORRECTION_THRESHOLD_SQUARED: f32 = 0.25 * 0.25;

pub struct FpsDemoNetworkPlugin;

#[derive(Default)]
struct FpsDemoNetworkRuntime {
    #[cfg(feature = "lightyear")]
    pub(super) local: Option<FpsDemoLocalLightyear>,
    #[cfg(all(feature = "lightyear", not(target_family = "wasm")))]
    native: network_native::FpsDemoNativeLightyear,
}

#[cfg(all(test, feature = "lightyear"))]
impl FpsDemoNetworkRuntime {
    fn local_player_server_state(&self) -> Option<FpsDemoPlayerState> {
        self.local.as_ref()?.local_player_server_state()
    }
}

impl Plugin for FpsDemoNetworkPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "lightyear")]
        if app.is_plugin_added::<crate::network::AfterglowLightyearPlugin>() {
            network_protocol::register_fps_demo_lightyear_protocol(app);
        }
        #[cfg(all(feature = "lightyear", not(target_family = "wasm")))]
        app.add_observer(network_native::configure_native_server_link);

        app.init_resource::<FpsDemoNetworkConfig>()
            .init_resource::<FpsDemoNetworkStatus>()
            .init_resource::<AfterglowLightyearConfig>()
            .init_resource::<ConsoleNetworkState>()
            .init_non_send_resource::<FpsDemoNetworkRuntime>();
        #[cfg(feature = "lightyear")]
        app.init_resource::<FpsDemoPredictionBuffer>();

        app.add_message::<ConsoleNetworkRequest>()
            .add_systems(Startup, start_configured_network)
            .add_systems(
                Update,
                (
                    apply_console_network_requests,
                    sync_scene_player_state_single,
                    sync_native_server_state,
                    push_scene_player_state_to_local_runner,
                    pump_local_lightyear,
                    update_native_lightyear,
                    apply_owned_player_corrections,
                    network_visuals::sync_visible_network_avatars,
                    network_visuals::ensure_remote_avatar_visuals,
                )
                    .chain()
                    .in_set(AfterglowSet::DebugAndMetrics),
            );
    }
}

pub(super) fn fps_demo_player_network_components(
    translation: Vec3,
) -> (StableEntityId, Replicated, FpsDemoPlayerState) {
    (
        FPS_DEMO_PLAYER_ID,
        Replicated,
        FpsDemoPlayerState::from_translation(translation),
    )
}

fn start_configured_network(
    config: Res<FpsDemoNetworkConfig>,
    mut status: ResMut<FpsDemoNetworkStatus>,
    mut console: ResMut<ConsoleNetworkState>,
    mut lightyear_config: ResMut<AfterglowLightyearConfig>,
    mut runtime: NonSendMut<FpsDemoNetworkRuntime>,
) {
    match &config.launch {
        FpsDemoLaunchMode::Local => connect_local(&mut runtime, &mut status, &mut console),
        FpsDemoLaunchMode::Remote(addr) => {
            connect_remote(
                addr,
                &mut runtime,
                &mut status,
                &mut console,
                &mut lightyear_config,
            );
        }
        FpsDemoLaunchMode::Server(addr) => start_native_server(
            addr,
            &mut runtime,
            &mut status,
            &mut console,
            &mut lightyear_config,
        ),
    }
}

fn apply_console_network_requests(
    mut requests: MessageReader<ConsoleNetworkRequest>,
    mut status: ResMut<FpsDemoNetworkStatus>,
    mut console: ResMut<ConsoleNetworkState>,
    mut lightyear_config: ResMut<AfterglowLightyearConfig>,
    mut runtime: NonSendMut<FpsDemoNetworkRuntime>,
) {
    for request in requests.read() {
        match request {
            ConsoleNetworkRequest::ConnectLocal => {
                connect_local(&mut runtime, &mut status, &mut console);
            }
            ConsoleNetworkRequest::ConnectRemote(addr) => {
                connect_remote(
                    addr,
                    &mut runtime,
                    &mut status,
                    &mut console,
                    &mut lightyear_config,
                );
            }
            ConsoleNetworkRequest::Disconnect => {
                disconnect(&mut runtime, &mut status, &mut console)
            }
            ConsoleNetworkRequest::StartLocalServer => {
                start_local_server(&mut runtime, &mut status, &mut console);
            }
            ConsoleNetworkRequest::StopLocalServer => {
                stop_local_server(&mut runtime, &mut status, &mut console);
            }
            ConsoleNetworkRequest::SetLatencyMs(ms) => {
                set_latency(*ms, &mut status, &mut lightyear_config);
            }
        }
    }
}

fn connect_local(
    runtime: &mut FpsDemoNetworkRuntime,
    status: &mut FpsDemoNetworkStatus,
    console: &mut ConsoleNetworkState,
) {
    ensure_local_runner(runtime);
    status.connection = FpsDemoConnectionState::Local;
    status.local_server_running = true;
    console.local_server_running = true;
    console.connection = crate::console::ConsoleConnectionState::ConnectingLocal;
    refresh_local_status(runtime, status);
}

fn connect_remote(
    addr: &str,
    runtime: &mut FpsDemoNetworkRuntime,
    status: &mut FpsDemoNetworkStatus,
    console: &mut ConsoleNetworkState,
    lightyear_config: &mut AfterglowLightyearConfig,
) {
    drop_local_runner(runtime);
    status.connection = FpsDemoConnectionState::Remote(addr.into());
    status.local_server_running = false;
    clear_link_status(status);
    console.local_server_running = false;
    console.connection = crate::console::ConsoleConnectionState::ConnectingRemote(addr.into());
    lightyear_config.role = LightyearRole::Client;
    lightyear_config.remote_addr = addr.into();
}

fn start_native_server(
    addr: &str,
    runtime: &mut FpsDemoNetworkRuntime,
    status: &mut FpsDemoNetworkStatus,
    console: &mut ConsoleNetworkState,
    lightyear_config: &mut AfterglowLightyearConfig,
) {
    drop_local_runner(runtime);
    status.connection = FpsDemoConnectionState::Server(addr.into());
    status.local_server_running = true;
    clear_link_status(status);
    console.local_server_running = true;
    lightyear_config.role = LightyearRole::Server;
    lightyear_config.server_addr = addr.into();
}

fn start_local_server(
    runtime: &mut FpsDemoNetworkRuntime,
    status: &mut FpsDemoNetworkStatus,
    console: &mut ConsoleNetworkState,
) {
    ensure_local_runner(runtime);
    status.local_server_running = true;
    console.local_server_running = true;
    refresh_local_status(runtime, status);
}

fn stop_local_server(
    runtime: &mut FpsDemoNetworkRuntime,
    status: &mut FpsDemoNetworkStatus,
    console: &mut ConsoleNetworkState,
) {
    drop_local_runner(runtime);
    status.local_server_running = false;
    clear_link_status(status);
    console.local_server_running = false;
    if matches!(
        status.connection,
        FpsDemoConnectionState::Local | FpsDemoConnectionState::Server(_)
    ) {
        status.connection = FpsDemoConnectionState::Disconnected;
        console.connection = crate::console::ConsoleConnectionState::Disconnected;
    }
}

fn disconnect(
    runtime: &mut FpsDemoNetworkRuntime,
    status: &mut FpsDemoNetworkStatus,
    console: &mut ConsoleNetworkState,
) {
    drop_local_runner(runtime);
    status.connection = FpsDemoConnectionState::Disconnected;
    status.local_server_running = false;
    clear_link_status(status);
    console.connection = crate::console::ConsoleConnectionState::Disconnected;
    console.local_server_running = false;
}

fn clear_link_status(status: &mut FpsDemoNetworkStatus) {
    status.lightyear_links = false;
    status.replicated_avatar = false;
    status.replicated_avatar_count = 0;
    status.visible_remote_avatar_count = 0;
    status.local_player_round_trip = false;
}

fn set_latency(
    ms: u32,
    status: &mut FpsDemoNetworkStatus,
    lightyear_config: &mut AfterglowLightyearConfig,
) {
    status.latency_ms = ms;
    lightyear_config.link_conditioner = Some(LightyearLinkConditioner {
        incoming_latency_ms: ms,
        incoming_jitter_ms: 0,
        incoming_loss: 0.0,
        outgoing_latency_ms: ms,
        outgoing_jitter_ms: 0,
        outgoing_loss: 0.0,
    });
}

fn pump_local_lightyear(
    mut status: ResMut<FpsDemoNetworkStatus>,
    mut runtime: NonSendMut<FpsDemoNetworkRuntime>,
) {
    #[cfg(feature = "lightyear")]
    if let Some(local) = runtime.local.as_mut() {
        local.pump_once();
        status.ticks = local.ticks;
    }
    refresh_local_status(&mut runtime, &mut status);
}

#[cfg(all(feature = "lightyear", not(target_family = "wasm")))]
fn sync_scene_player_state_single(
    mut players: Query<
        (
            &Transform,
            Option<&FirstPersonMotorState>,
            &mut FpsDemoPlayerState,
        ),
        Without<FpsDemoNativeServerAvatar>,
    >,
) {
    for (transform, motor, mut state) in &mut players {
        let translation = transform.translation;
        let yaw = motor.map_or(0.0, |m| m.yaw);
        let pitch = motor.map_or(0.0, |m| m.pitch);
        let tick = state.authoritative_tick;
        *state = FpsDemoPlayerState::from_translation(translation);
        state.yaw_milliradians = (yaw * 1000.0).round() as i32;
        state.pitch_milliradians = (pitch * 1000.0).round() as i32;
        state.authoritative_tick = tick;
    }
}

#[cfg(all(feature = "lightyear", not(target_family = "wasm")))]
fn sync_native_server_state(
    mut avatars: Query<
        (
            &Transform,
            Option<&FirstPersonMotorState>,
            &mut FpsDemoPlayerState,
            Option<&FpsServerHasProcessedInput>,
        ),
        (With<FpsDemoNativeServerAvatar>, Without<FpsDemoPlayer>),
    >,
) {
    for (transform, motor, mut state, processed) in &mut avatars {
        let translation = transform.translation;
        let yaw = motor.map_or(0.0, |m| m.yaw);
        let pitch = motor.map_or(0.0, |m| m.pitch);
        let tick = if processed.is_some() && state.authoritative_tick == 0 {
            1
        } else {
            state.authoritative_tick
        };
        *state = FpsDemoPlayerState::from_translation(translation);
        state.yaw_milliradians = (yaw * 1000.0).round() as i32;
        state.pitch_milliradians = (pitch * 1000.0).round() as i32;
        state.authoritative_tick = tick;
    }
}

#[cfg(any(not(feature = "lightyear"), target_family = "wasm"))]
fn sync_scene_player_state_single(
    mut players: Query<(&Transform, &mut FpsDemoPlayerState), With<FpsDemoPlayer>>,
) {
    for (transform, mut state) in &mut players {
        *state = FpsDemoPlayerState::from_translation(transform.translation);
    }
}
#[cfg(any(not(feature = "lightyear"), target_family = "wasm"))]
fn sync_native_server_state() {}
#[cfg(any(not(feature = "lightyear"), target_family = "wasm"))]
fn apply_owned_player_corrections() {}

#[cfg(feature = "lightyear")]
fn push_scene_player_state_to_local_runner(
    players: Query<
        (
            &FpsDemoPlayerState,
            Option<
                &leafwing_input_manager::action_state::ActionState<crate::input::AfterglowAction>,
            >,
        ),
        With<FpsDemoPlayer>,
    >,
    mut prediction: ResMut<FpsDemoPredictionBuffer>,
    mut runtime: NonSendMut<FpsDemoNetworkRuntime>,
) {
    if let Some(local) = runtime.local.as_mut()
        && let Ok((_state, action_state)) = players.single()
    {
        let command = fps_demo_input_command(FPS_DEMO_PLAYER_ID, local.ticks, action_state);
        prediction.push(command.clone());
        local.send_player_input(command);
    }
}

#[cfg(not(feature = "lightyear"))]
fn push_scene_player_state_to_local_runner() {}

#[cfg(any(not(feature = "lightyear"), target_family = "wasm"))]
fn update_native_lightyear() {}

#[cfg(feature = "lightyear")]
fn apply_owned_player_corrections(
    mut runtime: NonSendMut<FpsDemoNetworkRuntime>,
    mut prediction: ResMut<FpsDemoPredictionBuffer>,
    time: Res<Time>,
    mut commands: Commands,
    mut players: Query<
        (
            Entity,
            &mut Transform,
            &mut FpsDemoPlayerState,
            Option<&mut PredictionErrorSmoothing>,
        ),
        With<FpsDemoPlayer>,
    >,
    replicated_states: Query<(&StableEntityId, &FpsDemoPlayerState), Without<FpsDemoPlayer>>,
) {
    let Some(authoritative) = owned_authoritative_state(&mut runtime, &replicated_states) else {
        return;
    };
    if authoritative.authoritative_tick == 0 {
        return;
    }
    let Ok((entity, mut transform, _state, smoothing)) = players.single_mut() else {
        return;
    };
    let (predicted_state, predicted_motor) = prediction.replay_from_authoritative(authoritative);
    let predicted_translation = predicted_state.to_translation();
    let error = predicted_translation - transform.translation;
    let error_dist = error.length();
    let predicted_yaw = predicted_motor.yaw;
    if error_dist <= 0.25 {
        if smoothing.is_some() {
            commands.entity(entity).remove::<PredictionErrorSmoothing>();
        }
        return;
    }
    // Snap body to server position (reconciliation).
    transform.translation = predicted_translation;
    transform.rotation = Quat::from_rotation_y(predicted_yaw);
    commands.entity(entity).insert(predicted_state);
    if error_dist > 2.0 {
        // Teleport: no camera smoothing — snap is intentional.
        commands.entity(entity).remove::<PredictionErrorSmoothing>();
    } else {
        // Store the pre-snap position as a decaying camera offset
        // (Source: m_vecPredictionError, cl_smoothtime ~100ms).
        commands.entity(entity).insert(PredictionErrorSmoothing {
            error: -error,
            decay_start: time.elapsed_secs_f64() as f32,
        });
    }
}

#[cfg(feature = "lightyear")]
fn owned_authoritative_state(
    runtime: &mut FpsDemoNetworkRuntime,
    replicated_states: &Query<(&StableEntityId, &FpsDemoPlayerState), Without<FpsDemoPlayer>>,
) -> Option<FpsDemoPlayerState> {
    if let Some(local) = runtime.local.as_mut() {
        return local
            .replicated_avatar_states()
            .into_iter()
            .find_map(|(stable_id, state)| (stable_id == FPS_DEMO_PLAYER_ID).then_some(state));
    }
    #[cfg(not(target_family = "wasm"))]
    if let Some(local_player_id) = runtime.native.local_player_id() {
        return replicated_states
            .iter()
            .find_map(|(stable_id, state)| (*stable_id == local_player_id).then(|| state.clone()));
    }
    None
}

#[cfg(feature = "lightyear")]
fn ensure_local_runner(runtime: &mut FpsDemoNetworkRuntime) {
    if runtime.local.is_none() {
        runtime.local = Some(FpsDemoLocalLightyear::new());
    }
}

#[cfg(not(feature = "lightyear"))]
fn ensure_local_runner(_runtime: &mut FpsDemoNetworkRuntime) {}

#[cfg(feature = "lightyear")]
fn drop_local_runner(runtime: &mut FpsDemoNetworkRuntime) {
    runtime.local = None;
}

#[cfg(not(feature = "lightyear"))]
fn drop_local_runner(_runtime: &mut FpsDemoNetworkRuntime) {}

#[cfg(feature = "lightyear")]
fn refresh_local_status(runtime: &mut FpsDemoNetworkRuntime, status: &mut FpsDemoNetworkStatus) {
    if let Some(local) = runtime.local.as_mut() {
        status.lightyear_links = local.has_lightyear_links();
        status.replicated_avatar = local.client_has_replicated_avatar();
        let replicated = local.replicated_avatar_states();
        status.replicated_avatar_count = replicated.len();
        status.local_player_round_trip = replicated
            .iter()
            .any(|(stable_id, _)| *stable_id == FPS_DEMO_PLAYER_ID);
        status.ticks = local.ticks;
        return;
    }
    status.lightyear_links = false;
    status.replicated_avatar = false;
    status.replicated_avatar_count = 0;
    status.local_player_round_trip = false;
}

#[cfg(not(feature = "lightyear"))]
fn refresh_local_status(_runtime: &mut FpsDemoNetworkRuntime, status: &mut FpsDemoNetworkStatus) {
    status.lightyear_links = false;
    status.replicated_avatar = false;
    status.replicated_avatar_count = 0;
    status.local_player_round_trip = false;
}

#[cfg(feature = "lightyear")]
#[path = "network_lightyear.rs"]
mod lightyear_runner;
#[cfg(feature = "lightyear")]
use lightyear_runner::FpsDemoLocalLightyear;

#[cfg(test)]
#[cfg(all(feature = "lightyear", not(target_family = "wasm")))]
#[path = "network_native_tests.rs"]
mod native_tests;
#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
