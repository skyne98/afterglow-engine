# Afterglow Engine API Surface

## Workspace

Three crates: `afterglow-engine` (library), `agx` (binary CLI), and
`mock-rpg-network-tests` (test-only mock RPG networking harness).

## Crate: `afterglow-engine`

### Module Hierarchy
```
afterglow-engine
├── lib.rs
├── core/
│   ├── mod.rs
│   ├── identity.rs
│   └── schedule.rs
├── input/
│   ├── mod.rs
│   └── tests.rs       (cfg(test))
├── network/
│   ├── mod.rs
│   ├── commands.rs
│   ├── commands/
│   │   └── tests.rs   (cfg(test))
│   ├── session.rs
│   ├── session/
│   │   └── tests.rs   (cfg(test))
│   └── tests.rs       (cfg(test))
├── world/
│   ├── mod.rs
│   └── chunk.rs
├── setup.rs           (private)
├── testing.rs         (cfg(test) or feature = "test-support")
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
| `input` | Generic per-game input axis/action bindings and per-tick `PlayerCommand` generation. |
| `network` | Transport-independent peer, channel, packet, and fake transport primitives. |
| `network::commands` | Versioned wire envelope for serializing generic `PlayerCommand` batches. |
| `network::session` | Session identity maps between peers, platform identities, players, and avatars. |
| `world` | Chunk/cell loading systems. Currently owns the built-in demo cell loader. |
| `world::chunk` | Chunk IDs, demo-cell load state, and demo-cell loading system. |
| `testing` | Test app builders. Available in unit tests and through the `test-support` feature. |

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

**Update / `ReadInput`:** `collect_player_commands`

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
| `PlayerInputBindings` | Game-configured axis and key/mouse action bindings |
| `LocalPlayers` | Local session player IDs controlled by this app instance |
| `SimulationTick` | Monotonic command tick counter |
| `PlayerCommandQueue` | Current-frame local player commands |
| `NetworkProtocol` | Active engine network protocol version |
| `NetworkSession` | Runtime peer/player/platform/avatar identity map |
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

### Test Commands

| Command | Purpose |
|---|---|
| `cargo test -p afterglow-engine` | Fast unit tests and renderless app tests |
| `cargo test -p afterglow-engine --features test-support` | Unit tests plus real-adapter headless GPU render tests |
| `bun run test` | Runs normal cargo tests, then the `test-support` headless-render test suite |

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
| `InputAction` | String action ID | Game-defined action emitted from raw devices. The engine does not hardcode game actions. |
| `PlayerCommand` | player, tick, axes, actions, pointers | Deterministic command record for local-server simulation, prediction, replay, editor tools, and tests. Payload is game-defined; lobby/menu/no-camera scenes can emit no axes, actions, or pointers. |
| `PlayerInputBindings` | axes, actions | Game-configured keyboard, mouse, gamepad, and touch action bindings. Defaults emit no game-specific input. |
| `InputAxis` | String axis ID | Game-defined analog/digital axis name. |
| `InputAxisValue` | axis, value | One command-time axis sample. |
| `AxisBinding` | axis, source | Maps an axis source to one named axis. |
| `AxisSource` | KeyPair, GamepadAxis, GamepadButtonPair | Built-in axis sources. Touch, tablet, and custom editor controls can feed `VirtualInputState`. |
| `ActionBinding` | input, action | Maps one key, mouse button, gamepad button, or touch press to one game-defined action. |
| `ActionInput` | Key, Mouse, GamepadButton, TouchAny | Raw input source for an action binding. |
| `VirtualInputState` | axes, actions, pointers | Per-frame game/editor-fed inputs for touch virtual sticks, graphics tablet pens, custom devices, and UI tools. Cleared after command collection. |
| `PointerInput` | device, id, position, delta, pressure, tilt, twist, primary | Generic pointer sample for touch, pen/tablet, mouse-derived editor tools, or custom pointing devices. |
| `PointerDevice` | Mouse, Touch, Pen, Unknown | Pointer source classification. |
| `LocalPlayers` | peer, players | Local transport peer plus one or more local `NetworkPlayerId`s controlled by this app instance. Stable world-entity mapping lives outside input. |
| `SimulationTick` | u32 | Monotonic command tick. |
| `PlayerCommandQueue` | commands | Current-frame generated commands. |
| `NetworkProtocol` | version | Resource exposing the current network protocol version. |
| `ProtocolVersion` | major, minor, patch | Semver-style protocol version used in packet headers. |
| `PeerId` | u64 | Transport-session peer identifier. |
| `NetworkPlayerId` | u64 | Session-level player identity, separate from platform IDs and stable entity IDs. |
| `NetChannel` | Control, Commands, Snapshots, Events, Bulk, Custom | Engine packet channel classification. |
| `DeliveryMode` | Reliable, Unreliable, UnreliableSequenced | Delivery intent independent of any transport backend. |
| `PacketHeader` | protocol, channel, delivery, sequence | Transport-independent packet metadata. |
| `NetworkPacket` | from, to, header, payload | Transport-independent packet envelope. |
| `DisconnectReason` | Local, Remote, Timeout, ProtocolMismatch, Transport | Generic disconnect reason. |
| `TransportEvent` | Connected, Disconnected, Packet | Events emitted by transport backends. |
| `NetworkTransport` | trait | Minimal transport interface for polling events, sending packets, and disconnecting peers. |
| `MemoryTransport` | local_peer, protocol, queues, faults | Deterministic in-memory fake transport for unit tests and protocol development. |
| `FaultConfig` | drop_every, duplicate_every, delay_ticks, reverse_delivery | Deterministic packet fault injection for fake transport tests. |
| `CommandEnvelope` | protocol, commands | Versioned wire payload for one batch of player commands. |
| `WirePlayerCommand` | player, tick, axes, actions, pointers | Explicit transport DTO for `PlayerCommand`. |
| `WireAxisValue` | axis, value | Explicit transport DTO for one named axis value. |
| `WirePointerInput` | device, id, position, delta, pressure, tilt, twist, primary | Explicit transport DTO for pointer input. |
| `CommandDecodeError` | InvalidJson, ProtocolMismatch | Decode failure for malformed or incompatible command payloads. |
| `encode_player_commands()` | — | Serializes `PlayerCommand` values into a versioned command envelope. |
| `decode_player_commands()` | — | Deserializes and validates a versioned command envelope. |
| `NetworkSession` | next_player, peers, players | Runtime map from transport peers to platform identities, network players, and optional avatar stable IDs. |
| `PeerSession` | peer, platform, players | One connected transport peer and the local/splitscreen players it owns. |
| `PlayerSession` | player, peer, avatar | One session player, its owning peer, and optional controlled stable world entity. |
| `PlatformIdentity` | Local, Steam, Iroh, Anonymous | Backend-neutral authenticated identity descriptor. |
| `testing::unit_app()` | — | Builds a minimal non-rendering app with `AfterglowCorePlugin`. |
| `testing::asset_unit_app()` | — | Builds a minimal non-rendering app with assets and core systems. |
| `testing::headless_render::app()` | — | `test-support` only. Builds a no-window render app using a real GPU adapter, or returns `None` if unavailable. |
| `testing::headless_render::offscreen_texture()` | — | `test-support` only. Creates an offscreen render target suitable for GPU readback tests. |
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
| wgpu | 27 | optional, `test-support` only for real headless GPU tests |
| serde | 1 | derive |
| serde_json | 1 | — |
| tracing | 0.1 | — |
| tracing-subscriber | 0.3 | env-filter |
| tiny_http | 0.12 | — |
| web-time | 1 | wasm-compatible timing |

## Crate: `agx` (binary)

CLI entry point. Parses `--name` arg (unused), calls `afterglow_engine::run()`.

## Crate: `mock-rpg-network-tests`

Workspace test crate that exercises engine networking primitives against a mock
first-person open-world RPG simulation. It uses `MemoryTransport` and JSON
packet payloads to cover multi-client joins, 3D position/chunk math, snapshots,
chunk interest, authority, duplicate ticks, rollback-style save/restore,
malformed/spoofed/hacked client behavior, door use, item pickup conflicts,
combat events, and many simultaneous NPC/world-state changes.

## Docs

- `docs/api/` — this file
- `docs/research/` — design notes, benchmarks, investigations
- `docs/ROADMAP.md` — project vision and milestones
