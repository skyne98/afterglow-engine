# Server Rewind Component History Plan

**Status: Historical research, not current architecture.** The engine no longer
ships `ServerRewindPlugin`, `RewindHistoryStore`, `RewindedEntity`, component
history registration, restore/replay, or correction-diff APIs. The current
networking baseline uses client prediction, deterministic fixed-tick simulation,
fixed server input delay, and Lightyear reconciliation. Keep this note only as a
reference if a future feature explicitly reopens server rewind research.

## Goal

Build an authoritative server rewind layer that lets game code stay close to
single-player Bevy code while the engine records enough ticked history to correct
late-but-valid player commands.

The first version should be boring and robust:

- Typed component snapshots, not reflection-heavy field deltas.
- Component-level change recording using Bevy change detection.
- Stable rewind IDs for entity lifetime, spawn, and despawn history.
- Bounded per-domain history with predictable memory and CPU cost.
- Replay-safe gameplay truth only; no particles, audio voices, UI, or render-only
  state in rewind history.

This plan complements `docs/research/authoritative-rollback-consistency.md`.

## Design Rule

Replication and rewind are related but not identical.

```rust
app.replicate_component::<Health>();
app.replicate_component::<Transform>();

app.rewind_component::<Health>();
app.rewind_component::<CombatTransform>();
app.rewind_component::<ShieldState>();
app.rewind_component::<Hurtbox>();
```

Only components explicitly registered as rewindable get server history. A
replicated component that never participates in late command correction should not
pay rewind costs.

## Developer-Facing API

Target ergonomics:

```rust
app.add_plugins(ServerRewindPlugin {
    history: RewindHistoryBudget::ticks(18),
});

app.rewind_component::<CombatTransform>();
app.rewind_component::<Health>();
app.rewind_component::<ShieldState>();
app.rewind_component::<Hurtbox>();

commands.spawn((
    RewindedEntity::new(stable_id),
    Player,
    CombatTransform::from_translation(spawn),
    Health::new(100),
    ShieldState::default(),
    Hurtbox::capsule(...),
));
```

Gameplay systems keep mutating normal components in fixed schedules. The rewind
plugin owns history capture, restore, replay, and correction diffing.

## Rewind Component Registration

Use registration to choose the storage strategy. The common path stores the full
component value and only requires `Clone`:

```rust
app.rewind_component::<Health>();
```

For large components or components with caches, allow a custom snapshot adapter:

```rust
app.rewind_component_with::<AiState, AiTruthSnapshot>(
    |state| AiTruthSnapshot::from(state),
    |state, snapshot| state.restore_truth(snapshot),
);
```

The internal descriptor should look like this:

```rust
pub struct RewindComponentDescriptor<T, S> {
    pub snapshot: fn(&T) -> S,
    pub restore: fn(&mut T, &S),
}
```

The `rewind_component::<T>()` helper expands to the clone descriptor:

```rust
fn clone_snapshot<T: Clone>(value: &T) -> T { value.clone() }
fn clone_restore<T: Clone>(value: &mut T, snapshot: &T) { *value = snapshot.clone(); }
```

Do not use a blanket `impl RewindComponent for T` if we need custom per-type
snapshot representations later; Rust coherence would make that hard to override.

## History Storage

Use typed ring buffers per component type, plus domain checkpoints. Deltas alone
are not enough because restoring an old tick needs a complete starting state.

```rust
pub struct ComponentHistory<S> {
    checkpoints: RingBuffer<ComponentCheckpoint<S>>,
    slots: RingBuffer<TickSlot<S>>,
}

pub struct ComponentCheckpoint<S> {
    tick: Tick,
    values: Vec<(StableEntityId, S)>,
}

pub struct TickSlot<S> {
    tick: Tick,
    changes: Vec<ComponentChange<S>>,
}

pub enum ComponentChange<S> {
    Added { id: StableEntityId, value: S },
    Changed { id: StableEntityId, value: S },
    Removed { id: StableEntityId },
}
```

Restore uses the latest checkpoint at or before the target tick, then applies
component changes forward until the target tick. The first version can checkpoint
every retained window or every small fixed interval such as 8 ticks. Benchmarks
should decide the final interval.

Avoid this in the hot path:

```rust
HashMap<Tick, HashMap<Entity, Box<dyn Reflect>>>
```

Typed storage keeps cache behavior predictable and avoids allocation-heavy
reflection or serde machinery during fixed ticks.

## Entity Lifetime

Rewind must track entity existence, not just component values.

