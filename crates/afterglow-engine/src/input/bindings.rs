use bevy::prelude::*;

use super::{InputAction, InputAxis};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Reflect)]
pub struct InputContextId(pub String);

impl InputContextId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

#[derive(Resource, Clone, Debug, Default, Reflect)]
pub struct PlayerInputBindings {
    pub contexts: Vec<InputContext>,
}

impl PlayerInputBindings {
    pub fn with_context(mut self, context: InputContext) -> Self {
        self.insert_context(context);
        self
    }

    pub fn insert_context(&mut self, context: InputContext) {
        if let Some(existing) = self
            .contexts
            .iter_mut()
            .find(|existing| existing.id == context.id)
        {
            *existing = context;
            return;
        }
        self.contexts.push(context);
    }

    pub fn context_mut(&mut self, id: impl Into<String>) -> &mut InputContext {
        let id = InputContextId::new(id);
        if let Some(index) = self.contexts.iter().position(|context| context.id == id) {
            return &mut self.contexts[index];
        }
        self.contexts.push(InputContext::new(id.0));
        self.contexts.last_mut().unwrap()
    }

    pub fn set_context_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(context) = self.contexts.iter_mut().find(|context| context.id.0 == id) {
            context.enabled = enabled;
        }
    }

    pub fn with_key_axis(
        mut self,
        axis: impl Into<String>,
        negative: KeyCode,
        positive: KeyCode,
    ) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_key_axis(axis, negative, positive);
        self
    }

    pub fn with_normalized_key_axis(
        mut self,
        axis: impl Into<String>,
        negative: KeyCode,
        positive: KeyCode,
    ) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_normalized_key_axis(axis, negative, positive);
        self
    }

    pub fn with_gamepad_axis(
        mut self,
        axis: impl Into<String>,
        input: GamepadAxis,
        deadzone: f32,
    ) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_gamepad_axis(axis, input, deadzone);
        self
    }

    pub fn with_gamepad_button_axis(
        mut self,
        axis: impl Into<String>,
        negative: GamepadButton,
        positive: GamepadButton,
    ) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_gamepad_button_axis(axis, negative, positive);
        self
    }

    pub fn with_mouse_motion_axis(
        mut self,
        axis: impl Into<String>,
        component: AxisComponent,
        scale: f32,
    ) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_mouse_motion_axis(axis, component, scale);
        self
    }

    pub fn with_key_action(mut self, input: KeyCode, action: impl Into<String>) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_key_action(input, action);
        self
    }

    pub fn with_key_chord_action(
        mut self,
        input: KeyCode,
        modifiers: impl IntoIterator<Item = KeyCode>,
        action: impl Into<String>,
    ) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_key_chord_action(input, modifiers, action);
        self
    }

    pub fn with_mouse_action(mut self, input: MouseButton, action: impl Into<String>) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_mouse_action(input, action);
        self
    }

    pub fn with_gamepad_action(mut self, input: GamepadButton, action: impl Into<String>) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_gamepad_action(input, action);
        self
    }

    pub fn with_touch_action(mut self, action: impl Into<String>) -> Self {
        self.context_mut(InputContext::DEFAULT_GAMEPLAY)
            .add_touch_action(action);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct InputContext {
    pub id: InputContextId,
    pub priority: i32,
    pub enabled: bool,
    pub consume: bool,
    pub axes: Vec<AxisBinding>,
    pub actions: Vec<ActionBinding>,
}

impl InputContext {
    pub const DEFAULT_GAMEPLAY: &'static str = "gameplay";

    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: InputContextId::new(id),
            priority: 0,
            enabled: true,
            consume: false,
            axes: Vec::new(),
            actions: Vec::new(),
        }
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn consuming(mut self) -> Self {
        self.consume = true;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn add_key_axis(
        &mut self,
        axis: impl Into<String>,
        negative: KeyCode,
        positive: KeyCode,
    ) -> &mut Self {
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

    pub fn add_normalized_key_axis(
        &mut self,
        axis: impl Into<String>,
        negative: KeyCode,
        positive: KeyCode,
    ) -> &mut Self {
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

    pub fn add_gamepad_axis(
        &mut self,
        axis: impl Into<String>,
        input: GamepadAxis,
        deadzone: f32,
    ) -> &mut Self {
        self.axes.push(AxisBinding {
            axis: InputAxis::new(axis),
            source: AxisSource::GamepadAxis {
                input,
                deadzone: deadzone.max(0.0),
                scale: 1.0,
            },
        });
        self
    }

    pub fn add_gamepad_button_axis(
        &mut self,
        axis: impl Into<String>,
        negative: GamepadButton,
        positive: GamepadButton,
    ) -> &mut Self {
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

    pub fn add_mouse_motion_axis(
        &mut self,
        axis: impl Into<String>,
        component: AxisComponent,
        scale: f32,
    ) -> &mut Self {
        self.axes.push(AxisBinding {
            axis: InputAxis::new(axis),
            source: AxisSource::MouseMotion { component, scale },
        });
        self
    }

    pub fn add_key_action(&mut self, input: KeyCode, action: impl Into<String>) -> &mut Self {
        self.actions.push(ActionBinding {
            input: ActionInput::Key(input),
            action: InputAction::new(action),
        });
        self
    }

    pub fn add_key_chord_action(
        &mut self,
        input: KeyCode,
        modifiers: impl IntoIterator<Item = KeyCode>,
        action: impl Into<String>,
    ) -> &mut Self {
        self.actions.push(ActionBinding {
            input: ActionInput::KeyChord {
                input,
                modifiers: modifiers.into_iter().collect(),
            },
            action: InputAction::new(action),
        });
        self
    }

    pub fn add_mouse_action(&mut self, input: MouseButton, action: impl Into<String>) -> &mut Self {
        self.actions.push(ActionBinding {
            input: ActionInput::Mouse(input),
            action: InputAction::new(action),
        });
        self
    }

    pub fn add_gamepad_action(
        &mut self,
        input: GamepadButton,
        action: impl Into<String>,
    ) -> &mut Self {
        self.actions.push(ActionBinding {
            input: ActionInput::GamepadButton(input),
            action: InputAction::new(action),
        });
        self
    }

    pub fn add_touch_action(&mut self, action: impl Into<String>) -> &mut Self {
        self.actions.push(ActionBinding {
            input: ActionInput::TouchAny,
            action: InputAction::new(action),
        });
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum AxisComponent {
    X,
    Y,
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
        scale: f32,
    },
    GamepadButtonPair {
        negative: GamepadButton,
        positive: GamepadButton,
        normalize_pair: bool,
    },
    MouseMotion {
        component: AxisComponent,
        scale: f32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct ActionBinding {
    pub input: ActionInput,
    pub action: InputAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub enum ActionInput {
    Key(KeyCode),
    KeyChord {
        input: KeyCode,
        modifiers: Vec<KeyCode>,
    },
    Mouse(MouseButton),
    GamepadButton(GamepadButton),
    TouchAny,
}
