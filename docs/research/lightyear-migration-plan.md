# Lightyear Migration Plan

## Decision

Use Lightyear's built-in transport stack for the rewrite. Do not build a custom
Iroh, Steam, memory, or generic `NetworkTransport` adapter in phase one.

The rewrite replaces Afterglow's old networking architecture, not the idea of
transport choice. Transport choice moves below Lightyear:

```text
Afterglow gameplay systems
  -> Afterglow server rewind
  -> Lightyear replication, messages, input, prediction, interpolation
  -> Lightyear-supported transports
```

## Transport Policy

| Use case | Target |
|---|---|
| Unit/integration tests | Lightyear local/in-process transport such as crossbeam or its current test transport |
| Native dev multiplayer | Lightyear UDP/netcode path first |
| Browser/WASM | Lightyear WebTransport or WebSocket path, if the game needs browser multiplayer |
| Steam release | Lightyear Steam support if it is sufficient; otherwise defer Steam networking work |
| Iroh/NAT traversal | Defer. Revisit only after core Lightyear path works, and only as a Lightyear-compatible adapter if still needed |

Non-negotiable rule: do not revive Afterglow's old `NetworkTransport`,
`NetworkPacket`, custom packet header, custom session, or custom command wire
format during this migration.

## Target Architecture

```text
Leafwing Input Manager
  - local bindings
  - ActionState<AfterglowAction> on controlled entities

lightyear_inputs_leafwing
  - input snapshots by tick
  - input delay
  - redundant input sending
  - rollback input restore

Lightyear
  - client/server/link entities
  - messages/channels
  - component replication
  - client prediction
  - remote interpolation
  - built-in transports

Afterglow server rewind
  - StableEntityId and RewindedEntity
  - typed component checkpoints/deltas
  - spawn/despawn lifecycle history
  - replay fixed gameplay for late valid input
  - emit component/entity/cue corrections

Afterglow gameplay
  - normal Bevy fixed-tick systems
  - authoritative validation
  - persistence and stable world identity
```

## Compatibility Check First

Current research found Lightyear `0.26.4` on Bevy `0.18`, and this repo uses
Bevy `0.18.1`. Pin semver versions deliberately. If Lightyear has moved to a
newer Bevy by the time the migration starts, choose one of these explicitly:

| Option | When to choose |
|---|---|
| Pin Lightyear `0.26.x` | Fast migration while staying on Bevy `0.18.1` |
| Upgrade Bevy | Only if Lightyear support or critical bug fixes require it |

## Phase 0: Freeze The Legacy Boundary

Goal: prevent new work from landing on the old stack while migration begins.

Actions:

1. Add a short module-level `LEGACY` note to old network/input modules if they
   remain in-tree during the transition.
2. Stop adding tests for old `NetworkTransport`, old `PlayerCommand`, old
   replication snapshots, and old prediction/reconciliation.
3. Keep existing tests only as behavioral references for the new Lightyear path.
4. Do not refactor old code except to make deletion easier.

Exit criteria:

1. Docs and roadmap name Lightyear + Leafwing + server rewind as the only target.
2. New multiplayer work has a clear place in `network::lightyear`,
   `network::rewind`, or Leafwing input.

## Phase 1: Dependencies And Features

Goal: add the new crates without changing gameplay behavior yet.

Candidate workspace dependency shape, adjusted after checking exact Lightyear
features:

```toml
[workspace.dependencies]
lightyear = { version = "0.26.4", default-features = false }
lightyear_inputs_leafwing = { version = "0.26.4", default-features = false }
leafwing-input-manager = { version = "0.20", default-features = false, features = ["keyboard"] }
```

Do not keep old `iroh` or `steam` features wired to old modules. If feature
names are kept for user-facing compatibility, they should enable Lightyear-backed
paths or produce an intentional compile error until implemented.

Remove later, after compile path is clean:

| Dependency | Reason |
|---|---|
| `iroh` | Old custom transport only |
| `steamworks` | Old custom transport only; re-add later only for lobby/auth if needed |
| `tokio` | Old Iroh worker only unless another subsystem needs it |
| `bytes` | Old packet handoff only unless Lightyear path needs it directly |
| `ggrs` | Research benchmark only |
| `afterglow-engine-macros` | Old custom `Replicate` derive only |

