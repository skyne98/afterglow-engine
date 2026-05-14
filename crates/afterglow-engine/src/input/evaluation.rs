use bevy::prelude::*;
use std::cmp::Reverse;

use super::{
    ActionInput, AxisBinding, AxisComponent, AxisSource, InputActionPhase, InputActionValue,
    InputAxisValue, InputDeviceRoute, PlayerInputBindings,
};

pub struct GamepadInput<'a> {
    pub entity: Entity,
    pub gamepad: &'a Gamepad,
}

pub struct RawInputState<'a> {
    pub keyboard: &'a ButtonInput<KeyCode>,
    pub mouse: &'a ButtonInput<MouseButton>,
    pub touches: &'a Touches,
    pub gamepads: &'a [GamepadInput<'a>],
    pub mouse_delta: Vec2,
    pub devices: Option<&'a [InputDeviceRoute]>,
}

pub fn read_bound_inputs(
    bindings: &PlayerInputBindings,
    raw: &RawInputState,
) -> (Vec<InputAxisValue>, Vec<InputActionValue>) {
    let mut axes = Vec::new();
    let mut actions = Vec::new();
    let mut contexts = bindings
        .contexts
        .iter()
        .filter(|context| context.enabled)
        .collect::<Vec<_>>();
    contexts.sort_by_key(|context| Reverse(context.priority));

    for context in contexts {
        let before_axes = axes.len();
        let before_actions = actions.len();
        for binding in &context.axes {
            if let Some(value) = binding.value(raw) {
                push_axis_if_absent(
                    &mut axes,
                    InputAxisValue {
                        axis: binding.axis.clone(),
                        value,
                    },
                );
            }
        }
        for binding in &context.actions {
            actions.extend(binding.values(raw));
        }
        let emitted = axes.len() != before_axes || actions.len() != before_actions;
        if emitted && context.consume {
            break;
        }
    }

    (axes, actions)
}

fn push_axis_if_absent(axes: &mut Vec<InputAxisValue>, value: InputAxisValue) {
    if !axes.iter().any(|existing| existing.axis == value.axis) {
        axes.push(value);
    }
}

impl AxisBinding {
    fn value(&self, raw: &RawInputState) -> Option<f32> {
        match self.source {
            AxisSource::KeyPair {
                negative,
                positive,
                normalize_pair,
            } if device_allowed(raw.devices, InputDeviceRoute::KeyboardMouse) => axis_from_pair(
                raw.keyboard.pressed(negative),
                raw.keyboard.pressed(positive),
                normalize_pair,
            ),
            AxisSource::GamepadAxis {
                input,
                deadzone,
                scale,
            } => matching_gamepads(raw).find_map(|gamepad| {
                gamepad
                    .get(input)
                    .filter(|value| value.abs() > deadzone)
                    .map(|value| value * scale)
            }),
            AxisSource::GamepadButtonPair {
                negative,
                positive,
                normalize_pair,
            } => matching_gamepads(raw).find_map(|gamepad| {
                axis_from_pair(
                    gamepad.pressed(negative),
                    gamepad.pressed(positive),
                    normalize_pair,
                )
            }),
            AxisSource::MouseMotion { component, scale }
                if device_allowed(raw.devices, InputDeviceRoute::KeyboardMouse) =>
            {
                let value = match component {
                    AxisComponent::X => raw.mouse_delta.x,
                    AxisComponent::Y => raw.mouse_delta.y,
                } * scale;
                (value != 0.0).then_some(value)
            }
            _ => None,
        }
    }
}

impl super::ActionBinding {
    fn values(&self, raw: &RawInputState) -> Vec<InputActionValue> {
        let phases = match &self.input {
            ActionInput::Key(key)
                if device_allowed(raw.devices, InputDeviceRoute::KeyboardMouse) =>
            {
                button_phases(raw.keyboard, *key)
            }
            ActionInput::KeyChord { input, modifiers }
                if device_allowed(raw.devices, InputDeviceRoute::KeyboardMouse)
                    && modifiers
                        .iter()
                        .all(|modifier| raw.keyboard.pressed(*modifier)) =>
            {
                button_phases(raw.keyboard, *input)
            }
            ActionInput::Mouse(button)
                if device_allowed(raw.devices, InputDeviceRoute::KeyboardMouse) =>
            {
                button_phases(raw.mouse, *button)
            }
            ActionInput::GamepadButton(button) => gamepad_button_phases(raw, *button),
            ActionInput::TouchAny if device_allowed(raw.devices, InputDeviceRoute::Touch) => {
                touch_phases(raw.touches)
            }
            _ => Vec::new(),
        };
        phases
            .into_iter()
            .map(|phase| InputActionValue {
                action: self.action.clone(),
                phase,
            })
            .collect()
    }
}

fn button_phases<T: Copy + Eq + std::hash::Hash + Send + Sync + 'static>(
    input: &ButtonInput<T>,
    button: T,
) -> Vec<InputActionPhase> {
    let mut phases = Vec::with_capacity(2);
    if input.just_pressed(button) {
        phases.push(InputActionPhase::Pressed);
    }
    if input.just_released(button) {
        phases.push(InputActionPhase::Released);
    }
    if phases.is_empty() && input.pressed(button) {
        phases.push(InputActionPhase::Held);
    }
    phases
}

fn gamepad_button_phases(raw: &RawInputState, button: GamepadButton) -> Vec<InputActionPhase> {
    let mut pressed = false;
    let mut released = false;
    let mut held = false;
    for gamepad in matching_gamepads(raw) {
        pressed |= gamepad.just_pressed(button);
        released |= gamepad.just_released(button);
        held |= gamepad.pressed(button);
    }
    let mut phases = Vec::with_capacity(2);
    if pressed {
        phases.push(InputActionPhase::Pressed);
    }
    if released {
        phases.push(InputActionPhase::Released);
    }
    if phases.is_empty() && held {
        phases.push(InputActionPhase::Held);
    }
    phases
}

fn touch_phases(touches: &Touches) -> Vec<InputActionPhase> {
    let mut phases = Vec::with_capacity(2);
    if touches.any_just_pressed() {
        phases.push(InputActionPhase::Pressed);
    }
    if touches.any_just_released() {
        phases.push(InputActionPhase::Released);
    }
    if phases.is_empty() && touches.iter().next().is_some() {
        phases.push(InputActionPhase::Held);
    }
    phases
}

fn matching_gamepads<'a>(raw: &'a RawInputState) -> impl Iterator<Item = &'a Gamepad> {
    raw.gamepads.iter().filter_map(|input| {
        let route = InputDeviceRoute::Gamepad(input.entity);
        device_allowed(raw.devices, route).then_some(input.gamepad)
    })
}

pub fn device_allowed(devices: Option<&[InputDeviceRoute]>, device: InputDeviceRoute) -> bool {
    devices.is_none_or(|devices| devices.contains(&device))
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
