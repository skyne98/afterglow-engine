# Runtime And Plugin API

## Composition

| Item | Purpose |
|---|---|
| `AfterglowRuntimePlugins` | Demo-free runtime group: core, Leafwing input, Lightyear/rewind networking, physics, first-person controller, persistence, and world plugins. |
| `AfterglowEnginePlugin` | App-level engine plugin. Adds `AfterglowRuntimePlugins`, the perf HUD, trace collection, and metrics systems. |
| `demo::AfterglowDemoPlugin` | Optional demo content plugin. Installs the built-in demo cell manifest/load request plus demo animation systems. |
| `run()` | Native/wasm entrypoint. Adds Bevy defaults, `AfterglowEnginePlugin`, and `AfterglowDemoPlugin`. |

## Design Rules

- Engine runtime plugins must not install game/demo content by default.
- Demo content is opt-in through `AfterglowDemoPlugin`.
- `AfterglowEnginePlugin` is the composition root for engine runtime and diagnostics, not gameplay content.
- Feature plugins should own their resources and systems, and tests should verify registration without depending on demo content.

## Current Plugin Order

```text
AfterglowRuntimePlugins
  AfterglowCorePlugin
  AfterglowInputPlugin
  AfterglowNetworkPlugin
  AfterglowPhysicsPlugin
  AfterglowFirstPersonControllerPlugin
  AfterglowPersistencePlugin
  AfterglowWorldPlugin

AfterglowEnginePlugin
  AfterglowRuntimePlugins
  PerfHudPlugin
  metrics/trace update systems

run()
  DefaultPlugins
  AfterglowEnginePlugin
  AfterglowDemoPlugin
```

New networked gameplay should be written against Leafwing action state, Lightyear
replication/prediction/interpolation, and the custom server rewind plugin.

## Demo Plugin

`AfterglowDemoPlugin` inserts the built-in demo manifest into
`CellManifestRegistry`, requests `DEMO_CELL_CHUNK` through `CellLoadRequests`,
and runs `rotate_cubes`/`update_light` in `AfterglowSet::DebugAndMetrics`.
Those systems record perf data when `PerfData` exists, but still run without the
perf HUD in small tests.
