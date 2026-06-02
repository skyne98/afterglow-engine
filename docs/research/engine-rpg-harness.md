# Engine RPG Harness

## Overview

`engine-rpg-harness` is a deterministic, tick-level test rig for Bevy + Lightyear networked multiplayer games. It owns server + N client `App` instances, wires crossbeam (in-memory) or UDP transport between them, and exposes a schedule pump that steps all worlds in lockstep. Game concepts (health, combat, physics, doors) live in scenario modules on top of the rig — the rig itself is infrastructure-only.

**Why it exists:** Replace the legacy `mock-rpg-network-tests` crate with a clean, extensible harness that proves the engine's networking, prediction, controller, physics, and combat systems work correctly under both Crossbeam and UDP transports. The harness uses **fixed input delay instead of server rewind** — inputs are queued and processed N ticks later (configurable `delay_ms` → `delay_ticks`), so by the time the server processes tick T, all inputs for T have arrived.

**What it proves:** 80 tests across 12 categories covering rig infrastructure,
local controller+physics, networked combat (PVP/PVE), lockstep edge cases
(duplicate/reorder/drop/retention), PreSpawned entity lifecycle, AoE/status
effects/cooldowns, boss phases, knockback, door interactions, adversarial inputs
(NaN/Inf, future ticks, out-of-range attacks), stress replication, UDP
transport, full-stack UDP client-to-server `ActionState<AfterglowAction>`
delivery, and native Leafwing input over both Crossbeam and UDP — many
scenarios now include explicit UDP variants that exercise real netcode sockets
alongside the existing Crossbeam coverage.

## Architecture

### LightyearTestRig

**Location:** `crates/engine-rpg-harness/src/rig.rs`

Owns:
- **`server_app: App`** — authoritative simulation world
- **`client_apps: Vec<App>`** — N client prediction worlds
- **`server_links: Vec<Entity>`** — per-client connection entities on the server
- **`client_links: Vec<Entity>`** — per-client connection entities on the client
- **`entity_map: HashMap<StableEntityId, Vec<Entity>>`** — maps stable IDs to [server_entity, client0_entity, client1_entity, ...]
- **`current_tick: u32`** — global tick counter
- **`input_delay_ticks: u32`** — delay applied to `queue_action` deliveries
- **`pending_inputs: Vec<(u32, u32, Box<dyn FnOnce(&mut App)>)>`** — (intended_tick, deliver_at_tick, closure)
- **`retention_window_ticks: u32`** — stale input rejection window; 0 = no limit

Does NOT own any game/RPG concepts — those go in scenario modules.

**TransportConfig** (`crates/engine-rpg-harness/src/rig.rs`):
```rust
pub enum TransportConfig {
    Crossbeam,                    // in-memory channels, default
    Udp { server_port: u16 },     // real netcode UDP sockets
}
```

**Input delay model:** `with_input_delay_ms(delay_ms)` converts milliseconds to ticks at `1000 / tick_rate`. When `queue_action(tick, closure)` is called, the closure is stored with `deliver_at = tick + input_delay_ticks`. On each `advance()` step, inputs whose `deliver_at <= current_tick` are drained and applied to the server app. `with_retention_window_ticks(n)` drops inputs whose `intended_tick < current_tick - n` before delivery.

**advance() schedule pump** (`crates/engine-rpg-harness/src/rig.rs`):

Per tick:
1. **Clients:** `PreUpdate` (receive replication)
2. **Clients:** `FixedFirst` (advance Lightyear tick)
3. **Clients:** `FixedPreUpdate` (write/buffer native input)
4. **Clients:** `FixedUpdate` (predict physics/controller)
5. **Clients:** `PostUpdate` (send messages)
6. **Server:** `PreUpdate` (receive client messages)
7. **Server:** drain pending inputs (retention check → deliver matched)
8. **Server:** `FixedFirst` (advance Lightyear tick)
9. **Server:** `FixedPreUpdate` (apply received native input for the fixed tick)
10. **Server:** `FixedUpdate` (authoritative simulation)
11. **Server:** `FixedPostUpdate` (post-simulation hooks)
12. **Server:** `PostUpdate` (send replication)
13. **Clients:** `FixedPostUpdate` (reconcile prediction vs confirmed)

