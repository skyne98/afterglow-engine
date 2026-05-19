# Runtime And Plugin API

## Composition

| Item | Purpose |
|---|---|
| `AfterglowRuntimePlugins` | Demo-free runtime group: core, dev console overlay/core, Leafwing input, Lightyear/rewind networking, physics, first-person controller, persistence, and world plugins. |
| `AfterglowEnginePlugin` | App-level engine plugin. Adds `AfterglowRuntimePlugins`, the perf HUD, trace collection, and metrics systems. |
| `demo::AfterglowDemoPlugin` | Optional demo content plugin. Installs the built-in demo cell manifest/load request plus demo animation systems. |
| `run()` | Native/wasm entrypoint. Adds Bevy defaults, unthrottled window update settings, `AfterglowEnginePlugin`, and `AfterglowDemoPlugin`. |

## Design Rules

- Engine runtime plugins must not install game/demo content by default.
- Demo content is opt-in through `AfterglowDemoPlugin`.
- `AfterglowEnginePlugin` is the composition root for engine runtime and diagnostics, not gameplay content.
- Feature plugins should own their resources and systems, and tests should verify registration without depending on demo content.

## Current Plugin Order

```text
AfterglowRuntimePlugins
  AfterglowCorePlugin
  DevConsolePlugin
  AfterglowNetworkPlugin
  AfterglowInputPlugin
  AfterglowPhysicsPlugin
  AfterglowFirstPersonControllerPlugin
  AfterglowPersistencePlugin
  AfterglowWorldPlugin

AfterglowEnginePlugin
  AfterglowRuntimePlugins
  PerfHudPlugin
  metrics/trace update systems

run()
  WinitSettings::continuous()
  DefaultPlugins
  AfterglowEnginePlugin
  AfterglowDemoPlugin
```

The native/wasm run helpers insert `WinitSettings::continuous()` before
`DefaultPlugins`, so focused and unfocused windows both keep ticking. Windowed
FPS hosts therefore do not drop into Bevy's default low-power unfocused mode when
the server window loses focus.

New networked gameplay should be written against Leafwing action state,
Lightyear replication/prediction/interpolation, console-emitted network requests,
chunk-interest fanout, and the custom server rewind plugin.

## Demo Plugin

`AfterglowDemoPlugin` inserts the built-in demo manifest into
`CellManifestRegistry`, requests `DEMO_CELL_CHUNK` through `CellLoadRequests`,
and runs `rotate_cubes`/`update_light` in `AfterglowSet::DebugAndMetrics`.
Those systems record perf data when `PerfData` exists, but still run without the
perf HUD in small tests.

## FPS Demo Plugin

`FpsControllerDemoPlugin` installs `FpsDemoNetworkPlugin` before spawning the
visual controller playground. The network plugin defaults to local FPS networking,
consumes console network requests, and exposes `FpsDemoNetworkStatus` for tests
and diagnostics. With the `multiplayer` feature, local launch uses a real
Lightyear Crossbeam server with local clients: visible player input commands
cross the client/server boundary, authoritative avatar state is replicated back,
and non-local avatars are mirrored into the scene. `--connect` launch creates a
native UDP/netcode Lightyear client, while `--host` launch binds a native
UDP/netcode Lightyear server.
