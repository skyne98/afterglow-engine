# Authoritative Rollback Consistency

**Status: Historical research, superseded for the baseline.** This document
captures rollback/replay design investigation. The current engine path does not
use rollback domains, server rewind, replay drivers, or correction diffs. The
baseline uses fixed server input delay, deterministic fixed-tick gameplay,
Lightyear client prediction, and Lightyear reconciliation. Keep this document as
research material only.

## TLDR

Afterglow should not try to manually mark every possible irreversible gameplay
chain. The maintainable model is:

1. Keep authoritative gameplay truth in a snapshot-restorable domain.
2. Store commands by simulation tick.
3. When a late valid command arrives, restore the domain to the previous saved
   tick, replay commands in deterministic order, and publish the corrected
   snapshot/delta.
4. Treat UI, audio, particles, animation blends, and editor overlays as
   presentation derived from gameplay state or from replayed cue records.

This is the common shape behind GGPO/GGRS rollback, Unity Netcode prediction,
Source lag compensation, SnapNet server rewind, and Unreal Network Prediction.
They differ in scope, but the important invariant is the same: gameplay truth is
restorable and replayable; presentation is not allowed to be the source of truth.

## What Other Systems Do

### GGPO / GGRS

GGRS describes the minimal rollback contract as save state, load saved state, and
advance one frame from player inputs. It handles input exchange, prediction,
rollback scheduling, time synchronization, and desync detection around that
contract.

Source: https://docs.rs/ggrs/latest/ggrs/

The lesson for Afterglow is that our rollback API should expose a simple engine
contract: snapshot domain, restore domain, run fixed-tick gameplay. The current
`DeterministicRollbackBuffer` is a useful primitive, but it is too low-level to
be the final game-facing API.

### bevy_ggrs

bevy_ggrs snapshots registered components and resources, restores them on
rollback, reconciles spawned/despawned entities through stable rollback IDs, and
runs gameplay in a rollback schedule. Its pitfalls are directly relevant:
messages and `Local<T>` state are not snapshot-restored, raw Bevy `Entity` handles
need remapping, and query order must be deterministic.

Source: https://github.com/gschup/bevy_ggrs

The lesson is that Afterglow needs stable entity identity, deterministic schedule
order, and explicit snapshot ownership for components/resources in rollback
domains.

### Unity Netcode For Entities

Unity applies received server snapshots to predicted entities, finds the oldest
snapshot tick that changed, then runs the predicted simulation from that tick to
the prediction target. Only entities tagged for prediction participate in the
prediction loop; unchanged entities can continue from their previous state. The
same predicted gameplay code runs on the server once and on clients during
re-simulation.

Source: https://docs.unity.cn/Packages/com.unity.netcode%401.0/manual/prediction.html

The lesson is to make rollback domain membership automatic and queryable, like a
`Simulate` tag for the current replay tick, instead of making every gameplay
system branch manually on network state.

### Source Engine

Source uses an authoritative server with fixed ticks, sends snapshots, predicts
the local player, interpolates remote entities, and uses server-side lag
compensation for hits. For lag compensation, the server stores recent player
location and animation history, estimates the command execution time, rewinds
other players, evaluates the command, then restores the current state.

Sources:
- https://developer.valvesoftware.com/wiki/Source_Multiplayer_Networking
- https://developer.valvesoftware.com/wiki/Lag_Compensation
- https://developer.valvesoftware.com/wiki/Prediction
- https://developer.valvesoftware.com/wiki/Interpolation

The lesson is that an immersive sim RPG should combine snapshot replication,
client prediction, interpolation, and server rewind. Rewinding only hitboxes is
too narrow for spells, shields, doors, moving platforms, traps, and stealth
state; rewinding whole gameplay entities is safer.

### SnapNet

SnapNet frames rollback as requiring deterministic fixed ticks and fast game
state serialization. It calls out audio and visual effects as a special problem:
they often should not be fully snapshotted; instead gameplay state records that
an effect occurred, and presentation starts, stops, or ignores short effects
based on the corrected state. Its server rewind API rewinds entire entities, not
only selected hitboxes, so code can check historical ability state as well as
historical position.

Sources:
- https://snapnet.dev/blog/netcode-architectures-part-2-rollback/
- https://snapnet.dev/docs/unreal-engine-sdk/manual/server-rewind/

The lesson is to avoid a per-feature "undo this UI/audio thing" API. Gameplay
emits replayable cue facts; presentation reacts to the latest accepted cue log.

### Unreal Network Prediction