**Entity mapping:** `register_entity(sid, entities)` stores 1 server + N client entities. `server_entity(sid)` and `client_entity(sid, client_id)` look them up. `find_client_entity(client_id, sid)` queries by `StableEntityId` component (requires component registration). `spawn_replicated(sid, bundle)` spawns with `Replicate` + `PredictionTarget` on the server and triggers immediate `PostUpdate`/`PreUpdate` to push replication.

### Game Components

**Location:** `crates/engine-rpg-harness/src/scenarios/components.rs`

| Component | Fields | Description |
|---|---|---|
| `Health` | `current: i32, max: i32` | Current and maximum HP |
| `CombatState` | `shield_active_until: u32, dead: bool, last_attack_tick: u32` | Combat status: shield expiry tick, death flag, last attack tick (for cooldowns) |
| `Corpse` | `victim: StableEntityId` | Spawned on death for loot interaction |
| `Loot` | `owner: StableEntityId, picked_up: bool` | Pickup-able loot dropped on death |
| `ManaPool` | `current: i32, max: i32` | Mana resource (30 cost per primary attack) |
| `BurnEffect` | `remaining_ticks: u32, damage_per_tick: i32` | Damage-over-time status effect |
| `SpawnPoint` | `position: Vec3` | Respawn position |
| `DeadTimer` | `remaining: u32` | Countdown before respawn (10 ticks) |
| `DoorState` | `open: bool, locked: bool` | Door interactable state |
| `Team` | `u32` | Team affiliation (friendly fire disabled same-team) |
| `Enemy` | `attack_range: f32, damage: i32, detection_range: f32` | AI enemy parameters |
| `Boss` | `phase: u32, max_phases: u32, phase_hp_thresholds: Vec<i32>` | Multi-phase boss with HP thresholds |
| `DoorGrab` | `player: StableEntityId, door: StableEntityId` | PreSpawned cue for door interaction |

### Game Systems

**Location:** `crates/engine-rpg-harness/src/scenarios/systems.rs`

All systems are registered in `FixedUpdate` in scenario-specific chains:

| System | Description |
|---|---|
| `advance_history_tick` | Increments `HistoryTick` resource each tick |
| `resolve_shields` | Activates shield on `RaiseShield` action (lasts 20 ticks) |
| `resolve_attacks` | Targets nearest enemy within 8.0 range, applies 34 damage + 2.0 knockback; respects cooldown, team checks, shield blocking |
| `resolve_aoe_attacks` | AoE secondary attack (10.0 range, 25 damage); ignores knockback; respects team and shield |
| `apply_deaths` | Marks `CombatState::dead = true` when HP ≤ 0; spawns Corpse + Loot (server only) |
| `sync_dead_state` | Removes/restores `FirstPersonController` based on death state |
| `resolve_loot_pickup` | `Use` action within 2.0 range picks up loot |
| `process_mana_for_attack` | Deducts 30 mana per primary attack; clears action if insufficient |
| `apply_burn_damage` | Applies `damage_per_tick` each tick; removes `BurnEffect` when expired |
| `mark_dead_for_respawn` | Inserts `DeadTimer { remaining: 10 }` on newly dead entities |
| `respawn_dead_players` | Counts down DeadTimer; respawns at SpawnPoint with full HP when timer hits 0 |
| `move_players` | Direct transform translation from Move axis (5.0 speed, 60Hz) |
| `resolve_door_interactions` | Use action within 3.0 of closed, unlocked door spawns `DoorGrab` (server only) |
| `apply_door_open` | Opens door and pulls player toward it on confirmed grab |
| `enemy_attack_system` | AI: detects players within range, auto-attacks nearest |
| `boss_phase_transition` | Advances Boss phase based on HP thresholds |
| `door_grab_hash` | Deterministic hash `(player.0 ^ door.0) as u64` for PreSpawned matching |

**System ordering in `combat.rs`:**
1. `advance_history_tick` → `sync_dead_state` → `resolve_shields`
2. `enemy_attack_system` → `resolve_attacks` → `resolve_aoe_attacks`
3. `apply_deaths` → `mark_dead_for_respawn` → `respawn_dead_players` → `boss_phase_transition`

