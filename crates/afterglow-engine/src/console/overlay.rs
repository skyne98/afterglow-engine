use bevy::{
    input::{ButtonState, keyboard::KeyboardInput},
    prelude::*,
};

use super::{
    ConsoleAutocompleteRegistry, ConsoleCommandQueue, ConsoleCompletion, ConsoleCvarValue,
    ConsoleCvars, DevConsoleState, autocomplete::complete_console_input_from,
};

#[derive(Component)]
pub(super) struct ConsoleOverlayRoot;

#[derive(Component)]
pub(super) struct ConsoleScrollbackText;

#[derive(Component)]
pub(super) struct ConsoleCompletionText;

#[derive(Component)]
pub(super) struct ConsoleInputText;

const FONT_SIZE: f32 = 13.0;
const MAX_SCROLLBACK_LINES: usize = 14;

pub(super) fn spawn_console_overlay(mut commands: Commands) {
    commands
        .spawn((
            ConsoleOverlayRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(42.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::End,
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.01, 0.01, 0.012, 0.88)),
            GlobalZIndex(i32::MAX - 8),
            Visibility::Hidden,
        ))
        .with_children(|root| {
            root.spawn((
                ConsoleScrollbackText,
                Text(String::new()),
                TextFont {
                    font_size: FONT_SIZE,
                    ..default()
                },
                TextColor(Color::srgb(0.74, 0.84, 0.88)),
            ));
            root.spawn((
                ConsoleCompletionText,
                Text(String::new()),
                TextFont {
                    font_size: FONT_SIZE * 0.92,
                    ..default()
                },
                TextColor(Color::srgb(0.66, 0.78, 0.72)),
            ));
            root.spawn((
                ConsoleInputText,
                Text("] _".into()),
                TextFont {
                    font_size: FONT_SIZE * 1.08,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.98, 0.86)),
            ));
        });
}

pub(super) fn capture_console_keyboard(
    mut keyboard: MessageReader<KeyboardInput>,
    mut console: ResMut<DevConsoleState>,
    mut queue: ResMut<ConsoleCommandQueue>,
    cvars: Res<ConsoleCvars>,
    autocomplete: Res<ConsoleAutocompleteRegistry>,
) {
    for event in keyboard.read() {
        if !console.open || event.state != ButtonState::Pressed {
            continue;
        }

        let handled = match event.key_code {
            KeyCode::Enter | KeyCode::NumpadEnter => submit_current_input(&mut console, &mut queue),
            KeyCode::Backspace => pop_input(&mut console),
            KeyCode::Escape => close_console(&mut console),
            KeyCode::ArrowUp => recall_history(&mut console, HistoryDirection::Previous),
            KeyCode::ArrowDown => recall_history(&mut console, HistoryDirection::Next),
            KeyCode::Tab => accept_current_completion(&mut console, &cvars, &autocomplete),
            KeyCode::Backquote => true,
            _ => false,
        };

        if !handled && let Some(text) = &event.text {
            push_text(&mut console, text);
        }
    }
}

