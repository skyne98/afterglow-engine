# Local Physics Prediction + Rollback Architecture

**Status:** Implementation checklist in progress
**Date:** 2026-06-27

## Problem

When the local client pulls a block with a rope, the block should be simulated
locally at the fixed simulation rate and rendered smoothly at the render rate.
Server packets should not snap the block's current pose. They should update the
confirmed state for an older tick; Lightyear should then rollback to that tick,
replay deterministic simulation to the current tick, and apply visual correction
if the replayed pose differs.

Observed symptom: the local pulling client can see the block jitter, as if the
block moves at the replication/fixed snapshot rate rather than being fully local
and render-smoothed.

## Primary-source findings

### 1. Lightyear prediction already implements the desired model

Lightyear `lightyear_prediction-0.26.4` does the correct prediction loop:

- `RollbackSystems::Check` runs after `ReplicationSystems::Receive` in
  `PreUpdate` and checks confirmed components against prediction history.
- On mismatch, it sets a rollback tick from the confirmed server tick.
- `prepare_rollback::<C>` restores predicted components to the confirmed value
  for that old tick and records `PreviousVisual<C>` when correction is enabled.
- `run_rollback` re-runs `FixedMain` from `rollback_tick + 1` to the current
  local tick.
- `update_frame_interpolation_post_rollback` and
  `add_visual_correction::<C, D>` smooth the visual error after replay.

This is exactly the required architecture: server updates are past-tick facts,
not current-frame presentation commands.

### 2. Afterglow registers the correct networked physics state

`register_afterglow_lightyear_protocol` registers:

- `Transform` with prediction, linear correction, and interpolation;
- `LinearVelocity` with prediction;
- `StableEntityId` with prediction.

The Avian bridge makes `Transform` the canonical networked pose and treats
Avian `Position`/`Rotation` as local physics internals. During fixed simulation,
Transform is synced to Avian before physics and Avian writes Transform back
after physics; prediction history is recorded after writeback.

This is compatible with deterministic physics: rollback restores
`Transform`/`LinearVelocity`, Avian derives local internals, physics replays,
and the current tick result is deterministic.

### 3. The current manual confirmed-state sync violates the model

`sync_remote_predicted_confirmed_transforms` directly copies
`Confirmed<Transform>` and `Confirmed<LinearVelocity>` into the predicted
entity's live `Transform`, Avian `Position`/`Rotation`, and `LinearVelocity`:

```rust
*transform = confirmed.0;
position.0 = confirmed.translation;
rotation.0 = confirmed.rotation;
velocity.0 = confirmed_velocity.0.0;
```

For a locally rope-driven block, this applies a past server pose to the current
local physics body whenever a confirmed update arrives. That bypasses
Lightyear's rollback/replay/visual-correction path and produces visible jitter.

### 4. The manual sync still has a limited compatibility use

The multiplayer boxes demo currently predicts players/boxes to all clients, but
only the server and the owning predicted client create physical rope joints.
Non-owning clients keep remote ropes visual-only. Therefore a non-owning client
cannot locally derive a remote player's rope-pulled block purely from inputs;
it needs server-confirmed state for remote presentation.

The clean split is:

- **Local deterministic driver**: never hard-copy confirmed state into the live
  current tick. Let Lightyear rollback/replay/correct.
- **Remote/nonlocal presentation compatibility**: server-confirmed state may be
  mirrored into visible predicted presentation when the client is not locally
  driving that body.

This preserves immediate local feel while retaining current remote visibility.

## Architecture rules

1. Server-confirmed state is historical input to prediction, not a current-frame
   imperative, for any predicted deterministic physics body.
2. Gameplay/demo code should look like local multiplayer on one machine:
   spawn bodies, read input, run deterministic physics. It should not know which
   pose came from a packet and should not copy `Confirmed<T>` into live gameplay
   components. Local-only prediction lifecycle state is acceptable when it is
   input/ownership state (for example, hiding an already-active locally released
   rope until authoritative despawn catches up), not packet-to-pose copying.
