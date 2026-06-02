# Input API

## Status

The old string-keyed `PlayerCommand` input stack has been removed. The engine
input surface is Leafwing Input Manager for local input plus
`lightyear_inputs_leafwing` behind the `lightyear` feature for networked ticked
input.

## Plugin

| Item | Description |
|---|---|
| `AfterglowInputPlugin` | Thin engine wrapper around `leafwing_input_manager::plugin::InputManagerPlugin<AfterglowAction>`. |
| `AfterglowLeafwingPlugin` | Type alias for `AfterglowInputPlugin` kept for readability during the migration. |
| `AfterglowAction` | Engine-level `Actionlike` enum for movement, look, use, attack, shield, jump, crouch, sprint, menu, and debug actions. |

`AfterglowRuntimePlugins` includes `AfterglowInputPlugin`. Tests or MinimalPlugin
apps that update Leafwing systems directly must also install Bevy's
`InputPlugin`; normal runtime apps get that from `DefaultPlugins`.

## Gameplay Pattern

Networked gameplay systems read entity-scoped Leafwing state in fixed schedules:

```rust
fn player_combat(
    mut players: Query<(&mut ShieldState, &ActionState<AfterglowAction>)>,
) {
    for (mut shield, actions) in &mut players {
        shield.raised = actions.pressed(&AfterglowAction::RaiseShield);
    }
}
```

Systems that affect replicated gameplay state must run in `FixedUpdate` or the
Lightyear-compatible fixed gameplay schedule.

## Default Gameplay Bindings

`default_gameplay_input_map()` binds core gameplay actions for desktop play:

| Action | Default binding |
|---|---|
| `Move` | WASD / left gamepad stick |
| `Look` | mouse motion / right gamepad stick |
| `Use` | `E` |
| `AttackPrimary` | left mouse |
| `AttackSecondary` | right mouse |
| `RaiseShield` | `Q` |
| `Jump` | `Space` |
| `Crouch` | left control |
| `Sprint` | left shift |
| `Menu` | escape |
| `DebugToggle` | `F3` |

## Networking

`lightyear_inputs_leafwing` is installed by `AfterglowLightyearPlugin` when the
`lightyear` feature is enabled. Afterglow does not serialize custom
`WirePlayerCommand` DTOs anymore.

## Scripted And UI Input

Leafwing's Lightyear adapter is entity-scoped. Gameplay input should live on the
controlled avatar/control entity. UI, editor, and non-gameplay global input may
use separate Leafwing action sets or ordinary Bevy events, but those paths must
not become authoritative combat/network input unless they are mapped onto an
entity `ActionState<AfterglowAction>`.
