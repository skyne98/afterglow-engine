# Lightyear And Leafwing Input Manager

## Summary

Lightyear is a Bevy-native networking stack that handles Bevy's multi-threaded
runtime by staying inside Bevy's schedule model. It does not ask users to run a
separate gameplay thread or manually lock the ECS world. Instead, it partitions
network work into Bevy schedules, orders correctness-sensitive systems with
`SystemSet`s and `.chain()`, and relies on Bevy's ECS access analysis to run safe
systems in parallel.

For Afterglow, this is now the target multiplayer substrate, not just reference
material. The old custom networking/input stack was deleted and replaced with
Lightyear, Leafwing, and `lightyear_inputs_leafwing`. Fixed server input delay
replaces the old server rewind approach. Input buffering, snapshot receive,
prediction, rollback,
fixed-step simulation, and packet send must each have explicit ordering points.

## Sources

| Topic | Source |
|---|---|
| Crate landing page | https://crates.io/crates/lightyear |
| API docs | https://docs.rs/lightyear/latest/lightyear/ |
| Book introduction | https://cbournhonesque.github.io/lightyear/book/ |
| Bevy system ordering | https://cbournhonesque.github.io/lightyear/book/concepts/bevy_integration/system_order.html |
| Input handling | https://cbournhonesque.github.io/lightyear/book/concepts/advanced_replication/inputs.html |
| Leafwing adapter docs | https://docs.rs/lightyear_inputs_leafwing/latest/lightyear_inputs_leafwing/ |
| Client plugin source | https://raw.githubusercontent.com/cBournhonesque/lightyear/main/lightyear/src/client.rs |
| Shared plugin source | https://raw.githubusercontent.com/cBournhonesque/lightyear/main/lightyear/src/shared.rs |
| Generic input client source | https://raw.githubusercontent.com/cBournhonesque/lightyear/main/lightyear_inputs/src/client.rs |
| Generic input server source | https://raw.githubusercontent.com/cBournhonesque/lightyear/main/lightyear_inputs/src/server.rs |
| Leafwing adapter source | https://raw.githubusercontent.com/cBournhonesque/lightyear/main/lightyear_inputs_leafwing/src/plugin.rs |
| Prediction manager source | https://raw.githubusercontent.com/cBournhonesque/lightyear/main/lightyear_prediction/src/manager.rs |

## Bevy Multi-Threading Model

Bevy can execute systems in parallel when their ECS access is compatible.
Lightyear accepts that model rather than centralizing all networking in a single
monolithic system. The critical control mechanism is schedule ordering:

| Phase | Lightyear behavior |
|---|---|
| `PreUpdate` | Receive packets/messages and apply replicated components before game fixed logic. |
| `FixedPreUpdate` | Buffer or restore input for the tick that fixed simulation will consume. |
| `FixedUpdate` / `FixedMain` | User simulation, physics, and rollback-safe gameplay logic. |
| `FixedPostUpdate` | Restore delayed input and update prediction history after simulation. |
| `PostUpdate` | Prepare and send messages, send replication updates, cleanup old buffers. |

The Lightyear book states that packets are read in `PreUpdate` and sent in
`PostUpdate`. It also exposes ordering sets such as `BufferInputs` and `Main` so
game systems can be placed where prediction and rollback expect them.

The source follows the same pattern. `ClientPlugins` and `ServerPlugins` compose
smaller plugins. `SharedPlugins` installs core time/timeline, transport,
messages, connection, replication, and interpolation plugins once. This keeps
network responsibilities split into Bevy plugins instead of one external worker
that mutates game state outside the ECS scheduler.

## Correctness Under Parallel Execution

Lightyear uses several Bevy-native tools to prevent multi-threaded races:

| Tool | Purpose |
|---|---|
| `SystemSet` labels | Give users and plugins stable insertion points. |
| `.before()` / `.after()` | Order dependent systems without serializing unrelated work. |
| `.chain()` | Force local sequences where each step depends on the previous step. |
| ECS component/resource borrows | Let Bevy reject unsafe parallel access and run compatible systems concurrently. |
| `run_if` | Keep prediction systems inactive unless a client is connected and eligible. |
| Atomics / `RwLock` | Share small cross-system prediction flags without exclusive ECS access everywhere. |

The input client plugin is a good example. It chains
`WriteClientInputs -> BufferClientInputs` in `FixedPreUpdate`, chains input
message preparation, sync, send, cleanup, and message send in `PostUpdate`, and
places remote input receive before the fixed main loop. That ordering is doing
the thread-safety and determinism work: Bevy can still parallelize independent
systems, but network-sensitive edges are explicit.

Lightyear also avoids over-parallelizing some hot paths until semantics are
clear. The server input receiver has a `TODO` noting possible `par_iter_mut`, but
currently uses serial iteration while it drains per-client input messages,
rebroadcasts them, resolves target entities, and mutates input buffers. That is a
reasonable tradeoff: preserving packet ordering and ECS mutation semantics is
more important than speculative parallelism in the receive path.