Unreal's Network Prediction plugin separates simulation data from cue
dispatching. Its cue dispatcher is explicitly notified when simulation rolls
back so saved and transient cues can be handled around re-simulation.

Source: https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Plugins/NetworkPrediction/TNetSimCueDispatcher

The lesson is to make cues a first-class output of replay, not arbitrary Bevy
messages that may already have been consumed by UI, audio, or scripts.

### Gambetta Client Prediction

Gambetta's reconciliation model keeps unacknowledged client inputs after an
authoritative server update and reapplies them on top of the latest server
state. It also warns that predicted gameplay side effects such as kills should
not become final until server authority confirms them.

Source: https://www.gabrielgambetta.com/client-side-prediction-server-reconciliation.html

The lesson is that clients can stay responsive, but the server snapshot remains
the final truth. The client never repairs truth by interpreting presentation
state; it reapplies local pending commands onto authoritative state.

## Proposed Afterglow Model

### Rollback Domain

A rollback domain is a bounded authoritative gameplay state set, not necessarily
the whole open world. Examples:

- One loaded cell.
- A combat bubble around interacting players and NPCs.
- A streamed chunk plus nearby cross-chunk interactables.
- A deterministic puzzle mechanism.

Each domain stores:

- Stable domain ID.
- Current authoritative tick.
- Ring buffer of domain snapshots.
- Per-player/per-peer command log.
- Replay-generated cue log.
- Stable entity ID map for entities in the domain.

The domain is the unit of restore and replay. If a late command affects multiple
domains, it is either routed to the owning domain or escalated into a larger
domain. Keep this rare; most RPG interactions should have one authoritative
owner.

### Committed And Provisional State

Each domain has two state layers:

- **Committed state** is the durable anchor at `committed_tick`. It cannot be
  argued anymore and is safe for persistence, pruning, and irreversible external
  bookkeeping.
- **Provisional state** is the live authoritative gameplay result produced by
  replaying accepted commands after `committed_tick`.

Gameplay uses provisional state for movement, hit detection, command legality,
AI, projectiles, doors, and traps. Committed state is not used for live combat,
because it is intentionally old. The relationship is:

```text
committed_state + accepted_commands_after_committed_tick = provisional_state
```

The commit horizon is global per domain:

```text
committed_tick = current_tick - commit_delay_ticks
```

Commands at or before `committed_tick` are rejected as already final. Commands
after `committed_tick` can rebuild provisional state.

### Truth Components

Only gameplay truth belongs in snapshots:

- Stable IDs, ownership, chunk/cell membership.
- Transforms used for gameplay, velocities, collision state.
- Health, stamina, status effects, inventory, door state.
- Spell/projectile state.
- AI state that affects gameplay.
- Deterministic timers and RNG seeds.

Do not snapshot:

- Render meshes, materials, GPU handles.
- Audio voices.
- Particle instances.
- UI widgets.
- Camera shake.
- Animation blend presentation.
- Debug overlays.

If presentation data affects gameplay, it is not presentation data. Move the
gameplay-relevant value into a truth component and derive the visual from it.

### Replicated Component Caste

The Bevy-facing architecture should keep replicated components as their own
small truth schema. These components store only values the engine cares about for
networking, save/load, rollback, or authoritative correction. Normal Bevy
components can be rich, stateful, cached, hierarchical, or presentation-oriented,
but they derive from replicated truth.

Example:

```text
RepNpc { pos, hp, alert, animation_state, target }
  -> drives Transform, animation graph state, navigation caches, audio, UI
```

Server gameplay should mutate Lightyear-replicated components/resources through
ordered fixed-tick systems. The developer-facing target API is:

```text
app.register_component::<RepHealth>()
app.rewind_component::<RepHealth>()
app.add_systems(FixedUpdate, apply_damage)
```

Game code reads Leafwing action state and replay-safe retained fact streams.
During server rewind, the engine restores registered component state and runs the
same fixed systems again.

This keeps important multiplayer code visually separate: client input becomes
validated server intent, normal Bevy systems mutate Lightyear-replicated and
rewindable truth, then Lightyear replicates corrections to clients. The rewind
driver is intentionally small and Bevy-shaped:

```text
server_rewind.restore_domain(98)
server_rewind.replay_domain(98, 100)
```

`replay_domain(anchor, through)` restores registered rewind components and entity
lifetime to `anchor`, inserts accepted ticked input/facts, then runs the fixed
gameplay schedule for later ticks. Those systems use normal `Query`, `Res`, and
component mutation.

