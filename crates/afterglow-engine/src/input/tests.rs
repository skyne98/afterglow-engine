use super::*;
use crate::network::NetworkPlayerId;

mod bindings;
mod plugin;
mod routes;

fn command_with(
    keyboard: &ButtonInput<KeyCode>,
    mouse: &ButtonInput<MouseButton>,
    gamepads: &[GamepadInput],
    mouse_delta: Vec2,
    bindings: &PlayerInputBindings,
    virtual_input: &VirtualInputState,
    devices: Option<&[InputDeviceRoute]>,
) -> PlayerCommand {
    read_player_command(
        keyboard,
        mouse,
        &Touches::default(),
        gamepads,
        mouse_delta,
        bindings,
        virtual_input,
        devices,
        NetworkPlayerId(7),
        12,
    )
}
