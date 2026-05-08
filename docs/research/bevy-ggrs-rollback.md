# bevy_ggrs — Rollback Networking Deep Dive

## Overview

**GGRS** (Good Game Rollback System) is a pure-Rust reimplementation of GGPO-style rollback netcode.
**bevy_ggrs** integrates GGRS into Bevy's ECS and schedule system.

- **bevy_ggrs**: v0.21.0 / Bevy 0.18 / GGRS 0.12.0
- **GGRS**: https://github.com/gschup/ggrs
- **bevy_ggrs**: https://github.com/gschup/bevy_ggrs

## How Rollback Works

1. **Predict and advance**: Run at full speed using local input, repeat last known input for remote players
2. **Save state**: Every frame, save the full game state (components, resources, entities)
3. **Reconcile**: When remote input arrives, check if prediction was wrong:
   - **Roll back** to last confirmed frame (load saved state)
   - **Re-simulate** from that frame forward with correct inputs
   - The visual result "catches up" instantly

This means **local input has zero network latency**. The cost: the game must be **deterministic** — same inputs must always produce same output.

## Core Types (GGRS crate)

### `Config` Trait

```rust
pub trait Config: 'static {
    type Input: Copy + Clone + PartialEq + Default + Serialize + DeserializeOwned;
    type State;                        // Save-state type
    type Address: Clone + PartialEq + Eq + Hash + Debug;
    type InputPredictor: InputPredictor<Self::Input>;
}
```

### Session Types

| Type | Purpose |
|---|---|
| `P2PSession<T>` | Multiplayer — connects peers over a custom transport |
| `SyncTestSession<T>` | Local determinism testing — simulates rollbacks |
| `SpectatorSession<T>` | Watch-only |

### `GgrsRequest` — The Heart of the Loop

After `session.advance_frame()`, GGRS returns requests you MUST fulfill:

```rust
enum GgrsRequest<T: Config> {
    SaveGameState { cell, frame },  // Clone current state → cell.save(state)
    LoadGameState { cell, frame },  // Restore state → state = cell.load()
    AdvanceFrame { inputs },        // Run one frame with these inputs
}
```

A normal frame: `[SaveGameState, AdvanceFrame]`
A rollback: `[LoadGameState, AdvanceFrame, SaveGameState, AdvanceFrame, ...]`

### `NonBlockingSocket` Trait

```rust
pub trait NonBlockingSocket<A>: Send + Sync {
    fn send_to(&mut self, msg: &Message, addr: &A);
    fn receive_all(&mut self) -> Vec<(Message, A)>;
}
```

Built-in: `UdpNonBlockingSocket`. Custom: Steam, WebRTC, WebSocket, ENET — anything.

## bevy_ggrs Architecture

### Plugin Structure

```
GgrsPlugin<C: Config>
├── Registers resources: Session, PlayerInputs, LocalInputs, LocalPlayers, etc.
├── Creates schedules: GgrsSchedule, ReadInputs, SaveWorld, LoadWorld, AdvanceWorld
└── Adds system: run_ggrs_schedules (in PreUpdate by default)
```

### Frame Loop

```
Each Bevy Frame (PreUpdate):
└─ run_ggrs_schedules
   ├─ Poll session for remote input
   ├─ Accumulate delta time
   ├─ While enough time accumulated:
   │  ├─ session.advance_frame()
   │  ├─ For each GgrsRequest:
   │  │  ├─ SaveGameState → SaveWorld schedule (snapshot components/resources)
   │  │  ├─ LoadGameState → LoadWorld schedule (restore from snapshots)
   │  │  └─ AdvanceFrame  → AdvanceWorld schedule
   │  │                      └─ GgrsSchedule (your game logic systems)
   │  └─ Handle GgrsEvents
   └─ (Next Bevy frame)
```

### Key Types

