# Engine Game Stand-In Plan

**Build** a new `crates/engine-rpg-harness/` from scratch around fixed input
delay instead of rewind. Keep the existing `crates/mock-rpg-network-tests/` as
a frozen regression oracle until the new harness achieves scenario parity, then
delete it.

Client sends only `ActionState<AfterglowAction>` per tick (including Look as a
dual-axis). Server runs the same deterministic simulation, derives position/aim
from the simulated state, resolves targeting via distance checks (not raycasts).
Lightyear handles prediction/reconciliation.

**Server uses fixed input delay instead of rewind.** Inputs are queued and
processed N ticks later (configurable ms delay converted to ticks). By the time
the server processes tick T, all inputs for T have arrived — no rewind needed.
At 60Hz with 50ms delay = 3 ticks server lag, negligible for co-op PvE.

`HistoryTick` resource tracks tick for combat system ordering. The old
`ServerRewindPlugin` and typed component history API have been removed.

---

## Current State (2026-06-02)

**`crates/engine-rpg-harness/` is fully built.** 80 tests across rig,
controller, and scenario modules. The old `crates/mock-rpg-network-tests/` still exists but has been
superseded in coverage.

### What exists

| Module | Tests | Status |
|---|---|---|
| `rig.rs` — `LightyearTestRig` | — | [x] Crossbeam + UDP transport, tick stepping, input delay, retention window |
| `lib.rs` unit tests | 6 | [x] Rig boots, input delay, ActionState message, UDP connect/message/entity replication/link lifecycle |
| `controller.rs` | 5 | [x] Move, jump, look, crouch, wall collision |
| `scenarios/gold.rs` | 2 | [x] Shield blocks attack (Crossbeam + UDP) |
| `scenarios/lockstep.rs` | 12 | [x] Duplicate, reorder, delayed input, retention boundary, same-tick ordering, HP clamp — includes 5 UDP variants in `scenarios/udp_scenarios/` |
| `scenarios/adversarial.rs` | 6 | [x] Future-tick rejection, NaN/INF clamp, zero-damage noop — includes 3 UDP variants in `scenarios/udp_scenarios/` |
| `scenarios/prespawned.rs` | 6 | [x] Cue confirmed, cue expired, prediction drift correction — includes 3 UDP variants in `scenarios/udp_scenarios/` |
| `scenarios/physics.rs` | 4 | [x] Moving platform, character pushes dynamic, projectile, multi-player collision (local-only, no UDP needed) |
| `scenarios/rpg.rs` | 10 | [x] Networked movement, death/respawn, burn DoT, kill via DoT, cooldown+mana, AoE — includes 4 UDP variants in `scenarios/udp_scenarios/` |
| `scenarios/combat.rs` + `combat/{pvp,pve}.rs` | 15 | [x] PvP 1v1, shield blocks all, simultaneous, knockback, AoE, cooldown, no FF, death removal, PvE enemy, enemy respawn, boss phases — includes 4 UDP variants in `scenarios/udp_scenarios/` |
| `scenarios/doors.rs` | 3 | [x] Door opens on grab, locked door rejects — includes 1 UDP variant in `scenarios/udp_scenarios/` |
| `scenarios/stress.rs` | 2 | [x] Replicate many entities — includes 1 UDP variant in `scenarios/udp_scenarios/` |
| `scenarios/udp_scenarios/full_stack.rs` | 3 | [x] UDP client-to-server `ActionState<AfterglowAction>` delivery drives authoritative movement, combat, and shield-blocking systems |
| `scenarios/native_input.rs` | 2 | [x] Native Lightyear Leafwing input over Crossbeam: infrastructure setup (timeline sync, LeafwingBuffer, entity mapping) and local client-state buffering via `WriteClientInputs` |
| `scenarios/udp_scenarios/native_input.rs` | 4 | [x] Same native input tests over real UDP sockets: movement, combat, shield blocking, and press/release edge semantics without manual `ActionState` messages |

**Total: 80 tests, 0 benchmarks**

### What does NOT exist yet

- Main-path lag compensation plugin; historical collider queries remain
  prototype-only research
- PreSpawned cue entities for hit markers, damage numbers, beam trails (only
  generic PreSpawned and door-grab interaction tests exist)
- World + persistence systems
- Full netcode UDP scenarios beyond the gold path and focused rig-level UDP replication checks — now resolved with 28 UDP scenario variants in `scenarios/udp_scenarios/`
- Server rewind/replay/correction APIs; the baseline intentionally uses fixed
  input delay instead

---

## Architecture

```
Client (predicts)                   Server (authoritative)

Tick N:  ActionState → simulation   queues delayed ActionState
         show predicted result      processes queue[N] at tick N+delay

Tick N+1: ActionState → step        updates authoritative state
         receive confirmed[N]       sends confirmed state to client
         reconcile prediction
```