### Commands

Input is player-owned, tick-stamped intent. Leafwing action state should cover
continuous controls and common buttons:

- move/look axes
- use/interact
- attack/cast/shield
- jump/crouch/sprint

Game-specific selected targets, item IDs, dialogue choices, or ability IDs can be
retained replay facts/messages above Lightyear. The engine validates ownership,
tick window, sequence/duplicate rules, and domain routing. Game code validates
game-specific legality during replay.

### Replay

When a valid late command arrives:

1. Find the affected rollback domain.
2. Reject if the command tick is older than retained history.
3. Restore the latest snapshot before the command tick.
4. Clear replay-generated cues after the restored tick.
5. Replay all accepted commands in deterministic order through the fixed gameplay
   schedule until the current server tick.
6. Compare resulting authoritative state to the previous current state.
7. Publish snapshot/delta and entity lifecycle corrections to clients.

This removes per-message manual cleanup. A "B died" outcome that vanishes after
replay is not manually undone; the corrected snapshot says B is alive, and the
death cue entity is absent or unconfirmed.

Fast-moving projectiles and player body-blocking use the same rule. Collision is
evaluated against provisional transforms and swept colliders during replay. If a
late paladin movement command puts the paladin into a projectile path before the
mage, replay produces a paladin hit. If the command is already committed, it is
too late and the previous outcome remains final.

### Entity Lifecycle

Spawns and despawns are rollback truth. Deterministic gameplay should not treat
raw Bevy entity creation/destruction as the authoritative fact. Instead, it
records lifecycle facts:

- stable entity ID
- spawn tick
- optional despawn tick
- optional despawn reason

Entities spawned inside the provisional window are provisional. Entities
despawned inside the provisional window are tombstoned provisionally, not
physically forgotten by the authoritative model. Physical cleanup happens after
the relevant tick is committed.

This handles chains automatically. If corrected replay says a character died
before firing, the later projectile spawn is absent from the replay-generated
lifecycle outputs. If the shot occurred before death, the projectile remains.
Ammo, cooldown, projectile hit, damage, and cues follow from the replayed state.

### Cues

Cues are deterministic presentation outputs, not Bevy `MessageWriter` side
effects. Near term, correction-sensitive cues should be small Lightyear-predicted
entities: hit markers, damage numbers, decals, beams, or fadeable audio emitters.
The client predicts them with `PreSpawned`; the server confirms them with the same
hash or lets Lightyear expire the unmatched entity.

Presentation systems should key reversible effects to the cue entity. Entity
despawn means cancellation or fade. One-shot effects that cannot be stopped should
wait for confirmation or be harmless local feedback. A retained cue log can be
added later if replay auditability needs more than entity lifetime.

### Gameplay Message Streams

Networked entity-to-entity interaction should be message/command sourced through
a retained rollback stream, not Bevy's frame-local `Messages<T>`.

After replay, the engine has:

- **provisional messages**: the current replay result after `committed_tick`; these
  can be replaced by later correction and produce added/removed diffs.
- **committed messages**: facts whose ticks passed the commit horizon; these are
  final and safe for durable business logic.

Use provisional messages for live feedback and correction-aware presentation:

```text
DamageApplied provisional added -> show damage number
CharacterDied provisional removed ID -> cancel provisional death presentation
ShieldBlocked provisional added -> show shield impact
```

Use committed messages for irreversible or durable work:

```text
CharacterDied committed added -> persist kill, advance quest, award achievement
DoorOpened committed added -> write durable world delta
ItemPickedUp committed added -> finalize inventory ownership
```

This keeps the DX close to Bevy systems while avoiding raw `MessageWriter` in
rollback truth. Game systems can still be written as message transforms:

```text
ProjectileHit -> DamageApplied -> CharacterDied -> LootDropped
```

The difference is that the stream is retained, tick-addressed, stable-ID based,
and has provisional/committed views.

For performance, removed provisional facts are reported by message ID rather than
by cloning the old message payload. Presentation systems should track active
effects by message ID.

### Clients

Clients use three layers:

- Authoritative snapshot/delta application.
- Local command replay for predicted owned entities.
- Interpolation/extrapolation for remote entities.

The client does not need a manual rollback cleanup API. On correction, it applies
the server snapshot, replays still-unacknowledged local commands, and lets
presentation derive from corrected truth and cue entity lifetime.

## Implementation Direction

## Local Benchmark