Verification:

```sh
cargo check -p afterglow-engine
cargo tree -p afterglow-engine -e features
```

## Phase 2: Leafwing Input Replacement

Goal: replace old string-keyed input with entity-scoped `ActionState` before
touching replication.

Add or rewrite:

| Path | Purpose |
|---|---|
| `crates/afterglow-engine/src/input/mod.rs` | Leafwing wrapper and public re-exports |
| `crates/afterglow-engine/src/input/actions.rs` | `AfterglowAction` enum implementing `Actionlike` |
| `crates/afterglow-engine/src/input/plugin.rs` | `AfterglowLeafwingPlugin` |
| `crates/afterglow-engine/src/input/scripted.rs` | Optional helpers for tests/cutscenes to mutate action state |

Delete or replace old files:

| Path | Action |
|---|---|
| `src/input/command.rs` | Delete |
| `src/input/bindings.rs` | Delete |
| `src/input/evaluation.rs` | Delete |
| old `src/input/tests/*` | Rewrite around Leafwing action maps and action state |

Action enum first pass:

```rust
pub enum AfterglowAction {
    MoveX,
    MoveY,
    LookX,
    LookY,
    Use,
    AttackPrimary,
    AttackSecondary,
    RaiseShield,
    Jump,
    Crouch,
    Sprint,
    Menu,
    DebugToggle,
}
```

Port controller systems to query `ActionState<AfterglowAction>` from the
controlled entity in fixed schedules. Do not preserve `PlayerCommand` as an
intermediate compatibility layer.

Tests:

1. Action press/hold/release maps correctly.
2. Axis actions produce expected movement/look values.
3. Controller remains stable with empty/no input.
4. Scripted tests can set action state without raw device resources.

## Phase 3: Minimal Lightyear Plugin

Goal: add Lightyear to the app with no game-specific replication yet.

Add:

| Path | Purpose |
|---|---|
| `src/network/mod.rs` | Thin re-export of `lightyear` and `rewind` modules |
| `src/network/lightyear/mod.rs` | Public Lightyear integration module |
| `src/network/lightyear/config.rs` | `AfterglowLightyearConfig` |
| `src/network/lightyear/plugin.rs` | `AfterglowLightyearPlugin` |
| `src/network/lightyear/protocol.rs` | Protocol registration entry points |
| `src/network/lightyear/schedule.rs` | Schedule notes/helpers only if needed |

Initial plugin responsibilities:

1. Add Lightyear client/server/shared plugins with a 60 Hz tick duration.
2. Add `lightyear_inputs_leafwing::InputPlugin::<AfterglowAction>`.
3. Configure only Lightyear built-in local/test transport first.
4. Expose one app extension for registering Afterglow network protocol types.
5. Avoid custom packet, channel, handshake, or session types.

Smoke test:

1. Start one server app and one client app in-process.
2. Establish a Lightyear connection using built-in local/test transport.
3. Send one Lightyear message from client to server.
4. Replicate one trivial component from server to client.

## Phase 4: Protocol And Replication Port

Goal: port replicated truth types to Lightyear registration.

Replace old custom patterns:

```rust
#[derive(Replicate)]
app.replicate(component::<Health>())
```

With Lightyear protocol registration:

```rust
app.register_component::<Health>();
app.register_component::<CombatTransform>();
app.register_message::<DamageCue>();
```

Keep the exact Lightyear APIs version-specific and verified against docs.rs.

First replicated components:

| Component | Reason |
|---|---|
| `CombatTransform` or equivalent gameplay transform | Movement and hit validation |
| `Health` | Death/correction tests |
| `ShieldState` | Late shield rewind test |
| `Hurtbox` | Combat validation |
| `ProjectileState` | Projectile lifetime and hit tests |

Spawn pattern target:

```rust
commands.spawn((
    StableEntityId::new(...),
    RewindedEntity::new(...),
    Health::new(100),
    ShieldState::default(),
    Replicate::to_clients(NetworkTarget::All),
));
```

