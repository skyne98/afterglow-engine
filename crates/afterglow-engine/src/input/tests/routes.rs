use super::*;

#[test]
fn inactive_allowed_gamepad_axis_does_not_mask_later_active_gamepad() {
    let keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings =
        PlayerInputBindings::default().with_gamepad_axis("move.x", GamepadAxis::LeftStickX, 0.1);
    let inactive_entity = Entity::from_raw_u32(8).unwrap();
    let active_entity = Entity::from_raw_u32(9).unwrap();
    let mut inactive_gamepad = Gamepad::default();
    inactive_gamepad
        .analog_mut()
        .set(GamepadAxis::LeftStickX, 0.0);
    let mut active_gamepad = Gamepad::default();
    active_gamepad
        .analog_mut()
        .set(GamepadAxis::LeftStickX, 0.75);
    let gamepads = [
        GamepadInput {
            entity: inactive_entity,
            gamepad: &inactive_gamepad,
        },
        GamepadInput {
            entity: active_entity,
            gamepad: &active_gamepad,
        },
    ];

    let command = command_with(
        &keyboard,
        &mouse,
        &gamepads,
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        Some(&[
            InputDeviceRoute::Gamepad(inactive_entity),
            InputDeviceRoute::Gamepad(active_entity),
        ]),
    );

    assert_eq!(command.axis("move.x"), 0.75);
}

#[test]
fn empty_device_route_disables_all_physical_and_virtual_inputs() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings =
        PlayerInputBindings::default().with_key_axis("move.x", KeyCode::KeyA, KeyCode::KeyD);
    let mut virtual_input = VirtualInputState::default();
    keyboard.press(KeyCode::KeyD);
    virtual_input.set_axis("move.x", -1.0);
    virtual_input.press_action("ui.confirm");

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &virtual_input,
        Some(&[]),
    );

    assert!(command.axes.is_empty());
    assert!(command.actions.is_empty());
    assert!(command.pointers.is_empty());
}