**System ordering in rpg.rs:**
`advance_history_tick` → `process_mana_for_attack` → `resolve_shields` → `resolve_attacks` → `resolve_aoe_attacks` → `apply_burn_damage` → `apply_deaths` → `mark_dead_for_respawn` → `respawn_dead_players` → `resolve_loot_pickup` → `move_players`

**System ordering in gold.rs:**
`advance_history_tick` → `sync_dead_state` → `resolve_shields` → `resolve_attacks` → `apply_deaths` → `resolve_loot_pickup`

### register_protocol pattern

Each scenario defines a registration function called on every app (server + clients):

```rust
fn register_foo(app: &mut App, role: LightyearRole) {
    app.init_resource::<HistoryTick>();
    app.register_component::<StableEntityId>();
    app.register_component::<Health>().add_prediction();
    app.add_systems(FixedUpdate, (system_a, system_b).chain());
}
```

`LightyearRole` distinguishes Server/Client/Host. Systems that should only run on the server check `AfterglowLightyearConfig::role`.

## Test Coverage Map

### Rig Infrastructure (`crates/engine-rpg-harness/src/lib.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `rig_boots_and_delivers_message` | Crossbeam | Rig boots 1 client, sends/receives `TestPing` over custom channel, batching over multiple ticks |
| `input_delay_defers_server_processing` | Crossbeam | 50ms delay (~3 ticks) defers `queue_action` delivery; action fires exactly at delivery tick |
| `crossbeam_action_state_flows_as_custom_message` | Crossbeam | `ActionState<AfterglowAction>` serialized as Lightyear message, received on server with correct `Jump` press |
| `udp_rig_connects_and_delivers_message` | UDP | UDP transport boot, netcode handshake, message send/receive over real sockets |
| `udp_spawn_replicated_arrives_on_client` | UDP | `spawn_replicated` over UDP: client receives entity with matching `StableEntityId` |
| `udp_connect_adds_server_replication_sender_and_is_idempotent` | UDP | Server UDP links get `ReplicationSender` from the `LinkOf` observer while retaining Lightyear-managed `Transport`/`MessageManager`; repeated `connect()` is stable |

### Local Controller + Physics (`src/controller.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `player_moves_forward_with_controller_and_physics` | none (local) | FirstPersonController with Avian physics moves forward, gets `RigidBody` + `Collider` via authoring |
| `player_jumps_and_lands` | none (local) | Jump leaves ground, gravity pulls back, lands and re-grounds |
| `player_look_rotates_yaw` | none (local) | Look input rotates yaw; yaw stays constant without input |
| `player_crouches_under_ceiling` | none (local) | Crouch stance fits under low ceiling; cannot stand up while under |
| `wall_stops_movement` | none (local) | Player pushes into wall, stops before clipping, stays grounded |

### Physics Integration (`crates/engine-rpg-harness/src/scenarios/physics.rs`)

*Physics scenarios are local-only and do not need UDP variants — they test pure deterministic simulation independent of transport.*

| Test | Transport | What it proves |
|---|---|---|
| `moving_platform_carries_player` | none (local) | Kinematic platform carries dynamic body resting on it |
| `character_pushes_dynamic_object` | none (local) | Controller collides with dynamic crate, stops before it, crate stays put |
| `projectile_flight_and_collision` | none (local) | Dynamic sphere follows parabolic arc, collides with static wall, velocity changes |
| `multi_player_collision_separation` | none (local) | Two controllers move toward each other, collide, stay separated without clipping |

### Gold Scenarios (`crates/engine-rpg-harness/src/scenarios/gold.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `alice_attacks_bob_shield_blocks` | Crossbeam | Full stack: replicated entities, input delay, shield blocks attack via system ordering; server + client state match; no corpse/loot |
| `shield_blocks_attack_over_udp` | UDP | Same combat logic over real UDP sockets with entity replication; client states match server |

