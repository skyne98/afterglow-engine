use afterglow_engine::{
    console::{
        ConsoleConnectionState, ConsoleNetworkRequest, ConsoleNetworkState, ConsoleOutput,
        DevConsolePlugin, drain_console_network_requests, run_console_command,
    },
    core::identity::StableEntityId,
};
use bevy::prelude::*;
use std::collections::BTreeMap;

use super::{lightyear::LightyearNetworkedRpg, model::ClientInput};

pub struct ConsoleNetworkedRpg {
    console_app: App,
    connection: Option<LightyearNetworkedRpg>,
    retention_ticks: u32,
    mock_players: BTreeMap<String, StableEntityId>,
}

impl ConsoleNetworkedRpg {
    pub fn new(retention_ticks: u32) -> Self {
        let mut console_app = App::new();
        console_app.add_plugins((MinimalPlugins, DevConsolePlugin));
        Self {
            console_app,
            connection: None,
            retention_ticks,
            mock_players: BTreeMap::new(),
        }
    }

    pub fn command(&mut self, line: &str) -> ConsoleOutput {
        self.sync_console_stats();
        let output = run_console_command(self.console_app.world_mut(), line);
        for request in drain_console_network_requests(self.console_app.world_mut()) {
            self.handle_network_request(request);
        }
        self.sync_console_stats();
        output
    }

    pub fn register_mock_player(&mut self, name: impl Into<String>, stable_id: StableEntityId) {
        self.mock_players.insert(name.into(), stable_id);
    }

    pub fn mock_player(&self, name: &str) -> Option<StableEntityId> {
        self.mock_players.get(name).copied()
    }

    pub fn mock_player_count(&self) -> usize {
        self.mock_players.len()
    }

    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    pub fn has_lightyear_links(&self) -> bool {
        self.connection
            .as_ref()
            .is_some_and(LightyearNetworkedRpg::has_lightyear_links)
    }

    pub fn send(&mut self, input: ClientInput) {
        let latency_ticks = self.latency_ticks();
        let connection = self
            .connection
            .as_mut()
            .expect("console should connect before sending mock RPG input");
        connection.send(input, latency_ticks);
        self.console_app
            .world_mut()
            .resource_mut::<ConsoleNetworkState>()
            .sent_packets += 1;
    }

    pub fn advance_to(&mut self, tick: u32) {
        if let Some(connection) = &mut self.connection {
            connection.advance_to(tick);
        }
        self.sync_console_stats();
    }

    pub fn rpg(&self) -> Option<&LightyearNetworkedRpg> {
        self.connection.as_ref()
    }

    pub fn rpg_mut(&mut self) -> Option<&mut LightyearNetworkedRpg> {
        self.connection.as_mut()
    }

    fn handle_network_request(&mut self, request: ConsoleNetworkRequest) {
        match request {
            ConsoleNetworkRequest::ConnectLocal => self.connect_local(),
            ConsoleNetworkRequest::Disconnect | ConsoleNetworkRequest::StopLocalServer => {
                self.disconnect();
            }
            ConsoleNetworkRequest::ConnectRemote(_)
            | ConsoleNetworkRequest::StartLocalServer
            | ConsoleNetworkRequest::SetLatencyMs(_) => {}
        }
    }

    fn connect_local(&mut self) {
        if self.connection.is_none() {
            self.connection = Some(LightyearNetworkedRpg::new(self.retention_ticks));
        }
        let mut state = self
            .console_app
            .world_mut()
            .resource_mut::<ConsoleNetworkState>();
        state.local_server_running = true;
        state.connection = ConsoleConnectionState::ConnectingLocal;
    }

    fn disconnect(&mut self) {
        self.connection = None;
        let mut state = self
            .console_app
            .world_mut()
            .resource_mut::<ConsoleNetworkState>();
        state.local_server_running = false;
        state.connection = ConsoleConnectionState::Disconnected;
        state.sent_packets = 0;
        state.received_packets = 0;
    }

    fn sync_console_stats(&mut self) {
        let received = self.connection.as_ref().map_or(0, |connection| {
            connection.received_lightyear_inputs() as u64
        });
        self.console_app
            .world_mut()
            .resource_mut::<ConsoleNetworkState>()
            .received_packets = received;
    }

    fn latency_ticks(&self) -> u32 {
        self.console_app
            .world()
            .resource::<ConsoleNetworkState>()
            .latency_ms
            .saturating_add(15)
            / 16
    }
}