Prediction is the main place where Lightyear adds synchronization primitives.
`PredictionManager` stores rollback state in a `parking_lot::RwLock`, and stores
earliest mismatched input and last confirmed input with atomics. The source says
the lock exists because multiple systems may update rollback state in parallel.
That is a narrow use of shared synchronization around small state, not broad
locking around gameplay data.

## Fixed Ticks And Rollback

Lightyear's client and server plugin groups take a `tick_duration`, defaulting
to 1/60 second. The docs for `lightyear_inputs_leafwing` explicitly say systems
that depend on user input should be in `FixedUpdate`.

This fixed-step model is how Lightyear makes prediction and rollback tractable:

| System | Behavior |
|---|---|
| Client input buffer | Stores local action snapshots by tick, including delayed ticks. |
| Client send path | Sends the latest input plus redundant recent ticks to tolerate packet loss. |
| Server input buffer | Receives client input messages and applies the predicted/current value for the server tick. |
| Rollback path | Reuses buffered historical inputs when re-running old ticks. |
| Prediction history | Records predicted state after fixed simulation so mismatches can trigger rollback. |

The important multi-threading point is that Lightyear makes time explicit. A
system should not read "current input" whenever it happens to run. It should read
the action state that Lightyear restored for the current fixed tick.

## Leafwing Input Manager Integration

Lightyear ships `lightyear_inputs_leafwing` for `leafwing_input_manager` action
types. The action enum must implement Leafwing's `Actionlike` trait and the
serialization/reflection traits required by the protocol. Users add
`lightyear_inputs_leafwing::InputPlugin::<A>::default()`.

The adapter works by networking Leafwing `ActionState<A>` snapshots through
Lightyear's generic input buffering layer:

| Piece | Role |
|---|---|
| `ActionState<A>` | Leafwing's per-entity action state. |
| `LeafwingSequence<A>` | Lightyear's serializable sequence/diff representation. |
| `InputBuffer<ActionState<A>, A>` | Per-entity tick-indexed history for prediction, delay, redundancy, and rollback. |
| `ClientInputPlugin<LeafwingSequence<A>>` | Buffers local input and sends redundant input messages. |
| `ServerInputPlugin<LeafwingSequence<A>>` | Receives input messages and updates server-side action state. |

The adapter intentionally does not support global Leafwing inputs stored as a
resource. Lightyear's input networking is entity-centric because the target
entity must be identified, mapped across replication, buffered by tick, and
possibly rolled back.

There is one subtle schedule detail specific to Leafwing. The Lightyear input
client source says remote player input messages run after
`RunFixedMainLoopSystems::BeforeFixedMainLoop` so Leafwing's local states have
already been switched to fixed-update state. The Leafwing adapter also configures
`InputSystems::RestoreInputs` before `InputManagerSystem::Tick` in
`FixedPreUpdate`. This prevents Leafwing from ticking stale action state after
Lightyear has restored a delayed or rollback tick.

## Implications For Afterglow

Afterglow now uses the full Lightyear stack as its multiplayer substrate,
following the scheduling discipline documented above.

| Area | Approach |
|---|---|
| Packet receive | Drain transport events in `PreUpdate` before replicated state is consumed. |
| Input capture | Capture raw/player input before fixed simulation and store it by simulation tick. |
| Gameplay simulation | Keep network-authoritative simulation in fixed schedules only. |
| Prediction | Record post-fixed-step history by tick, not by render frame. |
| Reconciliation | Re-run only deterministic fixed-schedule gameplay systems when Lightyear prediction requires it. |
| Packet send | Serialize outbound commands/snapshots after fixed simulation and reconciliation. |
| Parallelism | Use system ordering for correctness, then let Bevy parallelize compatible systems. |

Afterglow uses Leafwing Input Manager with per-controlled-entity `ActionState`
components for networked controls. The ownership/control source is Lightyear's
existing `ControlledBy` / `Controlled` relationship plus a Leafwing
`InputMap<AfterglowAction>` on the controlled entity. Leafwing actions flow
through Lightyear's input networking layer (`lightyear_inputs_leafwing`), so
fixed input delay, packet redundancy, and remote prediction share one source of
truth. Afterglow does not add custom player/avatar marker components for this.

## Risks And Open Questions

- Lightyear 0.26.4 tracks Bevy 0.18. Its APIs are still moving, so treat this as
  a design reference rather than a stable dependency decision.
- The Leafwing adapter is entity-scoped only. UI/global commands need a separate
  command path or an explicit controlled entity.
- Rollback-safe resources must only be mutated by fixed simulation systems, or
  they need explicit history. Lightyear documents the same restriction for
  resource rollback.
- Extra locks or atomics should stay limited to small scheduling flags. Gameplay
  state should remain ECS-owned and tick-indexed.