pub(super) fn sync_console_overlay(
    console: Res<DevConsoleState>,
    cvars: Res<ConsoleCvars>,
    autocomplete: Res<ConsoleAutocompleteRegistry>,
    mut roots: Query<&mut Visibility, With<ConsoleOverlayRoot>>,
    mut text_group: ParamSet<(
        Query<&mut Text, With<ConsoleScrollbackText>>,
        Query<&mut Text, With<ConsoleCompletionText>>,
        Query<&mut Text, With<ConsoleInputText>>,
    )>,
) {
    for mut visibility in &mut roots {
        *visibility = if console.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    for mut text in &mut text_group.p0() {
        text.0 = format_scrollback(&console);
    }
    for mut text in &mut text_group.p1() {
        text.0 = format_completions(&console, &cvars, &autocomplete);
    }
    for mut text in &mut text_group.p2() {
        text.0 = if console.open {
            format!("] {}_", console.input)
        } else {
            "]".into()
        };
    }
}

#[derive(Clone, Copy)]
pub(super) enum HistoryDirection {
    Previous,
    Next,
}

pub(super) fn submit_current_input(
    console: &mut DevConsoleState,
    queue: &mut ConsoleCommandQueue,
) -> bool {
    let line = console.input.trim();
    if line.is_empty() {
        return false;
    }
    queue.push(line.to_string());
    console.input.clear();
    reset_console_navigation(console);
    true
}

pub(super) fn recall_history(console: &mut DevConsoleState, direction: HistoryDirection) -> bool {
    if console.history.is_empty() {
        return false;
    }

    match direction {
        HistoryDirection::Previous => {
            let index = console
                .history_cursor
                .map_or(console.history.len() - 1, |index| index.saturating_sub(1));
            console.history_cursor = Some(index);
            console.input = console.history[index].clone();
        }
        HistoryDirection::Next => {
            let Some(index) = console.history_cursor else {
                return false;
            };
            if index + 1 >= console.history.len() {
                console.history_cursor = None;
                console.input.clear();
            } else {
                let next = index + 1;
                console.history_cursor = Some(next);
                console.input = console.history[next].clone();
            }
        }
    }
    console.completion_index = 0;
    true
}

pub(super) fn accept_current_completion(
    console: &mut DevConsoleState,
    cvars: &ConsoleCvars,
    autocomplete: &ConsoleAutocompleteRegistry,
) -> bool {
    let completions = complete_console_input_from(cvars, autocomplete, &console.input);
    if completions.is_empty() {
        return false;
    }
    let index = selected_completion_index(console, completions.len());
    console.input = completions[index].replacement.clone();
    reset_console_navigation(console);
    true
}

fn close_console(console: &mut DevConsoleState) -> bool {
    console.open = false;
    reset_console_navigation(console);
    true
}

fn pop_input(console: &mut DevConsoleState) -> bool {
    let changed = console.input.pop().is_some();
    if changed {
        reset_console_navigation(console);
    }
    changed
}

fn push_text(console: &mut DevConsoleState, text: &str) {
    let previous_len = console.input.len();
    console
        .input
        .extend(text.chars().filter(|ch| !ch.is_control()));
    if console.input.len() != previous_len {
        reset_console_navigation(console);
    }
}

fn reset_console_navigation(console: &mut DevConsoleState) {
    console.history_cursor = None;
    console.completion_index = 0;
}

fn format_scrollback(console: &DevConsoleState) -> String {
    let mut lines = console
        .scrollback
        .iter()
        .rev()
        .flat_map(|output| {
            let prefix = if output.success { ">" } else { "!" };
            output
                .lines
                .iter()
                .rev()
                .map(move |line| format!("{prefix} {line}"))
        })
        .take(MAX_SCROLLBACK_LINES)
        .collect::<Vec<_>>();
    lines.reverse();
    lines.join("\n")
}

fn format_completions(
    console: &DevConsoleState,
    cvars: &ConsoleCvars,
    autocomplete: &ConsoleAutocompleteRegistry,
) -> String {
    if !console.open || console.input.is_empty() {
        return String::new();
    }
    let completions = complete_console_input_from(cvars, autocomplete, &console.input);
    let limit = completion_limit(cvars).min(completions.len());
    let selected = selected_completion_index(console, completions.len());
    completions
        .iter()
        .take(limit)
        .enumerate()
        .map(|(index, completion)| format_completion_line(index, selected, completion))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_completion_line(index: usize, selected: usize, completion: &ConsoleCompletion) -> String {
    let marker = if index == selected { ">" } else { " " };
    format!(
        "{marker} {:<24} {}",
        completion.display, completion.description
    )
}

fn selected_completion_index(console: &DevConsoleState, completion_count: usize) -> usize {
    if completion_count == 0 {
        0
    } else {
        console.completion_index % completion_count
    }
}

fn completion_limit(cvars: &ConsoleCvars) -> usize {
    match cvars
        .get("console.autocomplete.max_results")
        .map(|cvar| &cvar.value)
    {
        Some(ConsoleCvarValue::I64(value)) => (*value).clamp(1, 32) as usize,
        _ => 8,
    }
}