### Lockstep Matrix (`crates/engine-rpg-harness/src/scenarios/lockstep.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `duplicate_same_tick_input_rejected` | Crossbeam | Two heals queued for same tick — first applies, second sees marker and skips → 110 HP not 120 |
| `reordered_inputs_produce_same_result` | Crossbeam | Inputs queued in reverse chronological order — delivery ticks enforce correct ordering |
| `delayed_input_still_delivers` | Crossbeam | Input queued at tick 3 with 50ms delay → delivered at tick 6 |
| `input_within_delay_window_processed` | Crossbeam | Retention window=5; heal within window is delivered |
| `input_outside_delay_window_rejected` | Crossbeam | Input with intended_tick=1 delayed to deliver at 20, window=5 → dropped at tick 7 |
| `same_tick_shield_blocks_attack_ordered_correctly` | Crossbeam | Shield and attack same tick → shield runs first in system chain → BOB survives |
| `server_clamps_heal_to_max_hp` | Crossbeam | Heal attempted at max HP → server caps at max (100) |
| `udp_duplicate_same_tick_input_rejected` | UDP | Same-tick duplicate rejection over UDP |
| `udp_reordered_inputs_produce_same_result` | UDP | Reverse-chronological queue produces correct delivery order over UDP |
| `udp_delayed_input_still_delivers` | UDP | Input queued at tick 3 with 50ms delay → delivered at tick 6 over UDP |
| `udp_same_tick_shield_blocks_attack` | UDP | Same-tick shield+attack ordering over UDP |
| `udp_server_clamps_heal_to_max_hp` | UDP | Server caps HP at max over UDP |

### PreSpawned Entities (`crates/engine-rpg-harness/src/scenarios/prespawned.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `prespawned_cue_is_preserved_when_server_confirms` | Crossbeam | Client predicts `PreSpawned` entity → server spawns matching → entity preserved after confirmation |
| `prespawned_cue_expires_when_server_does_not_confirm` | Crossbeam | Client predicts `PreSpawned` entity → server never spawns → entity despawned after timeout (80 ticks) |
| `client_prediction_drift_corrected_by_server` | Crossbeam | Client changes HP to 100, server has 90 → replication corrects client back to 90 |
| `udp_prespawned_cue_is_preserved_when_server_confirms` | UDP | PreSpawned entity preserved after server confirmation over UDP |
| `udp_prespawned_cue_expires_when_server_does_not_confirm` | UDP | PreSpawned entity expires when server never spawns over UDP |
| `udp_client_prediction_drift_corrected_by_server` | UDP | Prediction drift correction via replication over UDP |

### RPG Scenarios (`crates/engine-rpg-harness/src/scenarios/rpg.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `networked_movement` | Crossbeam | Move input replicates; server and client positions match after 15 ticks |
| `death_respawn_cycle` | Crossbeam | Alice killed by Bob → CombatState.dead + DeadTimer → 30 ticks later respawned at spawn point with full HP |
| `status_effects_over_time` | Crossbeam | BurnEffect (5 dmg/tick × 10 ticks) ticks down correctly; effect removed after expiry |
| `status_effects_can_kill` | Crossbeam | BurnEffect on low HP → HP reaches 0 before all ticks expire → dead=true |
| `cooldown_and_resource_cost` | Crossbeam | Attack costs 30 mana (50→20); insufficient mana clears the second attack; BOB takes exactly 34 damage |
| `aoe_damage_hits_multiple_targets` | Crossbeam | Secondary AoE attack hits BOB (3u), CHARLIE (5u), DAVE (8u) equally; ALICE takes no self-damage |
| `udp_networked_movement` | UDP | Move input replicates over UDP; server and client positions match after 15 ticks |
| `udp_death_respawn_cycle` | UDP | Death → DeadTimer → respawn with full HP over UDP |
| `udp_status_effects_over_time` | UDP | BurnEffect ticks down correctly over UDP |
| `udp_cooldown_and_resource_cost` | UDP | Attack mana cost and insufficient-mana rejection enforced over UDP |

