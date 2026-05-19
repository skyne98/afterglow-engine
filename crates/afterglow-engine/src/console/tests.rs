use super::*;
use crate::console::{
    commands::{CvarCommand, NetCommand},
    executor::ConsoleConnectionState,
    overlay::{ConsoleOverlayRoot, HistoryDirection},
};

use bevy::input::{ButtonState, keyboard::KeyboardInput};

#[test]
fn console_toggle_ignores_perf_hud_chord() {
    let mut keys = ButtonInput::<KeyCode>::default();
    keys.press(KeyCode::Backquote);

    assert!(super::console_toggle_requested(&keys));

    let mut keys = ButtonInput::<KeyCode>::default();
    keys.press(KeyCode::ShiftLeft);
    keys.press(KeyCode::Backquote);

    assert!(!super::console_toggle_requested(&keys));
}

#[test]
fn parser_handles_nested_clap_commands() {
    let command = parse_console_command("net latency --ms 120").unwrap();

    assert!(matches!(
        command,
        ConsoleCommand::Net {
            command: NetCommand::Latency(args),
        } if args.ms == 120
    ));
}

#[test]
fn tokenizer_preserves_quoted_arguments() {
    let tokens = tokenize_console_line("connect \"example.test:8820\"").unwrap();

    assert_eq!(tokens, ["connect", "example.test:8820"]);
}

#[test]
fn parser_reports_unknown_commands() {
    let error = parse_console_command("teleport 1 2 3").unwrap_err();

    assert!(error.message.contains("unrecognized subcommand"));
}

#[test]
fn connect_local_updates_testable_network_state() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, DevConsolePlugin));

    let output = run_console_command(app.world_mut(), "connect local");

    assert!(output.success);
    assert_eq!(
        app.world().resource::<ConsoleNetworkState>().connection,
        ConsoleConnectionState::ConnectingLocal
    );
    assert!(
        app.world()
            .resource::<ConsoleNetworkState>()
            .local_server_running
    );
}

#[test]
fn direct_connect_local_emits_network_requests() {
    let mut world = World::new();

    let output = run_console_command(&mut world, "connect local");
    let requests = drain_console_network_requests(&mut world);

    assert!(output.success);
    assert_eq!(
        requests,
        [
            ConsoleNetworkRequest::StartLocalServer,
            ConsoleNetworkRequest::ConnectLocal,
        ]
    );
}

#[test]
fn plugin_spawns_hidden_console_overlay() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, DevConsolePlugin));

    app.update();

    let visibility = app
        .world_mut()
        .query_filtered::<&Visibility, With<ConsoleOverlayRoot>>()
        .single(app.world())
        .unwrap();
    assert_eq!(*visibility, Visibility::Hidden);
}

#[test]
fn keyboard_text_and_enter_submit_through_normal_update() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, DevConsolePlugin));
    app.world_mut().resource_mut::<DevConsoleState>().open = true;
    write_keyboard_input(&mut app, KeyCode::KeyN, Some("n"));
    write_keyboard_input(&mut app, KeyCode::KeyE, Some("e"));
    write_keyboard_input(&mut app, KeyCode::KeyT, Some("t"));
    write_keyboard_input(&mut app, KeyCode::Space, Some(" "));
    write_keyboard_input(&mut app, KeyCode::KeyS, Some("s"));
    write_keyboard_input(&mut app, KeyCode::KeyT, Some("t"));
    write_keyboard_input(&mut app, KeyCode::KeyA, Some("a"));
    write_keyboard_input(&mut app, KeyCode::KeyT, Some("t"));
    write_keyboard_input(&mut app, KeyCode::KeyS, Some("s"));
    write_keyboard_input(&mut app, KeyCode::Enter, None);

    app.update();

    let state = app.world().resource::<DevConsoleState>();
    assert!(state.input.is_empty());
    assert_eq!(state.history, ["net stats"]);
}

#[test]
fn history_navigation_recalls_recent_commands() {
    let mut state = DevConsoleState {
        history: vec!["net status".into(), "net stats".into()],
        ..default()
    };

    assert!(overlay::recall_history(
        &mut state,
        HistoryDirection::Previous
    ));
    assert_eq!(state.input, "net stats");
    assert!(overlay::recall_history(
        &mut state,
        HistoryDirection::Previous
    ));
    assert_eq!(state.input, "net status");
    assert!(overlay::recall_history(&mut state, HistoryDirection::Next));
    assert_eq!(state.input, "net stats");
}

