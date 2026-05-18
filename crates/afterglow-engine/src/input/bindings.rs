use bevy::prelude::*;
use leafwing_input_manager::{input_map::InputMap, user_input::virtual_axial::VirtualDPad};

use super::AfterglowAction;

pub fn default_gameplay_input_map() -> InputMap<AfterglowAction> {
    let mut map = InputMap::default();
    map.insert_dual_axis(AfterglowAction::Move, VirtualDPad::wasd());
    map.insert_dual_axis(
        AfterglowAction::Move,
        leafwing_input_manager::user_input::gamepad::GamepadStick::LEFT,
    );
    map.insert_dual_axis(
        AfterglowAction::Look,
        leafwing_input_manager::user_input::mouse::MouseMove::default(),
    );
    map.insert_dual_axis(
        AfterglowAction::Look,
        leafwing_input_manager::user_input::gamepad::GamepadStick::RIGHT,
    );
    map.insert(AfterglowAction::Use, KeyCode::KeyE);
    map.insert(AfterglowAction::AttackPrimary, MouseButton::Left);
    map.insert(AfterglowAction::AttackSecondary, MouseButton::Right);
    map.insert(AfterglowAction::RaiseShield, KeyCode::KeyQ);
    map.insert(AfterglowAction::Jump, KeyCode::Space);
    map.insert(AfterglowAction::Crouch, KeyCode::ControlLeft);
    map.insert(AfterglowAction::Sprint, KeyCode::ShiftLeft);
    map.insert(AfterglowAction::Menu, KeyCode::Escape);
    map.insert(AfterglowAction::DebugToggle, KeyCode::F3);
    map
}
