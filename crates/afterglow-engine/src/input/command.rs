use bevy::prelude::*;

use crate::network::{NetworkPlayerId, PeerId};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Reflect)]
pub struct InputAction(pub String);

impl InputAction {
    pub fn new(action: impl Into<String>) -> Self {
        Self(action.into())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
pub enum InputActionPhase {
    Pressed,
    Held,
    Released,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Reflect)]
pub struct InputActionValue {
    pub action: InputAction,
    pub phase: InputActionPhase,
}

impl InputActionValue {
    pub fn pressed(action: impl Into<String>) -> Self {
        Self::new(action, InputActionPhase::Pressed)
    }

    pub fn held(action: impl Into<String>) -> Self {
        Self::new(action, InputActionPhase::Held)
    }

    pub fn released(action: impl Into<String>) -> Self {
        Self::new(action, InputActionPhase::Released)
    }

    pub fn new(action: impl Into<String>, phase: InputActionPhase) -> Self {
        Self {
            action: InputAction::new(action),
            phase,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Reflect)]
pub struct InputAxis(pub String);

impl InputAxis {
    pub fn new(axis: impl Into<String>) -> Self {
        Self(axis.into())
    }
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct InputAxisValue {
    pub axis: InputAxis,
    pub value: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct PlayerCommand {
    pub player: NetworkPlayerId,
    pub tick: u32,
    pub axes: Vec<InputAxisValue>,
    pub actions: Vec<InputActionValue>,
    pub pointers: Vec<PointerInput>,
}

impl PlayerCommand {
    pub fn action_pressed(&self, action: &str) -> bool {
        self.actions
            .iter()
            .any(|value| value.action.0 == action && value.phase == InputActionPhase::Pressed)
    }

    pub fn action_held(&self, action: &str) -> bool {
        self.actions
            .iter()
            .any(|value| value.action.0 == action && value.phase == InputActionPhase::Held)
    }

    pub fn action_released(&self, action: &str) -> bool {
        self.actions
            .iter()
            .any(|value| value.action.0 == action && value.phase == InputActionPhase::Released)
    }

    pub fn axis(&self, axis: &str) -> f32 {
        self.axes
            .iter()
            .rev()
            .find(|value| value.axis.0 == axis)
            .map_or(0.0, |value| value.value)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct VirtualInputBuffer {
    axes: Vec<InputAxisValue>,
    actions: Vec<InputActionValue>,
    pointers: Vec<PointerInput>,
}

impl VirtualInputBuffer {
    pub fn set_axis(&mut self, axis: impl Into<String>, value: f32) {
        let axis = InputAxis::new(axis);
        if let Some(existing) = self.axes.iter_mut().find(|value| value.axis == axis) {
            existing.value = value;
            return;
        }
        self.axes.push(InputAxisValue { axis, value });
    }

    pub fn press_action(&mut self, action: impl Into<String>) {
        self.push_action(action, InputActionPhase::Pressed);
    }

    pub fn hold_action(&mut self, action: impl Into<String>) {
        self.push_action(action, InputActionPhase::Held);
    }

    pub fn release_action(&mut self, action: impl Into<String>) {
        self.push_action(action, InputActionPhase::Released);
    }

    pub fn push_action(&mut self, action: impl Into<String>, phase: InputActionPhase) {
        self.actions.push(InputActionValue::new(action, phase));
    }

    pub fn push_pointer(&mut self, pointer: PointerInput) {
        self.pointers.push(pointer);
    }

    pub fn axes(&self) -> &[InputAxisValue] {
        &self.axes
    }

    pub fn actions(&self) -> &[InputActionValue] {
        &self.actions
    }

    pub fn pointers(&self) -> &[PointerInput] {
        &self.pointers
    }

    pub fn clear(&mut self) {
        self.axes.clear();
        self.actions.clear();
        self.pointers.clear();
    }
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct PlayerVirtualInput {
    pub player: NetworkPlayerId,
    input: VirtualInputBuffer,
}

#[derive(Clone, Debug, Default, PartialEq, Reflect, Resource)]
pub struct VirtualInputState {
    shared: VirtualInputBuffer,
    players: Vec<PlayerVirtualInput>,
}

impl VirtualInputState {
    pub fn set_axis(&mut self, axis: impl Into<String>, value: f32) {
        self.shared.set_axis(axis, value);
    }

    pub fn press_action(&mut self, action: impl Into<String>) {
        self.shared.press_action(action);
    }

    pub fn hold_action(&mut self, action: impl Into<String>) {
        self.shared.hold_action(action);
    }

    pub fn release_action(&mut self, action: impl Into<String>) {
        self.shared.release_action(action);
    }

    pub fn push_action(&mut self, action: impl Into<String>, phase: InputActionPhase) {
        self.shared.push_action(action, phase);
    }

    pub fn push_pointer(&mut self, pointer: PointerInput) {
        self.shared.push_pointer(pointer);
    }

    pub fn set_player_axis(
        &mut self,
        player: NetworkPlayerId,
        axis: impl Into<String>,
        value: f32,
    ) {
        self.player_mut(player).set_axis(axis, value);
    }

    pub fn press_player_action(&mut self, player: NetworkPlayerId, action: impl Into<String>) {
        self.player_mut(player).press_action(action);
    }

    pub fn hold_player_action(&mut self, player: NetworkPlayerId, action: impl Into<String>) {
        self.player_mut(player).hold_action(action);
    }

    pub fn release_player_action(&mut self, player: NetworkPlayerId, action: impl Into<String>) {
        self.player_mut(player).release_action(action);
    }

    pub fn push_player_action(
        &mut self,
        player: NetworkPlayerId,
        action: impl Into<String>,
        phase: InputActionPhase,
    ) {
        self.player_mut(player).push_action(action, phase);
    }

    pub fn push_player_pointer(&mut self, player: NetworkPlayerId, pointer: PointerInput) {
        self.player_mut(player).push_pointer(pointer);
    }

    pub fn shared(&self) -> &VirtualInputBuffer {
        &self.shared
    }

    pub fn player(&self, player: NetworkPlayerId) -> Option<&VirtualInputBuffer> {
        self.players
            .iter()
            .find(|input| input.player == player)
            .map(|input| &input.input)
    }

    pub fn player_mut(&mut self, player: NetworkPlayerId) -> &mut VirtualInputBuffer {
        if let Some(index) = self.players.iter().position(|input| input.player == player) {
            return &mut self.players[index].input;
        }
        self.players.push(PlayerVirtualInput {
            player,
            input: VirtualInputBuffer::default(),
        });
        &mut self.players.last_mut().unwrap().input
    }

    pub fn clear_player(&mut self, player: NetworkPlayerId) {
        self.players.retain(|input| input.player != player);
    }

    pub fn clear(&mut self) {
        self.shared.clear();
        self.players.clear();
    }
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct PointerInput {
    pub device: PointerDevice,
    pub id: u64,
    pub position: Vec2,
    pub delta: Vec2,
    pub pressure: Option<f32>,
    pub tilt: Option<Vec2>,
    pub twist: Option<f32>,
    pub primary: bool,
}

impl PointerInput {
    pub fn pen(id: u64, position: Vec2) -> Self {
        Self {
            device: PointerDevice::Pen,
            id,
            position,
            delta: Vec2::ZERO,
            pressure: None,
            tilt: None,
            twist: None,
            primary: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum PointerDevice {
    Mouse,
    Touch,
    Pen,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
pub enum InputDeviceRoute {
    KeyboardMouse,
    Gamepad(Entity),
    Touch,
    Virtual,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct LocalInputRoute {
    pub player: NetworkPlayerId,
    pub devices: Vec<InputDeviceRoute>,
}

#[derive(Resource, Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct LocalInputRoutes {
    routes: Vec<LocalInputRoute>,
}

impl LocalInputRoutes {
    pub fn set_player_devices(
        &mut self,
        player: NetworkPlayerId,
        devices: impl IntoIterator<Item = InputDeviceRoute>,
    ) {
        let devices = devices.into_iter().collect::<Vec<_>>();
        if let Some(route) = self.routes.iter_mut().find(|route| route.player == player) {
            route.devices = devices;
            return;
        }
        self.routes.push(LocalInputRoute { player, devices });
    }

    pub fn clear_player(&mut self, player: NetworkPlayerId) {
        self.routes.retain(|route| route.player != player);
    }

    pub fn devices_for(&self, player: NetworkPlayerId) -> Option<&[InputDeviceRoute]> {
        self.routes
            .iter()
            .find(|route| route.player == player)
            .map(|route| route.devices.as_slice())
    }
}

#[derive(Resource, Clone, Debug, Eq, PartialEq, Reflect)]
pub struct LocalPlayers {
    pub peer: Option<PeerId>,
    players: Vec<NetworkPlayerId>,
}

impl LocalPlayers {
    pub fn single(player: NetworkPlayerId) -> Self {
        Self {
            peer: None,
            players: vec![player],
        }
    }

    pub fn players(&self) -> &[NetworkPlayerId] {
        &self.players
    }

    pub fn add_player(&mut self, player: NetworkPlayerId) {
        if !self.players.contains(&player) {
            self.players.push(player);
        }
    }

    pub fn remove_player(&mut self, player: NetworkPlayerId) {
        self.players.retain(|existing| *existing != player);
    }
}

impl Default for LocalPlayers {
    fn default() -> Self {
        Self::single(NetworkPlayerId(1))
    }
}

#[derive(Resource, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub struct SimulationTick(pub u32);

#[derive(Resource, Clone, Debug, Default, PartialEq, Reflect)]
pub struct PlayerCommandQueue {
    commands: Vec<PlayerCommand>,
}

impl PlayerCommandQueue {
    pub fn commands(&self) -> &[PlayerCommand] {
        &self.commands
    }

    pub(crate) fn replace(&mut self, commands: Vec<PlayerCommand>) {
        self.commands = commands;
    }

    pub fn drain(&mut self) -> impl Iterator<Item = PlayerCommand> + '_ {
        self.commands.drain(..)
    }
}
