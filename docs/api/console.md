# Console API

## Status

The engine has a Source-style development console backed by `clap` subcommands.
The parser/executor/autocomplete core is renderless and usable from in-game
systems, startup/test scripts, unit tests, and integration tests. The runtime
plugin also spawns a Bevy UI overlay on top of the same resources and command
execution API.

## Plugin Surface

| Item | Purpose |
|---|---|
| `DevConsolePlugin` | Registers console state, command queue, network state, cvars, autocomplete registry, and network request messages. |
| `DevConsoleState` | Open flag, input buffer, command history/navigation, autocomplete selection, and scrollback output. |
| `ConsoleCommandQueue` | Queue text commands for execution on `Update`; useful for tests, startup scripts, and UI submit actions. |
| `run_console_command(World, line)` | Immediate parser/executor entrypoint for tests and scripted repros. |
| `parse_console_command(line)` | Clap-backed command parser. |
| `complete_console_input(World, input)` | Deterministic tab-completion API with replacements, display labels, and descriptions. |
| `ConsoleNetworkRequest` | Typed request messages emitted by connect/disconnect/server/net debug commands. |
| `ConsoleNetworkState` | Testable mirror of requested connection/server/latency state until real Lightyear wiring consumes the requests. |
| `ConsoleCvars` | Typed console variable store. |
| `drain_console_network_requests(World)` | Test/integration helper for consuming typed network requests from systems or harnesses. |

## Overlay UI

`DevConsolePlugin` spawns a hidden overlay during startup and keeps it synced from
`DevConsoleState`:

- backquote toggles the console; Shift+Backquote remains reserved for the perf HUD
- keyboard text is appended to the input buffer while the console is open
- Enter queues the current command and executes it through the normal update path
- Escape closes the overlay
- Up/Down navigate command history
- Backspace edits the current input
- Tab accepts the current autocomplete selection
- the overlay renders scrollback, the prompt, autocomplete display labels, and descriptions

The UI is intentionally thin: tests and networking harnesses still drive the same
`run_console_command`, `ConsoleCommandQueue`, and `ConsoleNetworkRequest` APIs.

## Command Surface

Initial commands:

| Command | Purpose |
|---|---|
| `connect local` | Request in-process local server startup and local connection. |
| `connect <addr>` | Request remote connection. |
| `disconnect` | Request disconnect. |
| `server start` / `server stop` / `server status` | Local server control surface. |
| `net status` / `net stats` / `net links` | Network inspection. |
| `net latency --ms <value>` | Request simulated network latency. |
| `stats fps` / `stats systems` | Runtime stats placeholders. |
| `cvar get <name>` / `cvar set <name> <value>` | Typed cvar access. |
| `help [topic]` | Console help. |

## Autocomplete

`complete_console_input` handles:

- top-level command prefixes
- trailing-space subcommand completion
- `connect local` and known endpoint completion
- nested `net`, `server`, `stats`, and `cvar` commands
- option-name completion for `net latency --ms`
- cvar-name completion
- typed cvar-value completion for booleans, integers, floats, and text

Each completion includes a full replacement line, display text, and a
description so the overlay can render Source-style autocomplete rows without
reimplementing command knowledge.

## Test Pattern

Use the command API directly in tests:

```rust
let output = run_console_command(app.world_mut(), "connect local");
assert!(output.success);

let status = run_console_command(app.world_mut(), "net status");
assert!(status.text().contains("connecting-local"));
```

Use queued commands when testing systems that should execute through the normal
app update path:

```rust
app.world_mut()
    .resource_mut::<ConsoleCommandQueue>()
    .push("cvar set net.tick_rate 30");
app.update();
```

`crates/mock-rpg-network-tests` uses this path through `ConsoleNetworkedRpg`:
`connect local` consumes real `ConsoleNetworkRequest`s, starts the actual
Lightyear Crossbeam client/server harness, sends mock player inputs through the
connected client, advances the server, and reads live packet counters through
`net stats`.

## Remaining Wiring

The FPS controller demo no longer consumes `ConsoleNetworkRequest` and no longer
offers `--connect` or `--host` launch modes. Console networking remains exercised
by `crates/mock-rpg-network-tests`; native socket stats and server bind-address
control remain follow-up work for the shared network layer.