### PVP/PVE Combat (`crates/engine-rpg-harness/src/scenarios/combat.rs`, `combat/pvp.rs`, `combat/pve.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `pvp_1v1_melee_combat` | Crossbeam | Two players both attack → both take 34 damage (100→66) |
| `pvp_shield_blocks_all_attack_types` | Crossbeam | Shield blocks both primary and secondary attacks; BOB stays at 100 HP |
| `pvp_simultaneous_attacks_on_same_target` | Crossbeam | Two attackers on BOB → BOB takes 68 damage (100→32); neither attacker is targeted |
| `pvp_knockback_impulse` | Crossbeam | Attack applies knockback; BOB pushed away from ALICE |
| `pvp_aoe_hits_multiple_players` | Crossbeam | AoE from ALICE hits BOB and CAROL (both 100→75) |
| `pvp_cooldown_prevents_double_attack` | Crossbeam | AttackCooldown resource = 3 ticks → only one attack lands despite persistent ActionState |
| `pvp_team_no_friendly_fire` | Crossbeam | Same team → no damage dealt |
| `pvp_death_removes_from_combat` | Crossbeam | BOB killed (20→0 HP); stays dead; FirstPersonController removed via sync_dead_state |
| `pve_player_vs_enemy` | Crossbeam | Player attacks enemy (50 HP) → enemy takes 34 damage (50→16) |
| `pve_enemy_respawns` | Crossbeam | Enemy killed (20→0 HP) → DeadTimer → respawns at 10-tick delay with full HP |
| `pve_boss_multiple_phases` | Crossbeam | Boss with thresholds [70,30]: phase 1→2 at 66 HP, stays phase 2 at 32 HP, reaches phase 3 at 0 HP |
| `udp_pvp_1v1_melee_combat` | UDP | Two players both attack → both take 34 damage over UDP |
| `udp_pvp_shield_blocks_all_attack_types` | UDP | Shield blocks primary and secondary attacks over UDP |
| `udp_pvp_team_no_friendly_fire` | UDP | Same-team damage prevention over UDP |
| `udp_pve_player_vs_enemy` | UDP | Player attacks enemy; enemy takes 34 damage over UDP |

### Doors (`crates/engine-rpg-harness/src/scenarios/doors.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `door_opens_on_grab` | Crossbeam | Use action on door → server spawns DoorGrab → door opens, player pulled toward it; PreSpawned DoorGrab persists on client |
| `locked_door_rejects_grab_and_cleans_up` | Crossbeam | Locked door → no DoorGrab spawned → PreSpawned entity despawns after timeout; door remains locked+closed |
| `udp_door_opens_on_grab` | UDP | Door opens on grab interaction over UDP |

### Adversarial (`crates/engine-rpg-harness/src/scenarios/adversarial.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `queue_action_defers_until_intended_tick` | Crossbeam | Input queued for tick 50 → not processed by tick 5 (HP stays 100) |
| `nan_inf_action_values_clamped` | Crossbeam | NaN/Inf Move axis → position stays finite and within bounds |
| `zero_damage_attack_noop` | Crossbeam | Attack at 20u distance (out of 8u range) → no damage dealt |
| `udp_queue_action_defers_until_intended_tick` | UDP | Input queued for tick 50 not processed before its intended tick over UDP |
| `udp_nan_inf_action_values_clamped` | UDP | NaN/Inf Move axis clamped over UDP |
| `udp_zero_damage_attack_noop` | UDP | Out-of-range attack no-ops over UDP |

### Stress (`crates/engine-rpg-harness/src/scenarios/stress.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `replicate_many_physics_entities` | Crossbeam | 50 entities spawn-replicated → all present on client; persist after 10 ticks |
| `udp_replicate_many_entities` | UDP | 50 entities spawn-replicated over UDP → all present on client; persist after 10 ticks |

### Full-Stack UDP (Client-to-Server Input) (`crates/engine-rpg-harness/src/scenarios/udp_scenarios/full_stack.rs`)

| Test | Transport | What it proves |
|---|---|---|
| `udp_full_stack_movement_over_network` | UDP | Client sends `ActionState::Move` via Lightyear `MessageSender` over UDP → server receives via `MessageReceiver` and applies to authoritative entity → `move_players` in `FixedUpdate` moves the entity |
| `udp_full_stack_combat_over_network` | UDP | Client sends `ActionState::AttackPrimary` via Lightyear over UDP → server applies to authoritative entity → `resolve_attacks` in `FixedUpdate` deals exactly `ATTACK_DAMAGE` (34) to nearby target; cleared after one tick to prove single-attack damage |
| `udp_full_stack_shield_blocks_attack` | UDP | Two clients over UDP: client 1 sends `RaiseShield` for Bob, the server observes the shield, then client 0 sends `AttackPrimary` for Alice → shield blocks attack → Bob survives at 100 HP |

### Native Leafwing Input (`crates/engine-rpg-harness/src/scenarios/native_input.rs` and `udp_scenarios/native_input.rs`)

