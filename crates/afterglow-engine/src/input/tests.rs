use super::*;
use crate::{core::AfterglowCorePlugin, network::NetworkPlayerId, testing::unit_app};

#[test]
fn maps_configured_keys_to_axis_values() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    let bindings = PlayerInputBindings::default()
        .with_key_axis("move.x", KeyCode::KeyA, KeyCode::KeyD)
        .with_key_axis("move.y", KeyCode::KeyS, KeyCode::KeyW);
    keyboard.press(KeyCode::KeyW);
    keyboard.press(KeyCode::KeyD);

    let command = read_player_command(
        &keyboard,
        &mouse,
        &Touches::default(),
        [],
        &bindings,
        &VirtualInputState::default(),
        NetworkPlayerId(7),
        12,
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
fn maps_configured_buttons_to_ordered_string_actions() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mut mouse = ButtonInput::<MouseButton>::default();
    let bindings = PlayerInputBindings::default()
        .with_key_action(KeyCode::KeyE, "use")
        .with_mouse_action(MouseButton::Left, "attack.primary")
        .with_key_action(KeyCode::Tab, "inventory");
    keyboard.press(KeyCode::KeyE);
    keyboard.press(KeyCode::Tab);
    mouse.press(MouseButton::Left);

    let command = read_player_command(
        &keyboard,
        &mouse,
        &Touches::default(),
        [],
        &bindings,
        &VirtualInputState::default(),
        NetworkPlayerId(1),
        0,
    );

    assert_eq!(
        command.actions,
        [
            InputAction::new("use"),
            InputAction::new("attack.primary"),
            InputAction::new("inventory")
        ]
    );
}

#[test]
fn default_bindings_do_not_emit_game_specific_inputs() {
    let mut keyboard = ButtonInput::<KeyCode>::default();
    let mouse = ButtonInput::<MouseButton>::default();
    keyboard.press(KeyCode::KeyW);
    keyboard.press(KeyCode::KeyE);
    keyboard.press(KeyCode::Space);

    let command = read_player_command(
        &keyboard,
        &mouse,
        &Touches::default(),
        [],
        &PlayerInputBindings::default(),
        &VirtualInputState::default(),
        NetworkPlayerId(1),
        0,
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
    let mut gamepad = Gamepad::default();
    gamepad.analog_mut().set(GamepadAxis::LeftStickX, 0.75);
    gamepad.digital_mut().press(GamepadButton::South);

    let command = read_player_command(
        &keyboard,
        &mouse,
        &Touches::default(),
        [&gamepad],
        &bindings,
        &VirtualInputState::default(),
        NetworkPlayerId(1),
        0,
    );

    assert_eq!(
        command.axes,
        [InputAxisValue {
            axis: InputAxis::new("move.x"),
            value: 0.75,
        }]
    );
    assert_eq!(command.actions, [InputAction::new("confirm")]);
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

    let command = read_player_command(
        &keyboard,
        &mouse,
        &Touches::default(),
        [],
        &PlayerInputBindings::default(),
        &virtual_input,
        NetworkPlayerId(1),
        0,
    );

    assert_eq!(command.axes[0].axis, InputAxis::new("tablet.pressure"));
    assert_eq!(command.actions, [InputAction::new("editor.paint")]);
    assert_eq!(command.pointers[0].device, PointerDevice::Pen);
    assert_eq!(command.pointers[0].pressure, Some(0.5));
}

#[test]
fn input_plugin_enqueues_one_command_per_local_player() {
    let mut app = unit_app();
    app.add_plugins(AfterglowInputPlugin);
    app.world_mut()
        .resource_mut::<LocalPlayers>()
        .add_player(NetworkPlayerId(2));

    app.world_mut()
        .resource_mut::<PlayerInputBindings>()
        .axes
        .push(AxisBinding {
            axis: InputAxis::new("move.y"),
            source: AxisSource::KeyPair {
                negative: KeyCode::KeyS,
                positive: KeyCode::KeyW,
                normalize_pair: false,
            },
        });
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);

    app.update();

    let queue = app.world().resource::<PlayerCommandQueue>();
    assert_eq!(queue.commands().len(), 2);
    assert_eq!(queue.commands()[0].player, NetworkPlayerId(1));
    assert_eq!(queue.commands()[1].player, NetworkPlayerId(2));
    assert_eq!(queue.commands()[0].tick, 0);
    assert_eq!(queue.commands()[0].axes[0].axis, InputAxis::new("move.y"));
    assert_eq!(queue.commands()[0].axes[0].value, 1.0);

    app.update();

    let queue = app.world().resource::<PlayerCommandQueue>();
    assert_eq!(queue.commands().len(), 2);
    assert_eq!(queue.commands()[0].tick, 1);
}

#[test]
fn input_plugin_registers_with_core_plugin() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AfterglowCorePlugin, AfterglowInputPlugin));
    assert!(app.world().contains_resource::<PlayerCommandQueue>());
}
