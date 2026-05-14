use super::*;

#[test]
fn maps_configured_keys_to_axis_values() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings = PlayerInputBindings::default()
        .with_key_axis("move.x", KeyCode::KeyA, KeyCode::KeyD)
        .with_key_axis("move.y", KeyCode::KeyS, KeyCode::KeyW);
    keyboard.press(KeyCode::KeyW);
    keyboard.press(KeyCode::KeyD);

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        None,
    );

    assert_eq!(command.player, NetworkPlayerId(7));
    assert_eq!(command.tick, 12);
    assert_eq!(
        command.axes,
        [
            InputAxisValue {
                axis: InputAxis::new("move.x"),
                value: 1.0,
            },
            InputAxisValue {
                axis: InputAxis::new("move.y"),
                value: 1.0,
            },
        ]
    );
}

#[test]
fn maps_button_action_phases_to_ordered_string_actions() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mut mouse = ButtonInput::<MouseButton>::default();
    let bindings = PlayerInputBindings::default()
        .with_key_action(KeyCode::KeyE, "use")
        .with_mouse_action(MouseButton::Left, "attack.primary")
        .with_key_action(KeyCode::Tab, "inventory");
    keyboard.press(KeyCode::KeyE);
    keyboard.press(KeyCode::Tab);
    keyboard.clear_just_pressed(KeyCode::Tab);
    mouse.press(MouseButton::Left);
    mouse.clear_just_pressed(MouseButton::Left);
    mouse.release(MouseButton::Left);

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        None,
    );

    assert_eq!(
        command.actions,
        [
            InputActionValue::pressed("use"),
            InputActionValue::released("attack.primary"),
            InputActionValue::held("inventory"),
        ]
    );
    assert!(command.action_pressed("use"));
    assert!(command.action_held("inventory"));
    assert!(command.action_released("attack.primary"));
}

#[test]
fn same_tick_press_and_release_emits_both_action_edges() {
    let keyboard = ButtonInput::<KeyCode>::default();
    let mut mouse = ButtonInput::<MouseButton>::default();
    let bindings =
        PlayerInputBindings::default().with_mouse_action(MouseButton::Left, "attack.primary");
    mouse.press(MouseButton::Left);
    mouse.release(MouseButton::Left);

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        None,
    );

    assert_eq!(
        command.actions,
        [
            InputActionValue::pressed("attack.primary"),
            InputActionValue::released("attack.primary"),
        ]
    );
}

#[test]
fn key_chord_action_requires_pressed_modifiers() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings = PlayerInputBindings::default().with_key_chord_action(
        KeyCode::Backquote,
        [KeyCode::ShiftLeft],
        "debug.toggle",
    );
    keyboard.press(KeyCode::Backquote);

    let without_modifier = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        None,
    );

    keyboard.press(KeyCode::ShiftLeft);
    let with_modifier = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        None,
    );

    assert!(without_modifier.actions.is_empty());
    assert_eq!(
        with_modifier.actions,
        [InputActionValue::pressed("debug.toggle")]
    );
}

#[test]
fn input_context_priority_can_consume_lower_priority_bindings() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let mut ui = InputContext::new("ui").with_priority(10).consuming();
    ui.add_key_action(KeyCode::Escape, "ui.close");
    let bindings = PlayerInputBindings::default()
        .with_key_action(KeyCode::Escape, "game.pause")
        .with_context(ui);
    keyboard.press(KeyCode::Escape);

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        None,
    );

    assert_eq!(command.actions, [InputActionValue::pressed("ui.close")]);
}

#[test]
fn higher_priority_axis_context_wins_over_lower_priority_context() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let mut gameplay = InputContext::new("gameplay");
    gameplay.add_key_axis("move.x", KeyCode::KeyA, KeyCode::KeyD);
    let mut editor = InputContext::new("editor").with_priority(10);
    editor.add_key_axis("move.x", KeyCode::ArrowLeft, KeyCode::ArrowRight);
    let bindings = PlayerInputBindings::default()
        .with_context(gameplay)
        .with_context(editor);
    keyboard.press(KeyCode::KeyD);
    keyboard.press(KeyCode::ArrowLeft);

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        None,
    );

    assert_eq!(command.axis("move.x"), -1.0);
}

#[test]
fn axis_helper_uses_last_sample_for_overrides() {
    let command = PlayerCommand {
        axes: vec![
            InputAxisValue {
                axis: InputAxis::new("move.x"),
                value: 1.0,
            },
            InputAxisValue {
                axis: InputAxis::new("move.x"),
                value: -0.5,
            },
        ],
        ..Default::default()
    };

    assert_eq!(command.axis("move.x"), -0.5);
}

#[test]
fn disabled_contexts_do_not_emit_inputs() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let mut bindings = PlayerInputBindings::default().with_key_action(KeyCode::KeyE, "use");
    bindings.set_context_enabled(InputContext::DEFAULT_GAMEPLAY, false);
    keyboard.press(KeyCode::KeyE);

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        None,
    );

    assert!(command.actions.is_empty());
}

#[test]
fn virtual_axes_override_physical_axes_for_same_name() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings =
        PlayerInputBindings::default().with_key_axis("move.x", KeyCode::KeyA, KeyCode::KeyD);
    let mut virtual_input = VirtualInputState::default();
    keyboard.press(KeyCode::KeyD);
    virtual_input.set_axis("move.x", -0.25);

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &bindings,
        &virtual_input,
        None,
    );

    assert_eq!(command.axis("move.x"), -0.25);
}

