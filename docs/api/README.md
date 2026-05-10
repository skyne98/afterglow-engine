# Afterglow Engine API Surface

## Workspace

Two crates: `afterglow-engine` (library) and `agx` (binary CLI).

## Crate: `afterglow-engine`

### Module Hierarchy
```
afterglow-engine
├── lib.rs
├── core/
│   ├── mod.rs
│   ├── identity.rs
│   └── schedule.rs
├── world/
│   ├── mod.rs
│   └── chunk.rs
├── setup.rs           (private)
└── perf_hud/
    ├── mod.rs
    ├── data.rs         (private)
    ├── server.rs       (private)
    ├── ui.rs           (private)
    └── trace_collector.rs (pub submodule)
```

### Top-Level Exports

| Item | Kind | Description |
|---|---|---|
| `AfterglowEnginePlugin` | struct impl Plugin | Main engine plugin. Builds scene, registers HUD, tracing, metrics. |
| `AfterglowEnginePlugin::trace_accum` | field: AccumMap | Shared accumulator for tracing span data |
| `run()` | fn → AppExit | Creates App with DefaultPlugins + AfterglowEnginePlugin, runs it |

### Re-exported Public Modules

| Module | Description |
|---|---|
| `core` | Engine foundation systems. Currently owns stable identity and chunk membership. |
| `core::identity` | Stable entity IDs, chunk IDs, persistence/replication markers, and registry resources. |
| `core::schedule` | Ordered engine system sets for input, command building, simulation, persistence prep, and debug/metrics. |
| `world` | Chunk/cell loading systems. Currently owns the built-in demo cell loader. |
| `world::chunk` | Chunk IDs, demo-cell load state, and demo-cell loading system. |

### Re-exported from `perf_hud`

| Item | Kind | Source |
|---|---|---|
| `PerfData` | Resource | data.rs |
| `SharedMetrics` | Resource (Arc<Mutex<PerfData>>) | server.rs |
| `setup_tracing()` | fn → TraceData | trace_collector.rs |
| `AccumMap` | type alias | trace_collector.rs |
| `update_hud()` | system fn | ui.rs |

### Systems Registered (execution order)

**Startup:** `load_demo_cell`, `spawn_hud`

**Update sets:** `ReadInput` → `BuildCommands` → `Simulate` → `ApplyGameplay` → `PreparePersistence` → `DebugAndMetrics`

**Update / `DebugAndMetrics` (chained):**
1. `record_update_start`
2. `rotate_cubes`
3. `update_light`
4. `collect_frame`
5. `update_hud`
6. `record_update_end`
7. `sync_shared_metrics`

**Update (after sync):** `reset_trace_data`

**PostUpdate:** `maintain_stable_entity_registry`

### Resources

| Resource | Type |
|---|---|
| `TraceData` | `{ accum: AccumMap }` |
| `StableIdAllocator` | Monotonic allocator for process-local stable IDs |
| `StableEntityRegistry` | Runtime maps for stable ID ↔ entity and chunk → entities |
| `DemoCellState` | Tracks the built-in demo cell chunk and whether it has been loaded |
| `PerfData` | History, frame systems, trace snapshots, timing, name colors |
| `FrameProfiler` | `{ update_start: Option<Instant>, postupdate_start: Option<Instant> }` |
| `SharedMetrics` | `Arc<Mutex<PerfData>>` for HTTP server |

### Environment Variables

| Variable | Default | Purpose |
|---|---|---|
| `AGX_METRICS_PORT` | `9877` | HTTP metrics server port |
| `RUST_LOG` | `"info"` | Tracing filter |

### Platform Notes

- Native builds attach the custom trace collector through Bevy's `LogPlugin`,
  keeping a single tracing subscriber for stderr logs and in-memory span metrics.
- The engine no longer enables Bevy's `trace_chrome` output, so normal runs do
  not emit `trace-*.json` files.
