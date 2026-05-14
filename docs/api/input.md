# Input API

Afterglow input converts every control source into generic per-tick
`PlayerCommand` values. Gameplay systems should consume commands, not raw Bevy
keyboard, mouse, touch, or gamepad resources. Raw devices are only one producer:
scripts, cutscenes, AI possession, editor tools, tests, and UI can all drive the
same command stream.

## Plugin

| Item | Description |
|---|---|
| `AfterglowInputPlugin` | Registers input resources, mouse-motion messages, and command collection in `AfterglowSet::ReadInput`. |

`AfterglowRuntimePlugins` includes `AfterglowInputPlugin`.

## Commands

| Type | Description |
|---|---|
| `PlayerCommand` | Network/prediction/replay payload: `player`, `tick`, named `axes`, phased `actions`, and `pointers`. Empty commands are valid for menus, lobbies, spectators, or non-camera scenes. |
| `InputAxis` | String axis ID, for example `move.x`, `look.y`, `menu.scroll`. |
| `InputAxisValue` | One command-time axis sample. |
| `InputAction` | String action ID. |
| `InputActionPhase` | `Pressed`, `Held`, or `Released`. |
| `InputActionValue` | One action plus its phase. Constructors: `pressed`, `held`, `released`, `new`. |
| `PointerInput` | Generic pointer sample for mouse-like, touch, pen/tablet, or custom devices. Includes position, delta, pressure, tilt, twist, and primary state. |
| `PointerDevice` | `Mouse`, `Touch`, `Pen`, or `Unknown`. |

`PlayerCommand::axis()`, `action_pressed()`, `action_held()`, and
`action_released()` are convenience readers for gameplay systems.
`axis()` returns the last sample for a name, so explicit command-layer
overrides are deterministic.

## Bindings

| Type | Description |
|---|---|
| `PlayerInputBindings` | Resource containing input contexts. Defaults emit no game-specific input. Builder helpers add bindings to the default `gameplay` context. |
| `InputContext` | Named binding layer with `priority`, `enabled`, and `consume`. Higher-priority contexts run first; a consuming context blocks lower contexts when it emits input. |
| `InputContextId` | String context ID. |
| `AxisBinding` | Maps a raw source to a named axis. |
| `AxisSource` | `KeyPair`, `GamepadAxis`, `GamepadButtonPair`, or `MouseMotion`. |
| `AxisComponent` | `X` or `Y` selector for pointer-style axes. |
| `ActionBinding` | Maps a raw source to a named action. |
| `ActionInput` | `Key`, `KeyChord`, `Mouse`, `GamepadButton`, or `TouchAny`. |

Common helpers:

```rust
app.insert_resource(
    PlayerInputBindings::default()
        .with_key_axis("move.x", KeyCode::KeyA, KeyCode::KeyD)
        .with_key_axis("move.y", KeyCode::KeyS, KeyCode::KeyW)
        .with_mouse_motion_axis("look.x", AxisComponent::X, 0.002)
        .with_mouse_motion_axis("look.y", AxisComponent::Y, -0.002)
        .with_key_action(KeyCode::Space, "jump")
        .with_key_chord_action(KeyCode::Backquote, [KeyCode::ShiftLeft], "debug.toggle")
        .with_mouse_action(MouseButton::Left, "use.primary"),
);
```

When multiple contexts emit the same axis, the highest-priority context wins.
Virtual axes are appended after bound contexts and can intentionally override
physical input for the same axis name.

## Device Routing

| Type | Description |
|---|---|
| `LocalPlayers` | Local transport peer plus one or more local `NetworkPlayerId`s controlled by this app instance. |
| `LocalInputRoutes` | Optional per-player device routing. Without a route, a player can read all devices. |
| `LocalInputRoute` | One player and its allowed devices. |
| `InputDeviceRoute` | `KeyboardMouse`, `Gamepad(Entity)`, `Touch`, or `Virtual`. |

Use routes for split-screen, local co-op, editor/gameplay separation, or
assigning a specific gamepad to a specific player.

## Virtual And Editor Input

| Type | Description |
|---|---|
| `VirtualInputState` | Per-frame scripted command input. Supports shared input for every virtual-routed player and targeted input for one `NetworkPlayerId`. |
| `VirtualInputBuffer` | One scripted command buffer containing axes, actions, and pointers. |

`VirtualInputState` is the per-frame escape hatch for command input that does
not come from Bevy's normal keyboard/mouse/gamepad resources:

- touch virtual sticks
- UI widgets
- graphics tablet pens
- editor gizmos
- platform-specific devices
- cutscene or script control

It supports `set_axis`, `press_action`, `hold_action`, `release_action`,
`push_action`, and `push_pointer` for shared virtual input. It also supports
`set_player_axis`, `press_player_action`, `hold_player_action`,
`release_player_action`, `push_player_action`, and `push_player_pointer` for
one player. The resource is cleared after command collection each frame.

```rust
fn drive_cutscene(mut virtual_input: ResMut<VirtualInputState>) {
    virtual_input.set_player_axis(NetworkPlayerId(1), "move.x", 1.0);
    virtual_input.press_player_action(NetworkPlayerId(1), "script.use");
}
```

## Networking

The network command wire format mirrors `PlayerCommand` through explicit DTOs:
`WirePlayerCommand`, `WireAxisValue`, `WireActionValue`, `WireActionPhase`, and
`WirePointerInput`.
