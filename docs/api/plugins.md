# Runtime And Plugin API

## Composition

| Item | Purpose |
|---|---|
| `AfterglowRuntimePlugins` | Demo-free runtime group: core, dev console overlay/core, Lightyear networking, Leafwing input, physics, and first-person controller. |
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
`DefaultPlugins`, so focused and unfocused windows both keep ticking.

New networked gameplay should be written against Leafwing action state,
Lightyear replication/prediction/interpolation, console-emitted network requests,
fixed server input delay, deterministic fixed-tick simulation, and `PreSpawned`
predicted interaction entities.

## Demo Plugin

`AfterglowDemoPlugin` currently installs only the built-in demo animation systems
`rotate_cubes` and `update_light` in `AfterglowSet::DebugAndMetrics`. Those
systems record perf data when `PerfData` exists, but still run without the perf
HUD in small tests. It does not install world-streaming or persistence APIs.

## FPS Demo Plugin

`FpsControllerDemoPlugin` is a local-only first-person controller playground. It
spawns the physics room, stairs, slopes, crouch tunnel, local player controller,
camera rig, and trace logging systems. It does not install FPS-specific network
resources, replicated avatar state, remote avatar presentation, or multiplayer
launch modes.