- Wasm builds attach the same collector to Bevy's wasm logging stack.
- Wasm builds disable Bevy's anti-alias plugin and omit camera TAA because the
  Bevy 0.18 TAA pipeline fails WebGL validation in browser builds.
- Wasm builds disable Bevy's audio plugin until the engine exposes an explicit
  user-gesture driven audio startup flow.
- The HTTP metrics server is native-only; wasm builds keep the shared metrics
  resource but do not bind a socket.

### HTTP API

- `GET /metrics` or `GET /` → JSON: FPS stats, frame time stats, top 15 trace spans
- `GET /traces` → JSON: same but all trace spans (untruncated)

### Components

| Component | Module | Purpose |
|---|---|---|
| `StableEntityId(u128)` | core::identity | Durable ID for save/load and replication. Raw Bevy `Entity` must not be serialized or replicated. |
| `ChunkId(u64)` | core::identity | Stable chunk identifier. |
| `ChunkMembership { chunk }` | core::identity | Assigns an entity to a chunk for residency, save/load, and interest management. |
| `Persistent` | core::identity | Entity should receive/keep a stable ID and participate in persistence. |
| `Replicated` | core::identity | Entity should receive/keep a stable ID and participate in replication. |
| `RuntimeOnly` | core::identity | Entity is excluded from automatic stable ID assignment. |
| `Rotates { speed: f32 }` | setup | Marker + rotation speed |
| `HudRoot` | ui | HUD container |
| `FpsText` | ui | FPS text label |
| `FrameTimeText` | ui | Frame time text label |
| `FrameBar` | ui | Frame-time bar segment |
| `TraceHistBar` | ui | Trace history bar container |
| `TraceSeg` | ui | Trace bar segment |
| `SysLegendItem` | ui | System name legend |
| `BarLerp(f32)` | ui | Smoothing state for bar height |

### Data Types

| Type | Fields | Description |
|---|---|---|
| `AfterglowSet` | ReadInput, BuildCommands, Simulate, ApplyGameplay, PreparePersistence, DebugAndMetrics | Ordered engine system sets for deterministic feature layering. |
| `StableIdAllocator` | next | Allocates nonzero `StableEntityId` values for persistent/replicated entities missing one. |
| `StableEntityRegistry` | stable_to_entity, entity_to_stable, chunk_to_entities, duplicate_ids | Rebuilt after updates from entities with `StableEntityId` and optional `ChunkMembership`. |
| `DemoCellState` | chunk, load_state | Resource used by the demo loader to spawn one stable-ID chunk once. |
| `ChunkLoadState` | Unloaded, Loaded | Minimal chunk load state for the built-in demo cell. |
| `FrameSample` | fps, frame_time_ms, systems | One frame's metrics |
| `PerfData` | history, frame_systems, trace_snapshots, update_time_ms, extraction_time_ms, name_colors, next_color, smoothed_trace_max | Full performance data store |
| `SystemStats` | name, avg, p95, p99 | Per-system timing stats |
| `SpanSample` | name, duration_ms, count | Trace span measurement |
| `SharedMetrics` | `Arc<Mutex<PerfData>>` | Thread-safe share for HTTP server |

### Dependencies

| Crate | Version | Feature |
|---|---|---|
| bevy (workspace) | 0.18.1 | webgpu |
| bevy (native engine) | 0.18.1 | bevy_dev_tools, trace |
| bevy (native agx) | 0.18.1 | dynamic_linking, bevy_dev_tools, sysinfo_plugin, trace |
| serde | 1 | derive |
| serde_json | 1 | — |
| tracing | 0.1 | — |
| tracing-subscriber | 0.3 | env-filter |
| tiny_http | 0.12 | — |
| web-time | 1 | wasm-compatible timing |

## Crate: `agx` (binary)

CLI entry point. Parses `--name` arg (unused), calls `afterglow_engine::run()`.

## Docs

- `docs/api/` — this file
- `docs/research/` — design notes, benchmarks, investigations
- `docs/ROADMAP.md` — project vision and milestones