#[test]
fn tab_completion_accepts_current_selection() {
    let cvars = ConsoleCvars::default();
    let autocomplete = ConsoleAutocompleteRegistry::default();
    let mut state = DevConsoleState {
        input: "conn".into(),
        ..default()
    };

    assert!(overlay::accept_current_completion(
        &mut state,
        &cvars,
        &autocomplete
    ));
    assert_eq!(state.input, "connect ");
}

#[test]
fn cvar_get_set_uses_typed_values() {
    let mut world = World::new();

    assert_eq!(
        run_console_command(&mut world, "cvar get net.tick_rate").text(),
        "net.tick_rate=60"
    );
    assert!(run_console_command(&mut world, "cvar set net.tick_rate 30").success);
    assert_eq!(
        run_console_command(&mut world, "cvar get net.tick_rate").text(),
        "net.tick_rate=30"
    );
    assert!(!run_console_command(&mut world, "cvar set net.tick_rate nope").success);
}

#[test]
fn command_queue_executes_from_unit_tests() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, DevConsolePlugin));
    app.world_mut()
        .resource_mut::<ConsoleCommandQueue>()
        .push("disconnect");

    app.update();

    assert_eq!(
        app.world().resource::<DevConsoleState>().history,
        ["disconnect"]
    );
}

#[test]
fn autocomplete_completes_top_level_prefixes() {
    let world = World::new();
    let completions = complete_console_input(&world, "co");

    assert_eq!(completions[0].display, "connect");
    assert_eq!(completions[0].replacement, "connect ");
}

#[test]
fn autocomplete_handles_trailing_space_subcommands() {
    let world = World::new();
    let completions = complete_console_input(&world, "net ");
    let displays = completions
        .iter()
        .map(|completion| completion.display.as_str())
        .collect::<Vec<_>>();

    assert_eq!(displays, ["status", "stats", "links", "latency"]);
}

#[test]
fn autocomplete_completes_cvar_names_and_bool_values() {
    let mut world = World::new();
    world.init_resource::<ConsoleCvars>();

    let cvars = complete_console_input(&world, "cvar get net.");
    assert!(cvars.iter().any(|item| item.display == "net.tick_rate"));

    let values = complete_console_input(&world, "cvar set console.ui.enabled ");
    assert_eq!(
        values
            .iter()
            .map(|completion| completion.display.as_str())
            .collect::<Vec<_>>(),
        ["false", "true"]
    );
}

#[test]
fn autocomplete_suggests_connect_targets_and_latency_options() {
    let mut world = World::new();
    world.insert_resource(ConsoleAutocompleteRegistry {
        endpoints: vec!["10.0.0.2:8820".into()],
    });

    let targets = complete_console_input(&world, "connect ");
    assert!(targets.iter().any(|item| item.display == "local"));
    assert!(targets.iter().any(|item| item.display == "10.0.0.2:8820"));

    let option = complete_console_input(&world, "net latency ");
    assert_eq!(option[0].display, "--ms");

    let values = complete_console_input(&world, "net latency --ms 1");
    assert!(values.iter().any(|item| item.display == "100"));
    assert!(values.iter().any(|item| item.display == "150"));
}

#[test]
fn parser_exposes_cvar_commands_for_test_harnesses() {
    let command = parse_console_command("cvar set net.prediction_window 8").unwrap();

    assert!(matches!(
        command,
        ConsoleCommand::Cvar {
            command: CvarCommand::Set { name, value },
        } if name == "net.prediction_window" && value == "8"
    ));
}

fn write_keyboard_input(app: &mut App, key_code: KeyCode, text: Option<&str>) {
    let window = app.world_mut().spawn_empty().id();
    app.world_mut()
        .resource_mut::<Messages<KeyboardInput>>()
        .write(KeyboardInput {
            key_code,
            logical_key: bevy::input::keyboard::Key::Unidentified(
                bevy::input::keyboard::NativeKey::Unidentified,
            ),
            state: ButtonState::Pressed,
            text: text.map(Into::into),
            repeat: false,
            window,
        });
}
