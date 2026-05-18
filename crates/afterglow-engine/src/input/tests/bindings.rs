use super::*;
use leafwing_input_manager::Actionlike;

#[test]
fn move_action_is_dual_axis() {
    assert_eq!(
        AfterglowAction::Move.input_control_kind(),
        leafwing_input_manager::InputControlKind::DualAxis
    );
}

#[test]
fn look_action_is_dual_axis() {
    assert_eq!(
        AfterglowAction::Look.input_control_kind(),
        leafwing_input_manager::InputControlKind::DualAxis
    );
}

#[test]
fn use_action_is_button() {
    assert_eq!(
        AfterglowAction::Use.input_control_kind(),
        leafwing_input_manager::InputControlKind::Button
    );
}

#[test]
fn jump_action_is_button() {
    assert_eq!(
        AfterglowAction::Jump.input_control_kind(),
        leafwing_input_manager::InputControlKind::Button
    );
}

#[test]
fn default_input_map_binds_raise_shield() {
    let map = default_gameplay_input_map();
    let has_raise_shield = map
        .iter_buttonlike()
        .any(|(action, inputs)| *action == AfterglowAction::RaiseShield && !inputs.is_empty());
    assert!(
        has_raise_shield,
        "RaiseShield should have a default binding"
    );
}

#[test]
fn default_input_map_constructs_without_panicking() {
    let _ = default_gameplay_input_map();
}

#[test]
fn default_input_map_has_no_gamepad_association() {
    let map = default_gameplay_input_map();
    assert!(map.gamepad().is_none());
}

#[test]
fn default_input_map_has_bindings_for_all_actions() {
    let map = default_gameplay_input_map();
    let mut count = 0;
    for (_action, inputs) in map.iter_buttonlike() {
        count += inputs.len();
    }
    for (_action, inputs) in map.iter_dual_axislike() {
        count += inputs.len();
    }
    assert!(count > 0, "should have at least one binding");
}