#[test]
fn default_bindings_do_not_emit_game_specific_inputs() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    keyboard.press(KeyCode::KeyW);
    keyboard.press(KeyCode::KeyE);
    keyboard.press(KeyCode::Space);

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &PlayerInputBindings::default(),
        &VirtualInputState::default(),
        None,
    );

    assert!(command.axes.is_empty());
    assert!(command.actions.is_empty());
    assert!(command.pointers.is_empty());
}

#[test]
fn maps_gamepad_axes_and_buttons_to_named_inputs() {
    let keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings = PlayerInputBindings::default()
        .with_gamepad_axis("move.x", GamepadAxis::LeftStickX, 0.1)
        .with_gamepad_action(GamepadButton::South, "confirm");
    let gamepad_entity = Entity::from_raw_u32(99).unwrap();
    let mut gamepad = Gamepad::default();
    gamepad.analog_mut().set(GamepadAxis::LeftStickX, 0.75);
    gamepad.digital_mut().press(GamepadButton::South);
    let gamepads = [GamepadInput {
        entity: gamepad_entity,
        gamepad: &gamepad,
    }];

    let command = command_with(
        &keyboard,
        &mouse,
        &gamepads,
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        Some(&[InputDeviceRoute::Gamepad(gamepad_entity)]),
    );

    assert_eq!(
        command.axes,
        [InputAxisValue {
            axis: InputAxis::new("move.x"),
            value: 0.75,
        }]
    );
    assert_eq!(command.actions, [InputActionValue::pressed("confirm")]);
}

#[test]
fn gamepad_action_edges_are_not_masked_by_another_allowed_gamepad() {
    let keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings = PlayerInputBindings::default().with_gamepad_action(GamepadButton::South, "use");
    let held_entity = Entity::from_raw_u32(4).unwrap();
    let pressed_entity = Entity::from_raw_u32(5).unwrap();
    let mut held_gamepad = Gamepad::default();
    held_gamepad.digital_mut().press(GamepadButton::South);
    held_gamepad
        .digital_mut()
        .clear_just_pressed(GamepadButton::South);
    let mut pressed_gamepad = Gamepad::default();
    pressed_gamepad.digital_mut().press(GamepadButton::South);
    let gamepads = [
        GamepadInput {
            entity: held_entity,
            gamepad: &held_gamepad,
        },
        GamepadInput {
            entity: pressed_entity,
            gamepad: &pressed_gamepad,
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
            InputDeviceRoute::Gamepad(held_entity),
            InputDeviceRoute::Gamepad(pressed_entity),
        ]),
    );

    assert_eq!(command.actions, [InputActionValue::pressed("use")]);
}

#[test]
fn player_device_routes_filter_keyboard_and_gamepads() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings = PlayerInputBindings::default()
        .with_key_axis("move.x", KeyCode::KeyA, KeyCode::KeyD)
        .with_gamepad_axis("move.x", GamepadAxis::LeftStickX, 0.1);
    keyboard.press(KeyCode::KeyD);
    let gamepad_entity = Entity::from_raw_u32(4).unwrap();
    let mut gamepad = Gamepad::default();
    gamepad.analog_mut().set(GamepadAxis::LeftStickX, -0.5);
    let gamepads = [GamepadInput {
        entity: gamepad_entity,
        gamepad: &gamepad,
    }];

    let keyboard_command = command_with(
        &keyboard,
        &mouse,
        &gamepads,
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        Some(&[InputDeviceRoute::KeyboardMouse]),
    );
    let gamepad_command = command_with(
        &keyboard,
        &mouse,
        &gamepads,
        Vec2::ZERO,
        &bindings,
        &VirtualInputState::default(),
        Some(&[InputDeviceRoute::Gamepad(gamepad_entity)]),
    );

    assert_eq!(keyboard_command.axis("move.x"), 1.0);
    assert_eq!(gamepad_command.axis("move.x"), -0.5);
}

#[test]
fn mouse_motion_axes_support_camera_look() {
    let keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings = PlayerInputBindings::default()
        .with_mouse_motion_axis("look.x", AxisComponent::X, 0.5)
        .with_mouse_motion_axis("look.y", AxisComponent::Y, -0.25);

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::new(4.0, 8.0),
        &bindings,
        &VirtualInputState::default(),
        None,
    );

    assert_eq!(command.axis("look.x"), 2.0);
    assert_eq!(command.axis("look.y"), -2.0);
}

#[test]
fn virtual_input_supports_touch_and_pen_editor_feeds() {
    let keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let mut virtual_input = VirtualInputState::default();
    virtual_input.set_axis("tablet.pressure", 0.5);
    virtual_input.press_action("editor.paint");
    virtual_input.push_pointer(PointerInput {
        pressure: Some(0.5),
        tilt: Some(Vec2::new(0.1, -0.2)),
        ..PointerInput::pen(42, Vec2::new(100.0, 200.0))
    });

    let command = command_with(
        &keyboard,
        &mouse,
        &[],
        Vec2::ZERO,
        &PlayerInputBindings::default(),
        &virtual_input,
        Some(&[InputDeviceRoute::Virtual]),
    );

    assert_eq!(command.axes[0].axis, InputAxis::new("tablet.pressure"));
    assert_eq!(command.actions, [InputActionValue::pressed("editor.paint")]);
    assert_eq!(command.pointers[0].device, PointerDevice::Pen);
    assert_eq!(command.pointers[0].pressure, Some(0.5));
}
