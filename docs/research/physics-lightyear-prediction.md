# Physics Lightyear Prediction And Optional Lag Compensation

**Date:** 2026-05-20
**Crates:** `engine-rpg-harness`, `prototype-physics-lightyear`, legacy `mock-rpg-network-tests`

**Status:** PreSpawned prediction is part of the main harness. Physics lag
compensation is prototype-only research and is not part of the current engine or
`engine-rpg-harness` baseline.

## Goal

Prove that physics interactions can use Lightyear-native prediction and
server-authoritative reconciliation instead of a custom entity mapping layer. A
separate prototype proves that Lightyear Avian lag compensation can query
historical collider state, but that path is optional research only.

## Pre-Spawned Grab Constraints

The current `engine-rpg-harness` proves PreSpawned confirmation/expiration in
`crates/engine-rpg-harness/src/scenarios/prespawned.rs` and door interaction
prediction in `crates/engine-rpg-harness/src/scenarios/doors.rs`. The legacy
`mock-rpg-network-tests` crate still has an older two-client/one-server physics
grab regression in `src/network_e2e/physics_grab.rs`.

Scenario:

1. The server starts with a grabbable sphere at `GRAB_POINT`.
2. Client B sends an authoritative command that creates a box at the same point.
3. Client A has not seen the box yet and predicts a grab against the sphere.
4. Client A immediately spawns a local `GrabConstraint` with
   `PreSpawned::new(grab_hash(player, sequence)).for_receiver(client_a_link)`.
5. The server sorts commands by `(tick, player, sequence)`, resolves the grab
   against its authoritative body set, and replicates accepted constraints with
   the same `PreSpawned` hash plus `Replicate` and `PredictionTarget`.

Covered outcomes:

- If the unseen box is grabbable, server authority wins and the predicted entity
  is preserved but corrected from `target: SPHERE` to `target: BOX` through
  `Confirmed<GrabConstraint>`.
- If the unseen box is non-grabbable, the server rejects the grab, no matching
  pre-spawned entity arrives, and Lightyear cleans up the unmatched client entity
  after the pre-spawn timeout.

This proves the right pattern for transient predicted interaction entities:
predict an entity with `PreSpawned`, let the server reuse the hash only when the
interaction is accepted, and let Lightyear handle matching and expiration.

## Twitch Spellcasting Mapping

The same primitives map directly to twitch PvP spells, but the server must remain
authoritative. Client prediction is a responsiveness tool, not a trust boundary.

| Spell shape | Client behavior | Server behavior | Correction path |
|---|---|---|---|
| Hitscan ray/beam | Spawn local cast cue, muzzle flash, trail, and optional `PreSpawned` hit-marker entity. | Baseline: validate the delayed input tick and query current authoritative state for that tick. Optional research: historical ray query for FPS/twitch fairness. | Matching cue entity confirms it; no match lets Lightyear expire it. |
| Fast projectile | Spawn predicted projectile entity with `PreSpawned` and immediate visuals. | Validate input, spawn authoritative projectile with the same hash if accepted, simulate authoritative collision. | Matching `PreSpawned` confirms projectile; rejection expires it. |
| Grab/tether/link | Spawn predicted constraint/link entity with `PreSpawned`. | Validate range, line of sight, target state, and historical/currents checks as the spell requires. | Confirmation updates target/state; rejection removes the predicted link. |
| Area burst | Spawn local cast/impact cue immediately, but make final hit markers entity-backed or wait for confirmation. | Baseline: query candidates in the deterministic delayed-tick simulation. Optional research: historical overlap/swept shape. | Confirmed cue entities or replicated state drive final presentation. |

The important design rule is that the client may predict only presentation and
pending local gameplay state. Durable gameplay truth still comes from the server:
health, death, inventory, cooldown commits, projectile hit facts, and status
effects are authoritative outputs.

## Entity-Backed Presentation Cues

The near-term cue design is intentionally tiny: do not add a separate cue protocol
yet. If a predicted presentation effect needs server confirmation or cleanup, make
it an entity with a small cue component and Lightyear `PreSpawned` metadata.

Examples that should be entity-backed:

- hit markers
- floating damage numbers
- temporary beam/trail visuals
- impact decals or particles that should disappear on rejection
- looping or fadeable audio emitters

Flow:

```text
client predicts cue -> spawn cue entity with PreSpawned(hash).for_receiver(link)
server confirms cue -> spawn replicated cue entity with the same PreSpawned(hash)
server rejects cue -> spawn nothing; Lightyear expires the unmatched client entity
```

This keeps the correction semantics inside Lightyear's predicted entity matching.
The only engine rule is that presentation systems must key reversible visuals to
the cue entity so entity despawn means cancellation/fade. One-shot effects that
cannot be stopped should either wait for confirmation or be treated as harmless
local feedback.

## Optional Lag Compensation Guardrails

This section applies only if lag compensation is later promoted from prototype to
an optional FPS/twitch feature. It is not part of the current engine path.

Lag compensation reduces "I hit them on my screen but missed on the server". It
does not make lag disappear. The server is intentionally evaluating a historical
world, which can feel unfair to the target if the rewind window is too generous.

