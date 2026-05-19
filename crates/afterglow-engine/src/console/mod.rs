mod autocomplete;
mod commands;
mod executor;
mod overlay;

use bevy::{input::keyboard::KeyboardInput, prelude::*};

pub use autocomplete::{ConsoleAutocompleteRegistry, ConsoleCompletion, complete_console_input};
pub use commands::{
    ConsoleCommand, ConsoleParseError, parse_console_command, tokenize_console_line,
};
pub use executor::{
    ConsoleCommandQueue, ConsoleConnectionState, ConsoleCvar, ConsoleCvarValue, ConsoleCvars,
    ConsoleNetworkRequest, ConsoleNetworkState, ConsoleOutput, DevConsoleState,
    drain_console_network_requests, run_console_command,
};

pub struct DevConsolePlugin;

impl Plugin for DevConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DevConsoleState>()
            .init_resource::<ConsoleCommandQueue>()
            .init_resource::<ConsoleNetworkState>()
            .init_resource::<ConsoleCvars>()
            .init_resource::<ConsoleAutocompleteRegistry>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<KeyboardInput>()
            .add_message::<ConsoleNetworkRequest>()
            .add_systems(Startup, overlay::spawn_console_overlay)
            .add_systems(
                Update,
                (
                    toggle_console,
                    overlay::capture_console_keyboard,
                    executor::execute_queued_console_commands,
                    overlay::sync_console_overlay,
                )
                    .chain()
                    .in_set(crate::core::schedule::AfterglowSet::DebugAndMetrics),
            );
    }
}

fn toggle_console(keys: Option<Res<ButtonInput<KeyCode>>>, mut console: ResMut<DevConsoleState>) {
    if keys.is_some_and(|keys| console_toggle_requested(&keys)) {
        console.open = !console.open;
        console.history_cursor = None;
        console.completion_index = 0;
    }
}

fn console_toggle_requested(keys: &ButtonInput<KeyCode>) -> bool {
    keys.just_pressed(KeyCode::Backquote)
        && !keys.pressed(KeyCode::ShiftLeft)
        && !keys.pressed(KeyCode::ShiftRight)
}

#[cfg(test)]
mod tests;