Tests:

1. Server spawn appears on client.
2. Component update replicates.
3. Server despawn removes client entity.
4. Stable ID survives client entity remapping.

## Phase 5: Prediction And Interpolation

Goal: use Lightyear's built-in predicted/interpolated entity model.

Owned player target:

1. Server marks owner client as prediction target.
2. Client runs the same fixed movement code for predicted entity.
3. Lightyear reconciles when server state disagrees.

Remote player target:

1. Server marks non-owner clients as interpolation targets.
2. Client renders remote entities from interpolated state.
3. Presentation-only camera/headbob remains local and unreplicated.

Delete old modules after this phase passes:

| Old module | Replacement passed |
|---|---|
| `network/prediction` | Lightyear owned-player prediction test |
| `network/reconciliation` | Lightyear mismatch/correction test |
| `network/interpolation` | Lightyear remote interpolation test |

Tests:

1. Local movement responds before server round trip.
2. Server correction changes predicted position.
3. Remote entity moves smoothly with interpolation.
4. Presentation-only camera state is not replicated.

## Phase 6: Server Rewind Skeleton

Goal: add the Afterglow-specific layer Lightyear does not provide.

Add:

| Path | Purpose |
|---|---|
| `src/network/rewind/mod.rs` | Plugin and public API |
| `src/network/rewind/component.rs` | component snapshot descriptors and app extensions |
| `src/network/rewind/history.rs` | typed checkpoints and per-tick changes |
| `src/network/rewind/entity.rs` | `StableEntityId`, `RewindedEntity`, lifecycle events |
| `src/network/rewind/replay.rs` | restore and replay driver |
| `src/network/rewind/correction.rs` | correction diff output |

Registration API:

```rust
app.rewind_component::<Health>();
app.rewind_component::<ShieldState>();
app.rewind_component_with::<AiState, AiTruthSnapshot>(snapshot_ai, restore_ai);
```

Storage model:

1. Checkpoints provide full state anchors.
2. Per-tick component changes use Bevy `Added<T>`, `Changed<T>`, and
   `RemovedComponents<T>`.
3. Entity spawn/despawn lifecycle is tracked by `StableEntityId`.
4. History is bounded by ticks or duration.

Tests:

1. Record changed component by tick.
2. Restore checkpoint plus deltas to an old tick.
3. Restore removed component.
4. Restore spawned entity absence before spawn.
5. Restore despawned entity presence before despawn.
6. Reject restore outside retained history.

## Phase 7: Late Shield Regression

Goal: prove the rewrite solves the motivating fairness case.

Scenario:

```text
T100: A raises shield.
T108: server, missing A input, simulates B arrow killing A.
T108: death spawns corpse and loot as rewinded entities.
T111: A's valid shield input for T100 arrives.
Replay: shield blocks arrow.
Correction: A lives; corpse, loot, death cue, and projectile-hit cue vanish.
```

Assertions:

1. A has nonzero health and alive state after correction.
2. Corpse entity is absent on server and client.
3. Loot entity is absent on server and client.
4. Death cue is removed or canceled by ID.
5. No duplicate shield/block/damage cues are committed.
6. A stale late shield older than history is rejected.

This is the merge gate before deleting the old rollback/replication tests.

## Phase 8: Mock RPG Harness Rewrite

Goal: keep the integration harness alive, but rebuild it on the new stack.

Rewrite first:

| Test | Purpose |
|---|---|
| `network_e2e.rs` | current network-boundary latency/replay/correction correctness |
| `interactions.rs` | basic replicated use/pickup/door flow |
| `security.rs` | ownership and illegal action rejection |
| `projectile_edges.rs` | projectile lifetime, duplicate casts, stale hits |
| `smoothing.rs` | Lightyear prediction/interpolation behavior |
| `stress.rs` | many clients/NPCs under built-in Lightyear transport faults |

Keep pure math/rules helpers when useful. Delete custom legacy transport helpers,
custom packet DTOs, custom snapshot/delta builders, and custom prediction buffers.
The temporary local packet simulator exists only until real Lightyear link
entities can drive the same scenarios.