`cargo bench -p afterglow-engine --bench ggrs` measures GGRS 0.12.0 with a
`SyncTestSession`. This intentionally forces rollback/re-simulation, so it is a
good stress shape for "what if we use GGRS-style full-state save/load".

Latest local result:

| Case | State | Rollback | Average frame |
|---|---:|---:|---:|
| coordinator only | 28 B | none | 65 ns |
| 10k entities | 280 KB | none | 5.518 us |
| 10k entities | 280 KB | 4 ticks | 51.926 us |
| 10k entities | 280 KB | 8 ticks | 90.060 us |
| 100k entities | 2.8 MB | 8 ticks | 1.128 ms |

Interpretation:

- GGRS itself is not the limiting factor.
- State cloning, restoring, and replaying gameplay work are the limiting factors.
- Full-domain rollback of 100k tiny entities is already around 1.1 ms per
  simulated frame in this synthetic bench, before real physics, scripts, AI,
  animation, or Bevy world extraction costs.
- Therefore Afterglow should not roll back the entire open world. Use bounded
  authoritative domains/bubbles and keep their truth state compact.

The existing `rollback` bench remains useful for the engine's low-level byte
snapshot primitive. The GGRS bench is useful for estimating full-state
rollback-session pressure.

`cargo bench -p afterglow-engine --bench rollback` now also measures the local
committed/provisional domain API:

| Case | Commands | Average rebuild | Average promote |
|---|---:|---:|---:|
| current tick 128 | 64 | 2 us | 2 us |
| current tick 1024 | 128 | 6 us | 5 us |
| current tick 8192 | 256 | 18 us | 17 us |

This benchmark uses compact byte state and cue output. It is a baseline for the
domain coordinator, not a substitute for later Bevy-world or gameplay-heavy
benchmarks.

The same benchmark now measures retained message-stream maintenance:

| Messages | Ticks | Replace provisional stream | Commit half |
|---:|---:|---:|---:|
| 128 | 128 | 8 us | 6 us |
| 1024 | 128 | 28 us | 19 us |
| 8192 | 128 | 422 us | 305 us |
| 100000 | 128 | 7.295 ms | 3.397 ms |

The stream uses sorted tick buckets and removed-message IDs instead of cloning old
removed payloads. The 100k full-replacement case is still too expensive for a
single frame. That is expected: it means the replay produced 100k changed facts,
and the engine must hand 100k added facts to consumers. The design rule remains:
keep rollback streams per active domain, use compact message payloads, and do not
put particles or idle world entities in the retained gameplay message stream.

### Keep From Current Code

- `StableEntityId` and chunk membership remain the right identity foundation.
- The old command, prediction, interpolation, baseline, and transport modules are
  migration references only; Lightyear and Leafwing should replace them.
- The removed server-rewind layer is historical research only. Byte snapshots are
  optional for small deterministic subsystems only if that research is reopened.

### Change Next

Historical proposal: replace per-message commit delay with domain replay as the
main safety mechanism. Current baseline decision: do not do this; use fixed input
delay instead.

Add a higher-level API on top of `DeterministicRollbackBuffer`:

- `RollbackDomainId`
- `RollbackDomainSnapshot`
- `RollbackDomainHistory`
- `RollbackCue`
- `RollbackCueLog`
- `RollbackDomainRegistry`
- `RollbackReplaySchedule`

The first implementation can serialize a small mock RPG domain into bytes and
replay pure Rust test logic. It does not need Bevy world snapshotting on day one.

### Test Requirements

`mock-rpg-network-tests/tests/network_e2e.rs` is now a legacy regression oracle for
older rollback-style scenarios:

- Two players exchange spell projectiles.
- Player B sends a shield command late.
- The local packet simulator applies latency, duplication, reordering, drops,
  and stale delivery around `AfterglowNetworkPlugin`.
- The legacy harness owns its own local snapshot/replay logic and corrects "B
  died" into "B survived" without engine rewind-history APIs.
- No duplicate final death, stale combat log truth, stale projectile hit,
  orphaned death marker, or orphaned loot remains in the authoritative model.

Add a sync-test style harness:

- Run the same domain twice with the same commands.
- Compare checksums each tick.
- Fuzz command order, packet delivery, and reconnect snapshots.

## Decision

For the current baseline, use fixed server input delay as the universal
consistency model. Do not build feature-specific undo hooks and do not add a
server rewind layer unless a future feature proves fixed delay plus Lightyear
reconciliation is insufficient. Gameplay truth still belongs in deterministic
fixed-tick state; presentation reads replicated truth and cue entity lifetime.
