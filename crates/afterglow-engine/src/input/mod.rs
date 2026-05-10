use bevy::{input::touch::ForceTouch, prelude::*};

use crate::core::{identity::StableEntityId, schedule::AfterglowSet};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Reflect)]
pub struct InputAction(pub String);

impl InputAction {
    pub fn new(action: impl Into<String>) -> Self {
        Self(action.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct PlayerCommand {
    pub stable_player: StableEntityId,
    pub tick: u32,
    pub axes: Vec<InputAxisValue>,
    pub actions: Vec<InputAction>,
    pub pointers: Vec<PointerInput>,
}

#[derive(Resource, Clone, Debug, Reflect)]
pub struct PlayerInputBindings {
    pub axes: Vec<AxisBinding>,
    pub actions: Vec<ActionBinding>,
}

impl PlayerInputBindings {
    pub fn with_key_axis(
        mut self,
        axis: impl Into<String>,
        negative: KeyCode,
        positive: KeyCode,
    ) -> Self {
        self.axes.push(AxisBinding {
            axis: InputAxis::new(axis),
            source: AxisSource::KeyPair {
                negative,
                positive,
                normalize_pair: false,
            },
        });
        self
    }

    pub fn with_normalized_key_axis(
        mut self,
        axis: impl Into<String>,
        negative: KeyCode,
        positive: KeyCode,
    ) -> Self {
        self.axes.push(AxisBinding {
            axis: InputAxis::new(axis),
            source: AxisSource::KeyPair {
                negative,
                positive,
                normalize_pair: true,
            },
        });
        self
    }

    pub fn with_gamepad_axis(
        mut self,
        axis: impl Into<String>,
        input: GamepadAxis,
        deadzone: f32,
    ) -> Self {
        self.axes.push(AxisBinding {
            axis: InputAxis::new(axis),
            source: AxisSource::GamepadAxis {
                input,
                deadzone: deadzone.max(0.0),
            },
        });
        self
    }

    pub fn with_gamepad_button_axis(
        mut self,
        axis: impl Into<String>,
        negative: GamepadButton,
        positive: GamepadButton,
    ) -> Self {
        self.axes.push(AxisBinding {
            axis: InputAxis::new(axis),
            source: AxisSource::GamepadButtonPair {
                negative,
                positive,
                normalize_pair: false,
            },
        });
        self
    }

    pub fn with_key_action(mut self, input: KeyCode, action: impl Into<String>) -> Self {
        self.actions.push(ActionBinding {
            input: ActionInput::Key(input),
            action: InputAction::new(action),
        });
        self
    }

    pub fn with_mouse_action(mut self, input: MouseButton, action: impl Into<String>) -> Self {
        self.actions.push(ActionBinding {
            input: ActionInput::Mouse(input),
            action: InputAction::new(action),
        });
        self
    }

    pub fn with_gamepad_action(mut self, input: GamepadButton, action: impl Into<String>) -> Self {
        self.actions.push(ActionBinding {
            input: ActionInput::GamepadButton(input),
            action: InputAction::new(action),
        });
        self
    }

    pub fn with_touch_action(mut self, action: impl Into<String>) -> Self {
        self.actions.push(ActionBinding {
            input: ActionInput::TouchAny,
            action: InputAction::new(action),
        });
        self
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Reflect)]
pub struct InputAxis(pub String);

impl InputAxis {
    pub fn new(axis: impl Into<String>) -> Self {
        Self(axis.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct InputAxisValue {
    pub axis: InputAxis,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct AxisBinding {
    pub axis: InputAxis,
    pub source: AxisSource,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub enum AxisSource {
    KeyPair {
        negative: KeyCode,
        positive: KeyCode,
        normalize_pair: bool,
    },
    GamepadAxis {
        input: GamepadAxis,
        deadzone: f32,
    },
    GamepadButtonPair {
        negative: GamepadButton,
        positive: GamepadButton,
        normalize_pair: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Reflect, Resource)]
pub struct VirtualInputState {
    axes: Vec<InputAxisValue>,
    actions: Vec<InputAction>,
    pointers: Vec<PointerInput>,
}

impl VirtualInputState {
    pub fn set_axis(&mut self, axis: impl Into<String>, value: f32) {
        let axis = InputAxis::new(axis);
        if let Some(existing) = self.axes.iter_mut().find(|value| value.axis == axis) {
            existing.value = value;
            return;
        }
        self.axes.push(InputAxisValue { axis, value });
    }

    pub fn press_action(&mut self, action: impl Into<String>) {
        self.actions.push(InputAction::new(action));
    }

    pub fn push_pointer(&mut self, pointer: PointerInput) {
        self.pointers.push(pointer);
    }

    pub fn clear(&mut self) {
        self.axes.clear();
        self.actions.clear();
        self.pointers.clear();
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

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct ActionBinding {
    pub input: ActionInput,
    pub action: InputAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum ActionInput {
    Key(KeyCode),
    Mouse(MouseButton),
    GamepadButton(GamepadButton),
    TouchAny,
}

impl Default for PlayerInputBindings {
    fn default() -> Self {
        Self {
            axes: Vec::new(),
            actions: Vec::new(),
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub struct LocalPlayer {
    pub stable_id: StableEntityId,
}

impl Default for LocalPlayer {
    fn default() -> Self {
        Self {
            stable_id: StableEntityId::from_raw(1),
        }
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

    pub fn drain(&mut self) -> impl Iterator<Item = PlayerCommand> + '_ {
        self.commands.drain(..)
    }
}

pub struct AfterglowInputPlugin;

impl Plugin for AfterglowInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerInputBindings>()
            .init_resource::<VirtualInputState>()
            .init_resource::<LocalPlayer>()
            .init_resource::<SimulationTick>()
            .init_resource::<PlayerCommandQueue>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Touches>()
            .add_systems(
                Update,
                collect_player_commands.in_set(AfterglowSet::ReadInput),
            );
    }
}

pub fn collect_player_commands(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    gamepads: Query<&Gamepad>,
    bindings: Res<PlayerInputBindings>,
    mut virtual_input: ResMut<VirtualInputState>,
    local_player: Res<LocalPlayer>,
    mut tick: ResMut<SimulationTick>,
    mut commands: ResMut<PlayerCommandQueue>,
) {
    let command = read_player_command(
        &keyboard,
        &mouse,
        &touches,
        gamepads.iter(),
        &bindings,
        &virtual_input,
        local_player.stable_id,
        tick.0,
    );
    commands.commands.clear();
    commands.commands.push(command);
    virtual_input.clear();
    tick.0 = tick.0.saturating_add(1);
}

pub fn read_player_command<'a>(
    keyboard: &ButtonInput<KeyCode>,
    mouse: &ButtonInput<MouseButton>,
    touches: &Touches,
    gamepads: impl IntoIterator<Item = &'a Gamepad>,
    bindings: &PlayerInputBindings,
    virtual_input: &VirtualInputState,
    stable_player: StableEntityId,
    tick: u32,
) -> PlayerCommand {
    let gamepads: Vec<&Gamepad> = gamepads.into_iter().collect();
    let mut axes = Vec::new();
    for binding in &bindings.axes {
        if let Some(value) = binding.value(keyboard, &gamepads) {
            axes.push(InputAxisValue {
                axis: binding.axis.clone(),
                value,
            });
        }
    }
    axes.extend(virtual_input.axes.iter().cloned());

    let mut actions = Vec::new();
    for binding in &bindings.actions {
        if binding.just_pressed(keyboard, mouse, touches, &gamepads) {
            actions.push(binding.action.clone());
        }
    }
    actions.extend(virtual_input.actions.iter().cloned());
    let mut pointers = touches
        .iter()
        .map(|touch| PointerInput {
            device: PointerDevice::Touch,
            id: touch.id(),
            position: touch.position(),
            delta: touch.delta(),
            pressure: force_to_pressure(touch.force()),
            tilt: None,
            twist: None,
            primary: false,
        })
        .collect::<Vec<_>>();
    pointers.extend(virtual_input.pointers.iter().cloned());

    PlayerCommand {
        stable_player,
        tick,
        axes,
        actions,
        pointers,
    }
}

impl AxisBinding {
    fn value(&self, keyboard: &ButtonInput<KeyCode>, gamepads: &[&Gamepad]) -> Option<f32> {
        match self.source {
            AxisSource::KeyPair {
                negative,
                positive,
                normalize_pair,
            } => axis_from_pair(
                keyboard.pressed(negative),
                keyboard.pressed(positive),
                normalize_pair,
            ),
            AxisSource::GamepadAxis { input, deadzone } => gamepads
                .iter()
                .find_map(|gamepad| gamepad.get(input))
                .filter(|value| value.abs() > deadzone),
            AxisSource::GamepadButtonPair {
                negative,
                positive,
                normalize_pair,
            } => gamepads.iter().find_map(|gamepad| {
                axis_from_pair(
                    gamepad.pressed(negative),
                    gamepad.pressed(positive),
                    normalize_pair,
                )
            }),
        }
    }
}

impl ActionBinding {
    fn just_pressed(
        &self,
        keyboard: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
        touches: &Touches,
        gamepads: &[&Gamepad],
    ) -> bool {
        match self.input {
            ActionInput::Key(key) => keyboard.just_pressed(key),
            ActionInput::Mouse(button) => mouse.just_pressed(button),
            ActionInput::GamepadButton(button) => {
                gamepads.iter().any(|gamepad| gamepad.just_pressed(button))
            }
            ActionInput::TouchAny => touches.any_just_pressed(),
        }
    }
}

fn axis_from_pair(negative: bool, positive: bool, normalize_pair: bool) -> Option<f32> {
    let mut value: f32 = 0.0;
    if negative {
        value -= 1.0;
    }
    if positive {
        value += 1.0;
    }
    if value == 0.0 {
        None
    } else if normalize_pair {
        Some(value.signum())
    } else {
        Some(value)
    }
}

fn force_to_pressure(force: Option<ForceTouch>) -> Option<f32> {
    match force {
        Some(ForceTouch::Calibrated {
            force,
            max_possible_force,
            ..
        }) if max_possible_force > 0.0 => Some((force / max_possible_force) as f32),
        Some(ForceTouch::Calibrated { force, .. }) => Some(force as f32),
        Some(ForceTouch::Normalized(force)) => Some(force as f32),
        None => None,
    }
}

#[cfg(test)]
mod tests;