Lightyear's Leafwing input bridge sends ticked `ActionState` bundles (all button
and axis state including Look). The server reconstructs `ActionState` at each tick
and runs the same controller/physics simulation. Player position, yaw/pitch, and
derived aim vector come from the simulated state, not from client messages.

---

## Core Abstractions — Two Layers

### Low-level: `LightyearTestRig`

Infrastructure only. No RPG concepts. Owns:
- Server `App` + N client `App`s
- Crossbeam transport wiring per client
- UDP transport support (`TransportConfig::Udp`)
- Protocol registration callback per role
- Tick stepping (`advance`, `advance_to`)
- Input delivery (`queue_action`, `queue_action_at_deliver_tick`)
- Input delay (`with_input_delay_ms`)
- Retention window for stale input rejection (`with_retention_window_ticks`)
- Entity replication (`spawn_replicated`)
- Entity inspection (`server_component`, `client_component`, `find_client_entity`)
- StableEntityId entity mapping (`register_entity`, `server_entity`, `client_entity`)

```rust
let mut rig = LightyearTestRig::new(2, |app| {
    app.add_plugins(AfterglowPhysicsPlugin);
    app.add_plugins(AfterglowFirstPersonControllerPlugin);
}, |app, _role| {
    app.register_component::<Health>().add_prediction();
    app.register_component::<Transform>().add_prediction();
}).with_input_delay_ms(50);

let alice = rig.spawn_replicated(ALICE, player_bundle(Vec3::ZERO));
let alice_c0 = rig.find_client_entity(0, ALICE).unwrap();
rig.register_entity(ALICE, vec![alice, alice_c0]);

// Queue an input for tick 1 (delivered at tick 1 + delay_ticks)
rig.queue_action(1, |app| {
    set_action_state(app.world_mut(), alice, AfterglowAction::AttackPrimary);
});

rig.advance_to(10);

// Inspect server-simulated position
let pos = rig.server_component::<Transform>(alice).unwrap().translation;
```

### Higher-level: Scenario helpers

Per-scenario helpers in each module (no shared `Scenario` struct). Each test
module defines its own `player_bundle()`, `register_*()`, `set_action_state()`,
and inline scenario logic directly in test functions.

---

## Step Order — Implementation Status

### [x] Step 0: LightyearTestRig (infrastructure only)

**Status: Done.** `crates/engine-rpg-harness/src/rig.rs`.

- Boots N apps with `MinimalPlugins` + `AfterglowCorePlugin`
- Crossbeam transport wiring per client
- UDP transport (`TransportConfig::Udp`)
- Protocol registration via callback (runs after Lightyear plugins, before finish)
- Tick stepping: `advance(delta)` and `advance_to(target)` with a 10-step per-tick schedule pump
- Input delay via `with_input_delay_ms` → `queue_action`
- Retention window via `with_retention_window_ticks`
- Test: `rig_boots_and_delivers_message`, `input_delay_defers_server_processing`, `crossbeam_action_state_flows_as_custom_message`, `udp_rig_connects_and_delivers_message`

**Old mock RPG crate not yet deleted.**

### [x] Step 1: Look Is Part of ActionState — No Extra Messages

**Status: Done.** Implemented through `AfterglowAction::Look` dual-axis,
tested in `controller.rs:player_look_rotates_yaw`. Look axis is set via
`state.set_axis_pair(&AfterglowAction::Look, axis)` just like Move.

No separate `PlayerAim` or `ClientInput` message type — all actions flow through
`ActionState<AfterglowAction>`.

### [x] Step 2: Fixed Input Delay + Shared Tick

**Status: Done** (simplified vs plan).

**Input delay:** `LightyearTestRig::with_input_delay_ms(delay_ms)` converts to
ticks. `queue_action(tick, closure)` defers execution by `delay_ticks`. The
`advance()` loop drains pending inputs whose delivery tick ≤ current tick.
Proven by `input_delay_defers_server_processing`.

**Retention window:** `with_retention_window_ticks(n)` drops inputs whose
intended tick is outside `[current_tick - n, current_tick]`. Tested in
`lockstep.rs:input_within_delay_window_processed` and
`lockstep.rs:input_outside_delay_window_rejected`.

**Shared tick:** The harness uses a simple `HistoryTick` resource, incremented
each fixed tick, for combat system ordering. There is no generic component
history store and no server rewind plugin.

### [x] Step 3: Local Controller + Physics

**Status: Done.** `crates/engine-rpg-harness/src/controller.rs` uses
`AfterglowFirstPersonControllerPlugin` and `AfterglowPhysicsPlugin` directly
from the engine.

Tests:
- [x] Move forward → body translates
- [x] Jump → leaves ground, lands
- [x] Look rotates yaw/pitch
- [x] Crouch under low ceiling (stays crouching)
- [x] Wall collision stops movement
- [~] Breakable barrel breaks on impact — not implemented (no destructible prop system)

### [x] Step 4: Networked Physics — Gold Scenario