This is the modern Lightyear Leafwing input path. Instead of sending
`ActionState` as a manual message, the harness installs
`lightyear::prelude::input::leafwing::InputPlugin::<AfterglowAction>` and relies
on Lightyear's native input buffering, timeline sync, and fixed input delay. The
client writes desired input into `FixedPreUpdate` at
`InputSystems::WriteClientInputs`, inserts `default_gameplay_input_map()` on
controlled client entities, and configures
`InputTimelineConfig::default().with_input_delay(InputDelayConfig::fixed_input_delay(2))`
on each client link after connect. Inputs arrive at the server after the
fixed delay, through Lightyear's own input messages — no manual
`MessageSender<ActionState<AfterglowAction>>`.

| Test | Transport | What it proves |
|---|---|---|
| `native_input_infrastructure_setup` | Crossbeam | InputPlugin registers channels/timeline, InputMap insertion produces LeafwingBuffer, entity mapping works over Crossbeam |
| `native_input_local_client_state` | Crossbeam | `apply_desired_input` at `WriteClientInputs` sets client `ActionState` axis through the Leafwing pipeline |
| `udp_native_input_movement_over_network` | UDP | Client writes `Move` axis through native Leafwing buffers → server receives after fixed delay and moves authoritative entity; predicted client entity also moves |
| `udp_native_input_combat_over_network` | UDP | Client writes `AttackPrimary` through native Leafwing → server deals exactly `ATTACK_DAMAGE` (34) to nearby target |
| `udp_native_input_shield_blocks_attack` | UDP | Two clients: Bob's `RaiseShield` reaches server through native path → Alice's `AttackPrimary` blocked → Bob survives at 100 HP |
| `udp_native_input_edges_arrive_once` | UDP | Jump press and release edges fire exactly once on the server through native input; just-pressed and just-released each count 1; final state is released |

Full end-to-end input delivery over Crossbeam is not supported because
Lightyear's leafwing input path requires the Netcode connection lifecycle to
fully wire the message sender/receiver for the input message type. Crossbeam
tests verify the input pipeline infrastructure (timeline sync, component setup,
local input buffering) which relies on the same `InputPlugin`,
`InputSystems::WriteClientInputs`, and `InputTimelineConfig` code paths.

The edge test assumes Lightyear preserves `just_pressed`/`just_released` flags
from received Leafwing snapshots until gameplay reads `ActionState` in
`FixedUpdate`. The shield-blocking test depends on the scenario system chain
running `resolve_shields` before `resolve_attacks`, so an expiring held shield is
renewed before the attack resolver checks it.

## How to Extend

### Adding a new scenario

1. Create `crates/engine-rpg-harness/src/scenarios/my_scenario.rs` gated with `#[cfg(test)]`
2. Add `pub mod my_scenario;` under `#[cfg(test)]` in `crates/engine-rpg-harness/src/scenarios/mod.rs`
3. Define a `register_my_scenario(app, role)` function that registers components and adds systems
4. Write `#[test]` functions that create `LightyearTestRig::new(N, plugins, register_my_scenario).with_input_delay_ms(50)`
5. Use `spawn_replicated`, `queue_action`, `advance_to`, and assertions via `server_component`/`client_component`

### Adding a new component

1. Add the type to `crates/engine-rpg-harness/src/scenarios/components.rs` with `Component + Clone + Copy/Debug + PartialEq + Serialize + Deserialize`
2. Register in your scenario's protocol function: `app.register_component::<MyComponent>().add_prediction()`
3. Add `add_prediction()` if the component needs client-side prediction; omit if server-only
4. For systems that operate on the component, add them to `FixedUpdate` in the desired chain ordering

### Adding a new transport

Currently supports Crossbeam and UDP. To add a new transport (e.g., WebSocket/WebTransport):
1. Add a variant to `TransportConfig` in `rig.rs`
2. Implement a new constructor like `new_websocket(...)` that configures apps with the appropriate Io/transport types
3. Handle `connect()` if the transport needs explicit connection establishment
4. Wire link entities with the correct Lightyear transport components

### Known limitations