| Type | Purpose |
|---|---|
| `GgrsConfig<I, A, S>` | Default Config impl — use `type MyCfg = GgrsConfig<MyInput>` |
| `Session<T>` | Resource enum: `SyncTest \| P2P \| Spectator` |
| `GgrsSchedule` | Schedule — add your gameplay systems here |
| `ReadInputs` | Schedule — add input-reading systems here (NOT in GgrsSchedule!) |
| `PlayerInputs<T>` | Resource — `Vec<(Input, InputStatus)>` for current frame |
| `LocalInputs<T>` | Resource — populate in `ReadInputs`: `HashMap<PlayerHandle, Input>` |
| `LocalPlayers` | Resource — `Vec<PlayerHandle>` of local players |
| `Rollback` | Marker component — entities with this are included in rollbacks |
| `RollbackId` | Immutable stable ID — survives despawn/respawn |
| `GgrsTime` | Deterministic time — use instead of `Time<()>` in GgrsSchedule |

### Snapshot Strategies

| Strategy | Trait | Performance |
|---|---|---|
| `CopyStrategy` | `Copy` | Bitwise memcpy (fastest) |
| `CloneStrategy` | `Clone` | Heap allocation |
| `ReflectStrategy` | `Reflect + FromWorld` | Dynamic reflection (slow) |

## Complete Usage Pattern

### 1. Define Input

```rust
#[derive(Copy, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
struct FightInput(u8); // bitfield

type FightConfig = GgrsConfig<FightInput>;
```

### 2. Register rollback data

```rust
app.add_plugins(GgrsPlugin::<FightConfig>::default());
app.rollback_component_with_clone::<Transform>();
app.rollback_component_with_clone::<Health>();
app.rollback_resource_with_clone::<FrameCounter>();
app.checksum_resource_with_hash::<FrameCounter>();
```

### 3. Read input (ReadInputs schedule)

```rust
app.add_systems(ReadInputs, read_inputs);

fn read_inputs(keys: Res<ButtonInput<KeyCode>>, locals: Res<LocalPlayers>,
    mut commands: Commands) {
    let mut inputs = HashMap::new();
    for &handle in &locals.0 {
        let mut b: u8 = 0;
        if keys.pressed(KeyCode::KeyW) { b |= 1; }
        // ...
        inputs.insert(handle, FightInput(b));
    }
    commands.insert_resource(LocalInputs::<FightConfig>(inputs));
}
```

### 4. Game logic (GgrsSchedule)

```rust
app.add_systems(GgrsSchedule, physics_system);

fn physics_system(mut query: Query<&mut Transform, With<Rollback>>,
    inputs: Res<PlayerInputs<FightConfig>>, time: Res<Time>) {
    let dt = time.delta_secs_f32();
    // dt is deterministic (GgrsTime), NOT real wall-clock time
    for (i, mut tf) in query.iter_mut().enumerate() {
        let input = inputs[i].0;
        // apply input to transform
    }
}
```

### 5. Setup session

```rust
let session = SessionBuilder::<FightConfig>::new()
    .with_num_players(2)?
    .add_player(PlayerType::Local, 0)?
    .add_player(PlayerType::Remote("1.2.3.4:7000".parse()?), 1)?
    .start_p2p_session(udp_socket)?;
app.insert_resource(Session::P2P(session));
```

## Determinism Requirements

| Must be deterministic | Must NOT be used in GgrsSchedule |
|---|---|
| System iteration order (use `RollbackOrdered`) | `ButtonInput` (read in `ReadInputs` instead) |
| RNG (use synced seed) | `Time<()>` (use `GgrsTime` instead) |
| `f32`/`f64` math (watch for platform diffs) | `HashMap` iteration (non-deterministic) |
| Entity spawn/despawn (use `rollback_despawn()`) | Any non-rolled-back resource |

## Number of Players

GGRS supports **any number of players** (≥1) — no hard 2-player limit.

## References

- bevy_ggrs docs: https://docs.rs/bevy_ggrs/0.21.0/bevy_ggrs/
- GGRS docs: https://docs.rs/ggrs/0.12.0/ggrs/
- bevy_ggrs source: https://github.com/gschup/bevy_ggrs
- GGRS source: https://github.com/gschup/ggrs