## Phase 9: Delete Legacy Network Stack

Delete in this order to reduce compile churn:

1. `network/commands`
2. `network/authority`
3. `network/session`
4. `network/handshake`
5. `network/iroh`
6. `network/steam`
7. `network/local_server`
8. `network/baseline`
9. `network/interest`
10. `network/prediction`
11. `network/reconciliation`
12. `network/interpolation`
13. `network/replication`
14. legacy `network/rollback` once `network::rewind` owns the replacement tests
15. `afterglow-engine-macros`
16. old networking benches

Do not leave compatibility re-exports except possibly short-lived compile shims
inside the migration branch. The final state should not expose the old API.

## Phase 10: Production Transport Selection

Goal: choose a production Lightyear transport after gameplay correctness works.

Order:

1. Use Lightyear local/crossbeam transport for deterministic tests.
2. Use Lightyear UDP/netcode for native dev multiplayer.
3. Use Lightyear WebTransport/WebSocket only if browser multiplayer is required.
4. Use Lightyear Steam only after lobby/auth requirements are clear.
5. Do not implement Iroh until a concrete Lightyear transport gap is proven.

Tests:

1. Two native clients connect to native server over chosen Lightyear transport.
2. Packet loss/reorder simulation still passes server rewind tests.
3. Disconnect and reconnect do not resurrect deleted persistent entities.
4. Steam/lobby tests remain manual-gated until real Steam environment exists.

## Phase 11: Benchmarks

Delete old networking benches that measure removed code:

| Old bench | Replacement |
|---|---|
| `replication` | Lightyear replication integration pressure |
| `authority` | server gameplay validation and rewind intake |
| `prediction` | Lightyear prediction integration smoke/pressure |
| `reconciliation` | Lightyear correction integration smoke/pressure |
| `interpolation` | Lightyear interpolation integration smoke/pressure |
| `baseline` | persistence and Lightyear reconnect scenario if needed |
| `ggrs` | remove |

Add:

| New bench | Cases |
|---|---|
| `server_rewind` | 1k/10k/100k entities, sparse and dense changes, restore/replay/diff |
| `lightyear_integration` | replicated spawn/update/despawn, predicted local movement, remote interpolation |
| `persistence_streaming` | keep existing streaming/persistence pressure |

## Phase 12: Documentation Cleanup

Required doc updates after code migration:

1. `docs/api/README.md` reflects actual module tree.
2. `docs/api/input.md` documents only Leafwing input.
3. `docs/api/network.md` documents only Lightyear + server rewind.
4. `docs/ROADMAP.md` marks completed migration tasks.
5. Research notes for Iroh/Steam stay historical and clearly say not phase-one
   architecture.
6. Remove stale references to old `PlayerCommand`, old `NetworkTransport`, old
   `Replicate` macro, old fake transport, and old packet headers from API docs.

## Definition Of Done

The migration is complete when:

1. `cargo test -p afterglow-engine` passes without old network/input modules.
2. `cargo test -p mock-rpg-network-tests` passes on Lightyear + server rewind.
3. Late shield/canceled death/corpse/loot regression passes.
4. The old `afterglow-engine-macros` crate is removed from the workspace.
5. Old optional `iroh`, `steamworks`, `tokio`, `bytes`, and `ggrs` dependencies
   are gone unless reintroduced for non-network reasons.
6. New benchmarks exist for server rewind and Lightyear integration.
7. Docs describe the actual Lightyear architecture and no longer present the old
   custom stack as current API.

## Expected Savings

Existing touched old code is roughly:

| Area | LOC |
|---|---:|
| `src/network` | 10,982 |
| `src/input` | 1,842 |
| old network benches except persistence | 749 |
| `afterglow-engine-macros` | 79 |
| `mock-rpg-network-tests` | 2,852 |

Expected replacement code:

| Area | LOC |
|---|---:|
| Lightyear integration | 300-450 |
| Leafwing input wrapper | 200-350 |
| server rewind | 900-1,300 |
| rewritten mock RPG harness | 2,000-2,800 |
| new benchmarks | 300-500 |

Expected net reduction after final deletion: **10k-12k LOC**.
