# Afterglow Engine API Surface

## Workspace

Four crates: `afterglow-engine` (library), `afterglow-engine-macros`
(proc macros), `agx` (binary CLI), and `mock-rpg-network-tests` (test-only mock
RPG networking harness).

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
│   ├── authority.rs
│   ├── authority/
│   │   └── tests.rs   (cfg(test))
│   ├── baseline.rs
│   ├── baseline/
│   │   └── tests.rs   (cfg(test))
│   ├── commands.rs
│   ├── commands/
│   │   └── tests.rs   (cfg(test))
│   ├── interest.rs
│   ├── interest/
│   │   └── tests.rs   (cfg(test))
│   ├── interpolation.rs
│   ├── interpolation/
│   │   └── tests.rs   (cfg(test))
│   ├── prediction.rs
│   ├── prediction/
│   │   └── tests.rs   (cfg(test))
│   ├── reconciliation.rs
│   ├── reconciliation/
│   │   └── tests.rs   (cfg(test))
│   ├── replication.rs
│   ├── replication/
│   │   ├── ecs.rs
│   │   ├── ecs_edge_tests.rs (cfg(test))
│   │   ├── history.rs
│   │   ├── rollback.rs
│   │   ├── rollback_ecs_tests.rs (cfg(test))
│   │   ├── runtime.rs
│   │   ├── schedule.rs
│   │   ├── timeline_tests.rs (cfg(test))
│   │   ├── world_state.rs
│   │   ├── world_state_tests.rs (cfg(test))
│   │   └── tests.rs   (cfg(test))
│   ├── rollback.rs
│   ├── rollback/
│   │   ├── messages.rs
│   │   ├── messages/
│   │   │   └── tests.rs (cfg(test))
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

### Crate: `afterglow-engine-macros`

| Item | Kind | Description |
|---|---|---|
| `#[derive(Replicate)]` | derive macro | Implements `network::replication::Replicate` for replicated state-bearing components/resources. |
| `#[replicate]` | attribute macro | Attribute form of the same marker implementation for replicated state-bearing components/resources. |

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
| `network` | Transport-independent peer, channel, packet, shared session, and fake transport primitives. Real network libraries are expected to be thin backend adapters below this layer. |
| `network::authority` | Server-side command validation for peer ownership and duplicate simulation ticks. |
| `network::baseline` | Replication-compatible save data and reconnect baseline helpers. |
| `network::commands` | Versioned wire envelope for serializing generic `PlayerCommand` batches. |
| `network::handshake` | Backend-neutral reliable control handshake, compatibility validation, authenticated identity admission, and gameplay packet gating. |
| `network::interest` | Chunk-based interest map for filtering snapshots and deltas by player visibility. |
| `network::interpolation` | Remote entity sample buffering, interpolation, and bounded extrapolation. |
| `network::prediction` | Client-side command history and replay buffer for prediction after authoritative snapshots. |
| `network::reconciliation` | Reconciles authoritative snapshot/delta/correction ticks with local prediction history. |
| `network::replication` | Stable-ID keyed snapshot/delta primitives plus Bevy-facing replicated components, resources, and tick-addressed command/message timelines. |
| `network::rollback` | Small deterministic subsystem rollback history plus committed/provisional domain replay, lifecycle, and cue helpers. |
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

**Update / `BuildCommands`:** `clear_server_command_buffer`

**Update / `BuildCommands`:** `clear_reconciliation_queue`

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
| `ReconnectBaselineStore` | Per-peer/player reconnect baselines built from replication snapshots |
| `DeterministicRollbackBuffer` | Tick-indexed history for small deterministic subsystem rollback |
| `ServerCommandBuffer` | Per-frame accepted/rejected server-authoritative command buffer plus tick dedupe state |
| `ClientPredictionBuffer` | Local command history used to replay prediction after authoritative snapshots |
| `ClientReconciliationQueue` | Per-frame reconciliation results created from authoritative updates |
| `RemoteInterpolationBuffer` | Buffered remote entity samples for rendering remote entities smoothly behind server time |
| `InterestMap` | Chunk visibility map for players and replicated entities |
| `RollbackReplicationClock` | Current rollback tick and committed/provisional policy |
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

### Benchmark Commands