**Status: Done.** `scenarios/gold.rs` — Full stack: Lightyear + controller +
physics + fixed input delay. Two tests:

1. **Crossbeam:** Alice attacks Bob at tick 2, Bob raises shield at tick 1.
   Both queued with 50ms delay (~3 ticks). Shield processes first (tick 4),
   attack second (tick 5). Shield blocks, Bob lives, no corpse/loot. Client
   replicated state verified.
2. **UDP:** Same scenario over real UDP sockets with Lightyear entity
   replication and client state verification.

### [x] Step 5: Adversarial Lockstep Matrix

**Status: Done.** `scenarios/lockstep.rs` + `scenarios/adversarial.rs`:

| Category | Test | Status |
|---|---|---|
| Duplicate | Same tick double-heal rejected (marker check) | [x] `duplicate_same_tick_input_rejected` |
| Reorder | Reverse queue order → delivery by intended tick, not queue order | [x] `reordered_inputs_produce_same_result` |
| Drop + resend | Dropped tick 1, resent tick 3 → delivers at tick 6 | [x] `delayed_input_still_delivers` |
| Retention boundary | Input at retention edge → processed | [x] `input_within_delay_window_processed` |
| Retention stale | Input past retention floor → rejected | [x] `input_outside_delay_window_rejected` |
| Same-tick ordering | Shield + attack at same tick → resolve_shields before resolve_attacks | [x] `same_tick_shield_blocks_attack_ordered_correctly` |
| Server authoritative | Max HP cap enforced even with heal attempt | [x] `server_clamps_heal_to_max_hp` |
| Future tick rejection | Input at tick 50 not processed at tick 5 | [x] `queue_action_defers_until_intended_tick` |
| NaN/INF clamping | NaN/INF axis values clamped → position stays finite | [x] `nan_inf_action_values_clamped` |
| Zero-damage attack | Out-of-range attack no-ops | [x] `zero_damage_attack_noop` |

Remaining from plan matrix:
- [ ] Move validation — impossible move rejection under latency
- [ ] Ownership — Client A cannot drive Client B's entity
- [ ] Empty ticks — tick with no input → hold last or neutral

### [x] Step 6: Combat, RPG, Physics, Doors, Stress Scenarios

**Status: Done** (expanded beyond original plan with scenario modules not
in the original 8-step plan).

- [x] **Combat** (`combat.rs`, `combat/pvp.rs`, `combat/pve.rs`): PvP 1v1, shield blocks all, simultaneous
  double-attack, knockback impulse, AoE multi-hit, cooldown, no friendly fire,
  death removal, PvE enemy, enemy respawn, boss phases
- [x] **RPG** (`rpg.rs`): Networked movement, death/respawn cycle, burn DoT,
  lethal DoT, cooldown+mana cost, AoE damage
- [x] **Physics** (`physics.rs`): Moving platform, character pushes dynamic,
  projectile flight & collision, multi-player collision separation
- [x] **Doors** (`doors.rs`): PreSpawned door grab, locked door rejection
- [x] **Stress** (`stress.rs`): Replicate 50 entities across server/client
- [x] **Prespawned** (`prespawned.rs`): Cue preserved on confirm, expired on
  no-confirm, prediction drift corrected by server

### [~] Optional Research: Lag Compensation + Presentation Cues

**Status: Optional research.** PreSpawned cue entities work (proven in
`prespawned.rs` and `doors.rs`). Physics lag compensation is intentionally not
part of the main harness; projectile collision in `physics.rs` uses current
simulation state, not historical queries.

Optional future work:
- [ ] `LagCompensationPlugin` for FPS/twitch fairness research only
- [ ] Hit markers, damage numbers, beam trails as PreSpawned cue entities if a
  presentation scenario needs them

### [x] Step 7: UDP / Netcode

**Status: Expanded.** In addition to the baseline rig-level UDP tests and the
gold UDP scenario, `scenarios/udp_scenarios/` now provides **28 UDP scenario variants**
covering lockstep (5), adversarial (3), RPG (4), PreSpawned (3), combat (4),
doors (1), stress (1), full-stack client-to-server input (3), and native Leafwing
input (4) — all over real netcode UDP sockets. The full scenario suite now
exercises both Crossbeam and UDP transports.

### [ ] Step 8: World + Persistence

**Status: Not started.** Depends on engine world/persistence modules.

---

## What Gets Deleted (after parity)

Only delete `crates/mock-rpg-network-tests/` when every scenario below passes
in the new harness:

- [x] Late shield blocks death + loot + pickup (golden)
- [x] Duplicate/reordered/dropped input edge cases
- [x] Retention boundary & staleness
- [x] Same-tick deterministic ordering
- [~] Move validation + spell range under latency (basic range check exists)
- [x] PreSpawned grab confirm + cleanup
- [ ] Console connect/disconnect/latency/stats
- [ ] Sync components removes stale

Plan at `docs/research/mock-rpg-standin-plan.md` (current).