- **Camera reconciliation:** No camera entity or view system is tested — the harness focuses on server-authoritative state and client prediction of game components, not rendering.
- **Disconnect handling:** No tests simulate client disconnect/reconnect mid-session.
- **Lag compensation:** Historical collider queries are intentionally not part of this baseline. The harness relies on client prediction, deterministic simulation, fixed server input delay, and Lightyear reconciliation.

## API Reference

### LightyearTestRig public methods

| Method | Signature | Description |
|---|---|---|
| `new` | `(client_count, plugins, register_protocol) -> Self` | Creates rig with Crossbeam transport |
| `new_with_transport` | `(client_count, plugins, register_protocol, TransportConfig) -> Self` | Creates rig with explicit transport |
| `connect` | `(&mut self)` | Establishes UDP netcode handshake; no-op for Crossbeam |
| `spawn_replicated` | `(&mut self, StableEntityId, impl Bundle) -> Entity` | Spawns on server with replication markers; triggers immediate push |
| `queue_action` | `(&mut self, tick, FnOnce(&mut App) + 'static)` | Queues server action at tick+delay |
| `queue_action_at_deliver_tick` | `(&mut self, intended_tick, deliver_at, FnOnce)` | Queues action with explicit delivery tick (bypasses delay) |
| `advance` | `(&mut self, delta: u32)` | Steps all worlds by delta ticks |
| `advance_to` | `(&mut self, target: u32)` | Steps to absolute tick (no-op if behind) |
| `current_tick` | `(&self) -> u32` | Returns current tick |
| `server_link` | `(&self, client_id) -> Entity` | Server-side connection entity |
| `client_link` | `(&self, client_id) -> Entity` | Client-side connection entity |
| `server_world(_mut)` | `(&self/&mut self) -> &World/&mut World` | Server world access |
| `client_world(_mut)` | `(&self/&mut self, client_id) -> &World/&mut World` | Client world access |
| `server_component` | `(&self, Entity) -> Option<&C>` | Typed component read on server |
| `client_component` | `(&self, client_id, Entity) -> Option<&C>` | Typed component read on client |
| `find_client_entity` | `(&mut self, client_id, StableEntityId) -> Option<Entity>` | Query client world by StableEntityId |
| `register_entity` | `(&mut self, StableEntityId, Vec<Entity>)` | Store entity map [server, client0, client1, ...] |
| `server_entity` | `(&self, StableEntityId) -> Entity` | Lookup server entity by stable ID |
| `client_entity` | `(&self, StableEntityId, client_id) -> Entity` | Lookup client entity by stable ID |
| `with_input_delay_ms` | `(self, u32) -> Self` | Builder: set input delay in ms |
| `with_retention_window_ticks` | `(self, u32) -> Self` | Builder: set stale-input retention window |

### Scenario module public types

| Path | Type | Description |
|---|---|---|
| `scenarios::components::*` | All game components | Re-exported for use in tests |
| `scenarios::systems::*` | All game systems + constants | `ATTACK_DAMAGE`, `AOE_DAMAGE`, `ATTACK_COOLDOWN_TICKS`, `KNOCKBACK_FORCE` |
| `scenarios::systems::door_grab_hash` | `fn(player, door) -> u64` | Deterministic hash for PreSpawned matching |

### Common test patterns

**Standard setup:**
```rust
let mut rig = LightyearTestRig::new(
    2,
    |app| { app.add_plugins(AfterglowPhysicsPlugin); },
    register_combat,
).with_input_delay_ms(50);

let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
let alice_c0 = rig.find_client_entity(0, ALICE).unwrap();
rig.register_entity(ALICE, vec![alice, alice_c0]);
```

**Queue delayed action:**
```rust
rig.queue_action(1, |app| {
    let mut state = ActionState::<AfterglowAction>::default();
    state.press(&AfterglowAction::AttackPrimary);
    app.world_mut().entity_mut(alice).insert(state);
});
```

**Advance and assert:**
```rust
rig.advance_to(DELIVERY_TICK);
assert_eq!(rig.server_component::<Health>(bob).unwrap().current, 66);
```

**PreSpawned pattern:**
```rust
let hash = door_grab_hash(PLAYER, DOOR);
let client_link = rig.client_link(0);
let predicted = rig.client_world_mut(0).spawn((
    DoorGrab { player: PLAYER, door: DOOR },
    PreSpawned::new(hash).for_receiver(client_link),
)).id();
```