3. Networking-aware correction belongs in the engine Lightyear/physics bridge:
   protocol registration, prediction history, rollback triggers, replay, and
   visual correction.
4. Hard confirmed-to-live sync is forbidden in gameplay systems for predicted
   physics entities. If a compatibility bridge is ever needed, it must be an
   engine/test-harness adapter with a documented removal path, not demo logic.
5. Long term, if all clients are meant to predict all rope physics, then all
   clients must create the same deterministic rope joints and rollback from the
   replicated rope spawn tick. Until that is implemented, non-owner ropes may be
   presentation-only, but owner/local prediction must still be pure local
   deterministic physics plus rollback.

## Implementation checklist

- [x] Remove the gameplay-level manual confirmed-to-live sync system from the
      multiplayer boxes client path.
- [x] Remove unit tests that encoded direct confirmed-to-live copying as desired
      behavior.
- [x] Add an engine-level rollback-check guard for confirmed tick advancement so
      delayed/buffered confirmed state triggers Lightyear rollback instead of
      requiring gameplay pose copying.
- [x] Isolate the remaining manual Crossbeam visibility bridge as harness-only
      test code; it is not installed by `MultiplayerBoxesClientPlugin` and is
      documented as a removal target.
- [x] Add/extend a production UDP regression that exercises rope-pulled block
      local prediction, release while moving away, repeated stale reappearance
      suppression, and duplicate PreSpawned hash prevention.
- [x] Re-run multiplayer boxes unit tests and focused Crossbeam/UDP
      visible-transform regressions.
- [x] Run current workspace check and full serial harness after removing the
      gameplay sync bridge. Focused rope/block UDP coverage now exists; rerun
      full workspace check after the rope-release fix before merging.

## Verification so far

- `nix develop -c cargo test -p afterglow-engine --features test-support -- demos::multiplayer_boxes -- --nocapture` ✅
- `nix develop -c cargo test -p engine-rpg-harness multiplayer_boxes -- --test-threads=1 --nocapture` ✅
- `nix develop -c cargo test -p engine-rpg-harness udp_rope_pull_then_release_while_moving_away_does_not_reappear -- --nocapture` ✅
- `nix develop -c cargo check --workspace --features test-support` ✅
- `nix develop -c cargo test -p engine-rpg-harness -- --test-threads=1 --nocapture` ✅ (111 passed, 1 ignored after one transient corners rerun)

## Current implementation notes

- `MultiplayerBoxesClientPlugin` no longer installs any system that copies
  `Confirmed<Transform>` into live predicted physics bodies.
- `AfterglowLightyearPlugin` installs
  `request_rollback_check_on_confirmed_tick_changed`, an engine-level guard that
  notices confirmed-tick advancement on predicted entities and requests a
  Lightyear rollback check. This addresses delayed/buffered replication without
  gameplay code knowing about confirmed components.
- The manual Crossbeam regression harness still has a harness-only visibility
  adapter because its hand-stepped schedule does not yet deterministically drive
  all state rollback/correction cases the same way production UDP does. This is
  not part of game/demo runtime architecture.
- Local rope release now has two guarded prediction pieces:
  `LocallyReleasedRopes` suppresses stale active rope reappearances by
  deterministic `rope_id`, and `hide_local_rope_on_physical_release` hides an
  already-active owning-client rope immediately when the physical rope key is
  released. The authoritative release still travels through Lightyear input;
  the local hider only prevents visible flicker while delayed/rollback state
  catches up.

## Open follow-up

A stricter future architecture can remove the harness adapter entirely by making
both the manual Crossbeam rig and all-clients rope physics run through the same
production rollback path. All-clients rope physics requires rollback-safe joint
creation at the replicated rope spawn tick, deterministic joint handles, and
tests proving non-owner replay converges without server pose mirroring.