| Command | Purpose |
|---|---|
| `cargo bench -p afterglow-engine --bench replication` | Measures replication snapshot, delta, apply, and interest-filter costs at multiple entity counts |
| `cargo bench -p afterglow-engine --bench authority` | Measures bulk server-authoritative command validation and duplicate tick rejection |
| `cargo bench -p afterglow-engine --bench prediction` | Measures client prediction command recording and replay/rebase costs |
| `cargo bench -p afterglow-engine --bench reconciliation` | Measures authoritative correction reconciliation against local prediction history |
| `cargo bench -p afterglow-engine --bench interpolation` | Measures remote entity interpolation and bounded extrapolation costs |
| `cargo bench -p afterglow-engine --bench baseline` | Measures replication save serialization, restore, and interest-filtered reconnect baseline costs |
| `cargo bench -p afterglow-engine --bench rollback` | Measures deterministic subsystem state history, policy, and replay costs |
| `cargo bench -p afterglow-engine --bench ggrs` | Measures GGRS rollback-session coordinator cost plus synthetic full-state save/load/replay pressure |

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
| `DeliveryMode` | Reliable, Unreliable, UnreliableSequenced | Delivery intent independent of any transport backend. `MemoryTransport` drops stale/duplicate `UnreliableSequenced` packets per peer/channel and resets that sequence state on reconnect. |
| `PacketHeader` | protocol, channel, delivery, sequence | Transport-independent packet metadata. |
| `NetworkPacket` | from, to, header, payload | Transport-independent packet envelope. |
| `DisconnectReason` | Local, Remote, Timeout, ProtocolMismatch, Transport | Generic disconnect reason. |
| `TransportEvent` | Connected, Disconnected, Packet | Events emitted by transport backends. |
| `NetworkTransport` | trait | Minimal backend boundary for polling connection/packet events, sending engine packets, and disconnecting peers. Iroh, Steam, memory, and future transports should adapt to this trait without owning replication, rollback, prediction, command validation, or interest logic. |
| `MemoryTransport` | local_peer, protocol, queues, faults | Deterministic in-memory fake transport for unit tests and protocol development. Supports fault injection and protocol override for compatibility/rejection tests. |
| `FaultConfig` | drop_every, duplicate_every, delay_ticks, reverse_delivery | Deterministic packet fault injection for fake transport tests. |
| `NetworkHandshakeConfig` | protocol, build_hash, content_hash, backend, identity | Local control-handshake configuration shared by memory, Iroh, Steam, and future backends. |
| `NetworkBackendKind` | Memory, Iroh, Steam, Custom | Backend label carried in the control hello for diagnostics and policy. |
| `ControlMessage` | Hello, Accepted, Rejected | Reliable control-channel handshake payload. Gameplay packets are forwarded only after a peer's hello has been accepted into `NetworkSession`; accepted responses are valid only for already-admitted peers. |
| `ControlHello` | protocol, build_hash, content_hash, backend, identity | Compatibility and identity claim sent immediately after a raw transport connection. |
| `HandshakeRejectReason` | InvalidControlPayload, ProtocolMismatch, BuildMismatch, ContentMismatch, DuplicateIdentity, PeerIdentityChanged | Backend-neutral reason for refusing a peer before gameplay packets are accepted. Protocol mismatch is checked on control and gameplay packet headers plus control hello payloads. A connected peer cannot change platform identity by sending another hello. |
| `HandshakeReport` | sent_hellos, accepted_peers, rejected_peers, disconnected_peers, dropped_unauthorized_packets | Per-service-call diagnostics returned by `service_control_handshake()`. |
| `service_control_handshake()` | — | Polls a `NetworkTransport`, sends reliable hellos/rejects/accepts, updates `NetworkSession`, forwards authorized gameplay events, and drops pre-handshake gameplay packets. |
| `ReplicationSaveData` | tick, snapshot | Serializable save payload built from a `ReplicationWorld` snapshot and restorable back into `ReplicationWorld`. |
| `ReconnectBaseline` | peer, player, snapshot | Full or interest-filtered authoritative snapshot used when a player reconnects. |
| `ReconnectBaselineStore` | baselines | Runtime map of reconnect baselines keyed by `(PeerId, NetworkPlayerId)`. |
| `DeterministicRollbackBuffer` | max_saved_ticks, states | Opaque byte-state history for small deterministic subsystems, such as combat bubbles or puzzle mechanisms. |
| `CommittedRollbackDomain` | id, policy, committed/provisional state, commands, outputs | Authoritative rollback domain. Gameplay reads provisional state; committed state is the durable rollback anchor. |
| `RollbackDomainId` | u64 | Stable identifier for a rollback domain, combat bubble, cell subsystem, or deterministic puzzle. |
| `RollbackMessageId` | domain, tick, sequence | Stable message identity for retained rollback message streams. |
| `RollbackMessage<T>` | id, source_command_tick, entities, payload | Typed replay-generated gameplay fact. Use provisional messages for live/visual correction and committed messages for durable business logic. |
| `RollbackMessageStream<T>` | provisional, committed, committed_tick | Retained message log with a monotonic committed horizon. Replacing provisional messages ignores already committed ticks; committing through a tick promotes final facts without allowing committed facts to be rewritten. |
| `RollbackMessageDiff<T>` | added, removed | Provisional message diff for presentation and correction-aware readers. `removed` carries message IDs so cancellation does not clone old payloads. |
| `RollbackCommit<T>` | committed_tick, added | Messages newly promoted into the final committed stream. |
| `RollbackCommand` | tick, source, sequence, payload | Opaque deterministic command payload for subsystem replay; `(tick, source, sequence)` is the stable replay ordering and duplicate key. |
| `RollbackReplay` | from_tick, to_tick, initial_state, commands | Replay plan built from a saved authoritative state and commands after that tick. `build_replay()` returns `MissingState` when the anchor snapshot is absent and `DuplicateCommand` when the same `(tick, source, sequence)` has conflicting payloads. Late replay requires a saved state before the command tick, so tick-0 late replay currently returns `MissingState`. |
| `RollbackPolicy` | max_rollback_ticks, commit_delay_ticks | Server/authority policy for accepting late commands and deriving the committed/provisional tick boundary. |
| `RollbackCommandDecision` | Replay, TooOld, FromFuture | Result of classifying a late deterministic command against current tick. |
| `RollbackReplayError` | TooOld, FromFuture, MissingState, AlreadyCommitted, DuplicateCommand | Failure reason when building or inserting a policy-gated replay command. |
| `RollbackCue` | tick, sequence, kind, payload | Replay-generated cue fact for UI/audio/VFX or other presentation layers. |
| `RollbackCueDiff` | added, removed | Difference between previous replay cues and corrected replay cues. |
| `RollbackDomainOutputs` | cues, lifecycles | Replay output produced from deterministic command application. |
| `RollbackDomainReplay` | committed_tick, current_tick, previous_provisional_state, provisional_state, cue_diff, outputs | Result of rebuilding or promoting a committed/provisional domain. |
| `RollbackEntityLifecycle` | entity, spawn_tick, despawn_tick, despawn_reason | Rollback-friendly stable entity lifetime record for provisional spawns/despawns. |
| `replay_bytes()` | — | Helper that applies rollback commands to a byte-state clone. |
| `ServerCommandBuffer` | accepted, rejected, seen_ticks | Server-authoritative command intake. Validates peer ownership through `NetworkSession`, rejects duplicate player ticks, and exposes accepted generic `PlayerCommand`s for simulation. |
| `AuthoritativePlayerCommand` | peer, command | Accepted command tagged with the transport peer that submitted it. |
| `RejectedPlayerCommand` | peer, player, tick, reason | Rejected command metadata for logging, metrics, disconnect policy, or client correction. |
| `CommandRejectReason` | UnknownPlayer, PlayerNotOwned, DuplicateTick | Generic command authority rejection reason. |
| `CommandAuthorityResult` | Accepted, Rejected | Result returned by `ServerCommandBuffer::submit`. |
| `ClientPredictionBuffer` | commands, acknowledged | Per-player local command history for client prediction. Games apply commands immediately, then call `replay_after` when an authoritative snapshot/correction arrives. |
| `PredictionReplay` | player, authoritative_tick, commands | Ordered unacknowledged commands to replay on top of authoritative state. |
| `ClientReconciliationQueue` | results | Per-frame results from reconciling authoritative updates against local prediction history. |
| `AuthoritativeCorrection` | player, tick, source | Generic correction packet metadata. Carries the authoritative tick; game-specific state stays in snapshots/deltas or game packets. |
| `AuthoritativeUpdateSource` | Snapshot, Delta, Correction | Source classification for reconciliation metrics and client policy. |
| `ReconciliationResult` | player, authoritative_tick, source, replay_commands | Commands the game should replay after applying authoritative state. |
| `RemoteInterpolationBuffer` | delay_ticks, max_extrapolation_ticks, samples | Per-entity remote sample buffer. Renders behind latest server tick for interpolation and extrapolates only within a configured tick limit. |
| `RemoteEntitySample` | fields | Generic scalar sample fields such as `pos_x`, `pos_y`, `pos_z`, yaw, or animation blend values. |
| `SmoothedEntitySample` | entity, tick, mode, fields | Result returned to game/render systems for a remote entity at render time. |
| `SmoothingMode` | Exact, Interpolated, Extrapolated | Classifies how a remote sample was produced. |
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
| `PlatformIdentity` | Local, Steam, Iroh, Anonymous | Backend-neutral authenticated identity descriptor. External identities map to `PeerId`, then `NetworkPlayerId`, then optional avatar `StableEntityId`. |
| `ReplicationWorld` | entities | Generic stable-ID keyed replicated state map used to build snapshots and deltas. |
| `ReplicatedEntityState` | fields | Byte-valued field map for one replicated entity. |
| `WorldSnapshot` | tick, entities | Full replication baseline. |
| `WorldDelta` | from_tick, to_tick, changes, removed | Delta from one snapshot to a later state. |
| `EntitySnapshot` | entity, fields | Full entity state inside a snapshot. |
| `EntityDelta` | entity, changed, removed | Per-entity changed and removed fields. |
| `FieldValue` | name, value | One byte-valued replicated field. |
| `Replicate` | trait/macro | Marker for components/resources that are part of the replicated truth schema. Implement with `#[derive(Replicate)]` or `#[replicate]`. |
| `ReplicatedCommand` | trait | Bevy `Message` command accepted into the replicated timeline; exposes its simulation tick. |
| `ReplicatedMessage` | trait | Bevy `Message` fact accepted into the replicated timeline; exposes its simulation tick. |
| `ReplicationAppExt::replicate(...)` | app extension | Idempotently registers replicated components, resources, command messages, or timeline messages. |
| `component::<T>()` | registration helper | Registers replicated component type `T: Component + Replicate + Clone`. |
| `resource::<T>()` | registration helper | Registers replicated resource type `T: Resource + Replicate + Clone`. |
| `command::<T>()` | registration helper | Registers replicated command message type `T: ReplicatedCommand`. |
| `message::<T>()` | registration helper | Registers replicated message/fact type `T: ReplicatedMessage`. |
| `ReplicatedComponentState<T>` | resource | Latest collected replicated component values keyed by `StableEntityId`. |
| `ReplicatedResourceState<T>` | resource | Latest collected replicated resource value. |
| `ReplicatedComponentHistory<T>` | resource | Full tick snapshots of registered replicated component values keyed by valid `StableEntityId`; save/restore canonicalizes duplicate IDs and uses these snapshots as rollback anchors. |
| `ReplicatedResourceHistory<T>` | resource | Full tick snapshots of registered replicated resources, including absence. |
| `ReplicatedTimeline<T>` | resource | Tick-addressed bounded command/message timeline; rollback replay replaces and reissues retained messages while dropping stale out-of-order ticks. |
| `ReplicatedTick` | schedule | Dedicated schedule for replicated gameplay truth. Game code adds normal Bevy systems here; rollback can restore state, reissue messages, and run it repeatedly. |
| `ReplicatedRollbackWorldExt` | world extension | Adds `save_replicated_state(tick)`, `restore_replicated_state(tick)`, `run_replicated_tick(tick)`, and `replay_replicated_ticks(anchor, through)`; save/restore maintains stable IDs before touching snapshots. |
| `ReplicatedRollbackError` | InvalidRange, MissingSnapshot | Error returned by replicated ECS restore/replay helpers. |
| `ReplicationSet` | RestoreState, ReissueMessages, CollectMessages, CollectChanges | Ordered `Update` sets inside `AfterglowSet::ApplyGameplay`; replay reissues messages before gameplay collection snapshots changed replicated state. |
| `RollbackReplicationClock` | current_tick, policy | Drives the committed/provisional boundary for replay-aware replicated timelines. |
| `ReplicatedRollbackMessageStream<E>` | messages, last diff/commit | Retained committed/provisional rollback message stream wrapper for replay-produced facts. |
| `InterestMap` | entity_chunks, player_chunks | Chunk-based visibility filter for replicated snapshots and deltas. |
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
| ggrs | 0.12.0 | dev-only benchmark dependency |
| proc-macro-crate | 3 | macro crate-name resolution for renamed dependencies |
| proc-macro2 | 1 | proc macro support |
| quote | 1 | proc macro support |
| wgpu | 27 | optional, `test-support` only for real headless GPU tests |
| serde | 1 | derive |
| serde_json | 1 | — |
| syn | 2 | proc macro parsing, full |
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
combat messages, client prediction, reconciliation, remote interpolation,
bounded extrapolation, moving spell projectiles with collider hits under
delayed/reordered packets, projectile edge cases for duplicate/spoofed casts,
packet loss, swept collision, rejected prediction cleanup, out-of-order samples,
late shield correction that rewrites provisional death outcomes through
replicated ECS replay, and many simultaneous NPC/world-state changes.

## Docs

- `docs/api/` — this file
- `docs/research/` — design notes, benchmarks, investigations
- `docs/ROADMAP.md` — project vision and milestones
