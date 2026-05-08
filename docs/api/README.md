# Afterglow Engine API Surface

## Workspace

Two crates: `afterglow-engine` (library) and `agx` (binary CLI).

## Crate: `afterglow-engine`

### Module Hierarchy
```
afterglow-engine
├── lib.rs
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

### Re-exported from `perf_hud`

| Item | Kind | Source |
|---|---|---|
| `PerfData` | Resource | data.rs |
| `SharedMetrics` | Resource (Arc<Mutex<PerfData>>) | server.rs |
| `setup_tracing()` | fn → TraceData | trace_collector.rs |
| `AccumMap` | type alias | trace_collector.rs |
| `update_hud()` | system fn | ui.rs |

### Systems Registered (execution order)

**Startup:** `spawn_scene`, `spawn_hud`

**Update (chained):**
1. `record_update_start`
2. `rotate_cubes`
3. `update_light`
4. `collect_frame`
5. `update_hud`
6. `record_update_end`
7. `sync_shared_metrics`

**Update (after sync):** `reset_trace_data`

**PostUpdate (chained):** `record_postupdate_start` → `record_postupdate_end`

### Resources

| Resource | Type |
|---|---|
| `TraceData` | `{ accum: AccumMap }` |
| `PerfData` | History, frame systems, trace snapshots, timing, name colors |
| `FrameProfiler` | `{ update_start: Option<Instant>, postupdate_start: Option<Instant> }` |
| `SharedMetrics` | `Arc<Mutex<PerfData>>` for HTTP server |

### Environment Variables

| Variable | Default | Purpose |
|---|---|---|
| `AGX_METRICS_PORT` | `9877` | HTTP metrics server port |
| `RUST_LOG` | `"info"` | Tracing filter |

### HTTP API

- `GET /metrics` or `GET /` → JSON: FPS stats, frame time stats, top 15 trace spans
- `GET /traces` → JSON: same but all trace spans (untruncated)

### Components

| Component | Module | Purpose |
|---|---|---|
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
| `FrameSample` | fps, frame_time_ms, systems | One frame's metrics |
| `PerfData` | history, frame_systems, trace_snapshots, update_time_ms, extraction_time_ms, name_colors, next_color, smoothed_trace_max | Full performance data store |
| `SystemStats` | name, avg, p95, p99 | Per-system timing stats |
| `SpanSample` | name, duration_ms, count | Trace span measurement |
| `SharedMetrics` | `Arc<Mutex<PerfData>>` | Thread-safe share for HTTP server |

### Dependencies

| Crate | Version | Feature |
|---|---|---|
| bevy | 0.18.1 | dynamic_linking, bevy_dev_tools, sysinfo_plugin, trace, trace_chrome |
| serde | 1 | derive |
| serde_json | 1 | — |
| tracing | 0.1 | — |
| tracing-subscriber | 0.3 | env-filter |
| tiny_http | 0.12 | — |

## Crate: `agx` (binary)

CLI entry point. Parses `--name` arg (unused), calls `afterglow_engine::run()`.

## Docs

- `docs/api/` — this file
- `docs/research/` — design notes, benchmarks, investigations
- `docs/ROADMAP.md` — project vision and milestones
