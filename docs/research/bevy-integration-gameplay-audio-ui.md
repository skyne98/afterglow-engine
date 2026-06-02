# Bevy Integration: Gameplay, Animation, Audio, AI, And UI

## Scope

This note maps roadmap gameplay systems onto Bevy `0.18.1` and the current local code.

Current local state:

- [demo.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/demo.rs:1) has no player controller, only opt-in demo animation and demo cell installation.
- [perf_hud/ui.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/perf_hud/ui.rs:59) is the main local UI example.
- [perf_hud/ui.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/perf_hud/ui.rs:222) already uses `ButtonInput<KeyCode>` for HUD toggling.
- [lib.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/lib.rs:96) disables Bevy audio on wasm.

## Input And Controller

Add an engine-owned action layer before controller logic.

Useful Bevy sources:

- `bevy_input-0.18.1/src/button_input.rs`
- `bevy_input-0.18.1/src/keyboard.rs`
- `bevy_input-0.18.1/src/mouse.rs`
- `bevy_input-0.18.1/src/gamepad.rs`
- `bevy_window-0.18.1/src/cursor/*`

Do not let gameplay read raw device state directly. Convert it to Leafwing action
state on the controlled entity:

```rust
Query<&ActionState<AfterglowAction>>
```

This keeps controller code compatible with local simulation, Lightyear prediction,
fixed server input delay, and tests.

## Physics And Interaction

The engine now integrates Avian through `AfterglowPhysicsPlugin`. Game-facing
code can start from the generic `PhysicsBody`, `PhysicsCollider`, and
`PhysicsVelocity` authoring components, or use
`afterglow_engine::physics::avian::*` directly for backend-specific features.

Interaction should use semantic commands:

- `UseCommand`
- `PickupCommand`
- `DoorCommand`
- `ContainerCommand`
- `TriggerCommand`

Useful Bevy sources for early prototypes:

- `bevy_camera-0.18.1/src/camera.rs`
- `bevy_picking-0.18.1/src/mesh_picking/ray_cast/mod.rs`

Use Bevy camera rays and mesh picking only for quick prototypes. Final gameplay should use physics queries so player use, AI sight, projectile collision, and sound occlusion agree.

## Inventory, Equipment, And Effects

Keep item logic independent from UI.

Start with pure ECS/data assets:

```rust
pub struct Inventory;
pub struct ItemStack;
pub struct EquipmentSlots;
pub struct Equipped;
pub struct Condition;
pub struct EffectInstance;
pub struct StatModifier;
```

Flow:

1. UI/input emits commands.
2. Gameplay systems validate commands.
3. Inventory/equipment/effects mutate ECS state.
4. Audio, animation, UI, and networking react to events/deltas.

This works for save/load and replication because state changes are explicit.

## Animation And Skinned Proxies

Use Bevy animation first.

Useful Bevy sources:

- `bevy_animation-0.18.1/src/lib.rs`
- `bevy_animation-0.18.1/src/graph.rs`
- `bevy_animation-0.18.1/src/transition.rs`
- `bevy_mesh-0.18.1/src/skinning.rs`
- `bevy_pbr-0.18.1/src/render/skin.rs`
- `bevy_pbr-0.18.1/src/render/skinning.wgsl`

Plan:

1. Use `AnimationPlayer` and `AnimationGraph` for clips/blends/masks.
2. Track gameplay animation state separately from Bevy player internals.
3. Attach occludee/occluder proxies to joint entities for skinned meshes.
4. Extract bone-attached proxy transforms after transform propagation.
5. Use OR visibility across skinned occludee proxies.

Do not use Bevy's static mesh AABB as the source of skinned occlusion correctness.

## Audio And Steam Audio

Bevy audio is useful as a temporary playback API, not the final stealth audio model.

Useful Bevy sources:

- `bevy_audio-0.18.1/src/audio.rs`
- `bevy_audio-0.18.1/src/audio_output.rs`
- `bevy_audio-0.18.1/src/sinks.rs`

Engine API should be backend-neutral:

```rust
pub struct SoundEmitter;
pub struct SoundListener;
pub struct SoundMaterial;
pub struct AudioOccluder;
pub struct AudioProbeBatch;
pub struct NoiseEvent;
```

Chunk data should own:

- audio geometry
- acoustic material IDs
- Steam Audio probe batches
- occluder state

Bevy audio can play temporary emitter sounds, but Steam Audio or `audionimbus` should own ray-traced occlusion/propagation when that system arrives. On wasm, audio needs explicit startup because current local code disables `AudioPlugin`.

## AI Perception

Render visibility is not gameplay visibility.

Useful Bevy source:

- `bevy_camera-0.18.1/src/visibility/mod.rs`

AI perception should combine:

- physics line/shape casts
- light exposure or stealth visibility sample
- propagated `NoiseEvent`s
- faction relation
- memory/suspicion state

Suggested data flow:

```text
movement/doors/items/weapons -> NoiseEvent
lights/fog/cover -> VisibilitySample
physics queries -> LineOfSight
factions -> Relation
AI system -> Suspicion / AlertState
```

Use render debug visibility only as an overlay, not as an AI truth source.

## UI, HUD, And Diegetic Interfaces

Build immediate debug UI using Bevy UI, following the existing perf HUD.

Useful Bevy sources:

- `bevy_ui-0.18.1/src/ui_node.rs`
- `bevy_ui-0.18.1/src/focus.rs`
- `bevy_text-0.18.1/src/*`

Local example:

- [perf_hud/ui.rs](/home/fox/Project/afterglow-engine/crates/afterglow-engine/src/perf_hud/ui.rs:59)

Plan:

1. Keep debug overlays in Bevy UI.
2. Keep gameplay UI state in ECS resources/components.
3. Let UI emit commands, not mutate gameplay directly.
4. Build diegetic interfaces as world-space panels or render-to-texture later.

## Implementation Order

1. Leafwing input action mapping.
2. Lightyear Leafwing input networking.
3. First-person body on top of Avian-backed physics.
4. Interaction commands and physics ray/query.
5. Inventory/equipment/effects data model.
6. Animation graph wrapper and gameplay animation state.
7. Bone-attached skinned proxy extraction.
8. Backend-neutral audio emitter/material/probe API.
9. Noise/light/LOS AI perception.
10. HUD/debug UI, then diegetic UI.