```rust
pub struct RewindedEntity {
    pub domain: RewindDomainId,
}

pub enum RewindEntityEvent {
    Spawned {
        tick: Tick,
        id: StableEntityId,
        archetype: RewindArchetypeId,
        cause: Option<RewindCauseId>,
    },
    Despawned {
        tick: Tick,
        id: StableEntityId,
        cause: Option<RewindCauseId>,
    },
}
```

If a player death at tick `T` spawns a corpse and loot, those spawned entities are
provisional while `T` remains inside the rewind window. If replay later proves the
death did not happen, the corpse and loot do not appear in the replay result, so
the correction diff despawns them for clients automatically.

## Tick Schedule

Use one strict convention:

```text
FixedPreUpdate:
  apply accepted commands for tick T

FixedUpdate:
  run authoritative gameplay simulation for tick T

FixedPostUpdate:
  record rewind history for tick T
  prune history older than the budget
```

When a late command for tick `T` arrives:

```text
1. Reject it if T is older than retained history.
2. Restore the latest complete state before T.
3. Insert the command into the domain command log.
4. Replay fixed gameplay from T through current server tick.
5. Diff the previous live authoritative result against the replay result.
6. Replicate component changes plus entity spawn/despawn corrections, including cue entities.
```

## Change Detection

Use Bevy change detection to reduce writes:

```rust
Added<T>
Changed<T>
RemovedComponents<T>
```

Bevy tells us which components changed, but not which fields changed or what the
old values were. That is enough for the first version because component-level
snapshots are simpler and usually cheaper than field-delta machinery for combat
state.

## Field Deltas Later

Do not start with serde-based field deltas. `serde` serializes data; it does not
provide stable numeric field IDs, old values, reversible patches, or dirty-field
tracking by itself.

If profiling proves component snapshots are too expensive, add an explicit derive:

```rust
#[derive(Component, RewindDelta)]
struct LargeState {
    #[rewind(id = 1)]
    important_counter: u32,
    #[rewind(id = 2)]
    compact_flags: u64,
}
```

Generated code should own diff semantics. It may serialize deltas with serde or
bincode, but serde should not be the source of field identity or change tracking.

## Cost Model

Approximate upper bounds:

```text
memory = ticks_kept * changed_or_snapshotted_entities * snapshot_bytes
write_bandwidth = tick_rate * changed_or_snapshotted_entities * snapshot_bytes
```

Examples:

| Window | Entities | Snapshot | Memory |
|---:|---:|---:|---:|
| 15 ticks | 10,000 | 64 B | 9.6 MB |
| 30 ticks | 10,000 | 64 B | 19.2 MB |
| 60 ticks | 10,000 | 128 B | 76.8 MB |

This is acceptable if history is scoped to active authoritative domains and uses
typed contiguous storage. It is not acceptable if the entire open world, render
state, UI state, or large AI caches are blindly snapshotted.

## Implementation Milestones

1. Use `StableEntityId` with `RewindedEntity`, `RewindDomainId`, and a domain registry.
2. Add `RewindComponent` and `app.rewind_component::<T>()` registration.
3. Record `Added<T>`, `Changed<T>`, `RemovedComponents<T>` into typed ring
   buffers during `FixedPostUpdate`.
4. Record `Spawned` and `Despawned` entity lifecycle events with stable IDs.
5. Add restore-to-tick for one domain and a small set of registered components.
6. Add replay driver that runs the authoritative fixed gameplay schedule from a
   restored tick to the current tick.
7. Add correction diff output for component updates, entity spawns, entity
   despawns, and entity-backed gameplay cues.
8. Add benchmarks for 1k, 10k, and 100k entities with sparse and dense changes.
9. Add mock RPG regression tests for late shield, canceled death, corpse removal,
   loot removal, projectile lifetime, duplicate packets, reordered packets, and
   stale commands outside history.

## First Regression Scenario

```text
T100: A raises shield locally.
T108: Server, missing A's input, simulates B's arrow killing A.
T108: Death spawns a corpse and loot, both rewinded entities.
T111: A's valid shield input for T100 arrives.
T111: Server restores before T100 and replays.
T108 replay: Shield is active, arrow is blocked, A survives.
T111 correction: A health/state corrected; corpse and loot despawned on clients.
```

The test passes only if no stale death fact, corpse, loot, projectile hit, or
duplicate committed event remains after correction.

## Non-Goals For Version One

- No automatic field-level serde diffing.
- No rewind of presentation-only entities.
- No whole-open-world rollback.
- No rollback of arbitrary `Local<T>` state inside Bevy systems.
- No durable persistence or achievements from provisional events before the
  commit horizon expires.
