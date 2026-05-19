use std::collections::{BTreeMap, VecDeque};

use bevy::prelude::*;

use super::{
    commands::{ConsoleCommand, CvarCommand, NetCommand, ServerCommand, StatsCommand},
    parse_console_command,
};

#[derive(Resource, Debug)]
pub struct DevConsoleState {
    pub open: bool,
    pub input: String,
    pub history: Vec<String>,
    pub history_cursor: Option<usize>,
    pub completion_index: usize,
    pub scrollback: Vec<ConsoleOutput>,
    pub max_history: usize,
    pub max_scrollback: usize,
}

impl Default for DevConsoleState {
    fn default() -> Self {
        Self {
            open: false,
            input: String::new(),
            history: Vec::new(),
            history_cursor: None,
            completion_index: 0,
            scrollback: Vec::new(),
            max_history: 128,
            max_scrollback: 512,
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct ConsoleCommandQueue {
    lines: VecDeque<String>,
}

impl ConsoleCommandQueue {
    pub fn push(&mut self, line: impl Into<String>) {
        self.lines.push_back(line.into());
    }
}

#[derive(Message, Clone, Debug, Eq, PartialEq)]
pub enum ConsoleNetworkRequest {
    ConnectLocal,
    ConnectRemote(String),
    Disconnect,
    StartLocalServer,
    StopLocalServer,
    SetLatencyMs(u32),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ConsoleConnectionState {
    #[default]
    Disconnected,
    ConnectingLocal,
    ConnectingRemote(String),
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq)]
pub struct ConsoleNetworkState {
    pub connection: ConsoleConnectionState,
    pub local_server_running: bool,
    pub latency_ms: u32,
    pub sent_packets: u64,
    pub received_packets: u64,
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ConsoleCvars {
    values: BTreeMap<String, ConsoleCvar>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleCvar {
    pub value: ConsoleCvarValue,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConsoleCvarValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleOutput {
    pub success: bool,
    pub lines: Vec<String>,
}

impl ConsoleOutput {
    pub fn ok(line: impl Into<String>) -> Self {
        Self {
            success: true,
            lines: vec![line.into()],
        }
    }

    pub fn ok_lines(lines: impl IntoIterator<Item = String>) -> Self {
        Self {
            success: true,
            lines: lines.into_iter().collect(),
        }
    }

    pub fn error(line: impl Into<String>) -> Self {
        Self {
            success: false,
            lines: vec![line.into()],
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

impl Default for ConsoleCvars {
    fn default() -> Self {
        let mut values = BTreeMap::new();
        insert_cvar(
            &mut values,
            "net.tick_rate",
            ConsoleCvarValue::I64(60),
            "network ticks per second",
        );
        insert_cvar(
            &mut values,
            "net.prediction_window",
            ConsoleCvarValue::I64(12),
            "client prediction window in ticks",
        );
        insert_cvar(
            &mut values,
            "console.ui.enabled",
            ConsoleCvarValue::Bool(true),
            "enable the in-game console overlay",
        );
        insert_cvar(
            &mut values,
            "console.autocomplete.max_results",
            ConsoleCvarValue::I64(16),
            "maximum displayed autocomplete candidates",
        );
        Self { values }
    }
}

impl ConsoleCvars {
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    pub fn get(&self, name: &str) -> Option<&ConsoleCvar> {
        self.values.get(name)
    }

    fn set(&mut self, name: &str, value: &str) -> Result<(), String> {
        let cvar = self
            .values
            .get_mut(name)
            .ok_or_else(|| format!("unknown cvar '{name}'"))?;
        cvar.value = cvar.value.parse_same_type(value)?;
        Ok(())
    }
}

impl ConsoleCvarValue {
    pub fn as_text(&self) -> String {
        match self {
            Self::Bool(value) => value.to_string(),
            Self::I64(value) => value.to_string(),
            Self::F64(value) => value.to_string(),
            Self::Text(value) => value.clone(),
        }
    }

    fn parse_same_type(&self, value: &str) -> Result<Self, String> {
        match self {
            Self::Bool(_) => value
                .parse::<bool>()
                .map(Self::Bool)
                .map_err(|_| format!("expected boolean value, got '{value}'")),
            Self::I64(_) => value
                .parse::<i64>()
                .map(Self::I64)
                .map_err(|_| format!("expected integer value, got '{value}'")),
            Self::F64(_) => value
                .parse::<f64>()
                .map(Self::F64)
                .map_err(|_| format!("expected float value, got '{value}'")),
            Self::Text(_) => Ok(Self::Text(value.into())),
        }
    }
}

pub fn run_console_command(world: &mut World, line: &str) -> ConsoleOutput {
    ensure_console_resources(world);
    let output = match parse_console_command(line) {
        Ok(command) => execute_command(world, command),
        Err(error) => ConsoleOutput::error(error.message),
    };
    record_console_output(world, line, output.clone());
    output
}

pub(super) fn execute_queued_console_commands(world: &mut World) {
    ensure_console_resources(world);
    let lines = world
        .resource_mut::<ConsoleCommandQueue>()
        .lines
        .drain(..)
        .collect::<Vec<_>>();
    for line in lines {
        run_console_command(world, &line);
    }
}

pub fn drain_console_network_requests(world: &mut World) -> Vec<ConsoleNetworkRequest> {
    world
        .get_resource_mut::<Messages<ConsoleNetworkRequest>>()
        .map(|mut messages| messages.drain().collect())
        .unwrap_or_default()
}

fn execute_command(world: &mut World, command: ConsoleCommand) -> ConsoleOutput {
    match command {
        ConsoleCommand::Connect(args) => connect(world, &args.target),
        ConsoleCommand::Disconnect => disconnect(world),
        ConsoleCommand::Server { command } => server(world, command),
        ConsoleCommand::Net { command } => net(world, command),
        ConsoleCommand::Stats { command } => stats(command),
        ConsoleCommand::Cvar { command } => cvar(world, command),
        ConsoleCommand::Help(args) => help(args.topic.as_deref()),
    }
}

fn connect(world: &mut World, target: &str) -> ConsoleOutput {
    if target == "local" {
        let mut state = world.resource_mut::<ConsoleNetworkState>();
        state.local_server_running = true;
        state.connection = ConsoleConnectionState::ConnectingLocal;
        emit_network_request(world, ConsoleNetworkRequest::StartLocalServer);
        emit_network_request(world, ConsoleNetworkRequest::ConnectLocal);
        return ConsoleOutput::ok("connect requested: local");
    }
    let mut state = world.resource_mut::<ConsoleNetworkState>();
    state.connection = ConsoleConnectionState::ConnectingRemote(target.into());
    emit_network_request(world, ConsoleNetworkRequest::ConnectRemote(target.into()));
    ConsoleOutput::ok(format!("connect requested: {target}"))
}

fn disconnect(world: &mut World) -> ConsoleOutput {
    world.resource_mut::<ConsoleNetworkState>().connection = ConsoleConnectionState::Disconnected;
    emit_network_request(world, ConsoleNetworkRequest::Disconnect);
    ConsoleOutput::ok("disconnect requested")
}

fn server(world: &mut World, command: ServerCommand) -> ConsoleOutput {
    match command {
        ServerCommand::Start => {
            world
                .resource_mut::<ConsoleNetworkState>()
                .local_server_running = true;
            emit_network_request(world, ConsoleNetworkRequest::StartLocalServer);
            ConsoleOutput::ok("local server start requested")
        }
        ServerCommand::Stop => {
            world
                .resource_mut::<ConsoleNetworkState>()
                .local_server_running = false;
            emit_network_request(world, ConsoleNetworkRequest::StopLocalServer);
            ConsoleOutput::ok("local server stop requested")
        }
        ServerCommand::Status => {
            let running = world.resource::<ConsoleNetworkState>().local_server_running;
            ConsoleOutput::ok(format!("local_server={running}"))
        }
    }
}

fn net(world: &mut World, command: NetCommand) -> ConsoleOutput {
    let state = world.resource::<ConsoleNetworkState>().clone();
    match command {
        NetCommand::Status => ConsoleOutput::ok_lines([
            format!("connection={}", connection_label(&state.connection)),
            format!("local_server={}", state.local_server_running),
        ]),
        NetCommand::Stats => ConsoleOutput::ok_lines([
            format!("sent_packets={}", state.sent_packets),
            format!("received_packets={}", state.received_packets),
            format!("latency_ms={}", state.latency_ms),
        ]),
        NetCommand::Links => ConsoleOutput::ok(connection_label(&state.connection)),
        NetCommand::Latency(args) => {
            world.resource_mut::<ConsoleNetworkState>().latency_ms = args.ms;
            emit_network_request(world, ConsoleNetworkRequest::SetLatencyMs(args.ms));
            ConsoleOutput::ok(format!("latency_ms={}", args.ms))
        }
    }
}

fn stats(command: StatsCommand) -> ConsoleOutput {
    match command {
        StatsCommand::Fps => ConsoleOutput::ok("fps=unavailable"),
        StatsCommand::Systems => ConsoleOutput::ok("systems=unavailable"),
    }
}

fn cvar(world: &mut World, command: CvarCommand) -> ConsoleOutput {
    match command {
        CvarCommand::Get { name } => {
            let cvars = world.resource::<ConsoleCvars>();
            match cvars.get(&name) {
                Some(cvar) => ConsoleOutput::ok(format!("{name}={}", cvar.value.as_text())),
                None => ConsoleOutput::error(format!("unknown cvar '{name}'")),
            }
        }
        CvarCommand::Set { name, value } => {
            match world.resource_mut::<ConsoleCvars>().set(&name, &value) {
                Ok(()) => ConsoleOutput::ok(format!("{name}={value}")),
                Err(error) => ConsoleOutput::error(error),
            }
        }
    }
}

fn help(topic: Option<&str>) -> ConsoleOutput {
    let lines = match topic {
        Some("net") => vec!["net status|stats|links|latency --ms <value>".into()],
        Some("server") => vec!["server start|stop|status".into()],
        Some("cvar") => vec!["cvar get <name>", "cvar set <name> <value>"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        Some(topic) => vec![format!("no help for '{topic}'")],
        None => vec!["connect, disconnect, server, net, stats, cvar, help".into()],
    };
    ConsoleOutput::ok_lines(lines)
}

fn record_console_output(world: &mut World, line: &str, output: ConsoleOutput) {
    let mut state = world.resource_mut::<DevConsoleState>();
    state.history.push(line.into());
    if state.history.len() > state.max_history {
        state.history.remove(0);
    }
    state.scrollback.push(output);
    if state.scrollback.len() > state.max_scrollback {
        state.scrollback.remove(0);
    }
}

fn emit_network_request(world: &mut World, request: ConsoleNetworkRequest) {
    if let Some(mut messages) = world.get_resource_mut::<Messages<ConsoleNetworkRequest>>() {
        messages.write(request);
    }
}

fn ensure_console_resources(world: &mut World) {
    world.get_resource_or_insert_with(DevConsoleState::default);
    world.get_resource_or_insert_with(ConsoleCommandQueue::default);
    world.get_resource_or_insert_with(ConsoleNetworkState::default);
    world.get_resource_or_insert_with(ConsoleCvars::default);
    world.get_resource_or_insert_with(Messages::<ConsoleNetworkRequest>::default);
}

fn insert_cvar(
    values: &mut BTreeMap<String, ConsoleCvar>,
    name: &str,
    value: ConsoleCvarValue,
    description: &str,
) {
    values.insert(
        name.into(),
        ConsoleCvar {
            value,
            description: description.into(),
        },
    );
}

fn connection_label(connection: &ConsoleConnectionState) -> String {
    match connection {
        ConsoleConnectionState::Disconnected => "disconnected".into(),
        ConsoleConnectionState::ConnectingLocal => "connecting-local".into(),
        ConsoleConnectionState::ConnectingRemote(addr) => format!("connecting-remote:{addr}"),
    }
}