If adopted, production PvP should enforce these guardrails:

- Cap the rewind window per game mode and spell class.
- Reject commands older than retained history or newer than the server's accepted
  client timeline estimate.
- Tie command ticks to Lightyear input history, not arbitrary client-provided
  timestamps.
- Run historical line-of-sight and cover checks for hitscan and lock-on spells.
- Prefer current-state validation for effects that should not pass through newly
  closed doors, shields, invulnerability, or phase-state changes.
- Sort same-tick commands deterministically by stable player ID and sequence.
- Deduplicate commands by `(player, sequence)` before applying gameplay effects.
- Keep victim-facing feedback honest: if high-ping rewind hits someone already
  behind cover, the game needs conservative caps or cover-favoring rules.
- Never let the client submit final hit lists, damage numbers, or target entity
  IDs without server-side reconstruction.

Spell classes can choose different fairness rules. A slow PvE projectile can be
generous. A competitive hitscan spell should use a smaller rewind cap and stronger
cover validation. A co-op grab spell can favor the caster more than a PvP stun.

## Regression Envelope

Before promoting this from prototype to an optional spellcast pipeline, the
current harness should box in these edge cases:

- Normal hitscan hit, miss, and near-edge collider hit.
- Rewind exactly at the oldest retained tick and one tick older than retained
  history.
- Fractional interpolation delays between two collider samples.
- Duplicate cast command delivery.
- Reordered same-tick casts from different players.
- Stale cast after the target despawned or changed teams.
- Target hidden by cover at historical tick but visible currently, and the inverse.
- Caster predicted a projectile spawn that the server rejects for cooldown, mana,
  silence, stun, or invalid weapon state.
- Server accepts a predicted projectile/grab but corrects its target or spawn pose.
- Client receives the authoritative entity after the local `PreSpawned` timeout.
- Malicious client submits impossible cast ticks, impossible aim directions, or
  target IDs outside its visibility/interest set.

These tests should run through Lightyear link entities, not only pure functions,
because the important failure modes are schedule ordering, prediction metadata,
and entity matching.

## Code Map

| File | Why it matters |
|---|---|
| `crates/engine-rpg-harness/src/scenarios/prespawned.rs` | `PreSpawned` entity confirmation, expiration, and correction. |
| `crates/engine-rpg-harness/src/scenarios/doors.rs` | Door interaction prediction using `PreSpawned` entities. |
| `crates/mock-rpg-network-tests/src/network_e2e/physics_grab.rs` | Legacy end-to-end `PreSpawned` interaction entity prediction with two clients and one authoritative server. |
| `crates/prototypes/prototype-physics-lightyear/src/main.rs` | Minimal Lightyear Avian lag-compensation setup and historical ray-query tests. |
| `docs/api/network.md` | Current network API surface and the documented prediction/reconciliation pattern. |
| `docs/api/physics.md` | Current Avian integration notes and prototype verification commands. |

## Avian Lag Compensation

`crates/prototypes/prototype-physics-lightyear` now tests
`lightyear_avian3d::LagCompensationPlugin` with Avian colliders.

The prototype app:

- uses `PhysicsPlugins::new(FixedUpdate)` and `LagCompensationPlugin`
- initializes `LocalTimeline`
- spawns a `Server` entity
- marks the target collider with `LagCompensationHistory`
- records historical `Position` values by advancing `LocalTimeline` and running
  `PhysicsSchedule`

Covered outcomes:

- A ray can miss the current collider position while hitting the same collider at
  an older timeline tick via `LagCompensationSpatialQuery` and
  `InterpolationDelay`.
- Fractional interpolation delay returns the interpolated collider pose between
  two recorded ticks.

Important dependency note: `lightyear_avian3d 0.26.4` depends on `avian3d 0.5`,
so the prototype uses Avian 0.5 even though other engine physics prototypes may
use newer Avian versions.

## Optional Implementation Work

The prototype proves the primitives, not a complete spell system. If lag
compensation becomes an optional feature later, that layer would need:

- A `SpellCastCommand` or equivalent Lightyear input/message schema with stable
  caster identity, sequence, spell kind, aim ray, and input tick.
- Server-side command validation for cooldowns, resources, stance, silence, stun,
  weapon state, target filters, and interest/visibility constraints.
- A small historical query adapter that wraps `LagCompensationSpatialQuery` for
  common spell shapes: ray, sphere overlap, capsule sweep, and cone fan.
- Prediction helpers for spell and cue entities that should use `PreSpawned`
  hashes.
- Optional retained cue/fact logs later if rollback needs auditability beyond
  Lightyear's predicted entity confirmation and timeout cleanup.
- Adversarial integration tests for PvP latency, jitter, packet duplication,
  stale commands, and malicious timestamps.

## Verification

Focused checks:

```sh
cargo test --package engine-rpg-harness prespawned -- --nocapture
cargo test --package engine-rpg-harness doors -- --nocapture
cargo test --package prototype-physics-lightyear lag_compensation -- --nocapture
```

Broader checks already passed after the implementation:

```sh
cargo test --package engine-rpg-harness
cargo test --package mock-rpg-network-tests
cargo test --package prototype-physics-lightyear
```
