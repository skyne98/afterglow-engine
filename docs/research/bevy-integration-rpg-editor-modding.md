# Bevy Integration: RPG Data, Editor, And Modding

## Scope

This note maps the open-world RPG layer onto Bevy `0.18.1`.

It covers:

- factions, stats, skills, dialogue, quests
- procedural and authored placement
- editor/tooling
- modding through data and asset override layers

These systems depend on stable identity, chunk lifecycle, and explicit save/replication schemas from [bevy-integration-world-runtime.md](bevy-integration-world-runtime.md).

## RPG Data Model

Keep RPG state as explicit data, not ad hoc script state.

Core assets:

```rust
pub struct FactionAsset;
pub struct SkillAsset;
pub struct StatAsset;
pub struct EffectAsset;
pub struct DialogueAsset;
pub struct QuestAsset;
```

Core components:

```rust
pub struct FactionMember;
pub struct StatBlock;
pub struct SkillBlock;
pub struct ActiveEffects;
pub struct DialogueState;
pub struct QuestState;
```

Useful Bevy systems:

- `bevy_asset-0.18.1/src/assets.rs`
- `bevy_asset-0.18.1/src/server/mod.rs`
- `bevy_reflect-0.18.1/src/*`
- `bevy_ecs-0.18.1/src/change_detection.rs`

Use Bevy assets for definitions and ECS components for runtime state. Do not put mutable save-game state inside asset definitions.

## Factions And AI Hooks

Faction data should feed AI perception, dialogue, crime, stealth, and combat.

Suggested data:

```rust
pub struct FactionRelation {
    pub source: FactionId,
    pub target: FactionId,
    pub disposition: i16,
    pub hostile: bool,
}
```

Runtime systems:

1. evaluate relation
2. combine with sight/noise memory
3. update suspicion/alert state
4. emit dialogue/combat/flee commands

This should run in gameplay simulation, not UI or render code.

## Dialogue And Quests

Dialogue and quest data should be asset-backed and stable-ID aware.

Rules:

- dialogue nodes reference `StableEntityId`, `FactionId`, `QuestId`, not raw `Entity`
- quest state is saved per world/save, not inside `QuestAsset`
- dialogue conditions query ECS/RPG state through explicit evaluators
- dialogue actions emit commands/events, not direct mutations from UI

Useful Bevy systems:

- asset loading for dialogue/quest definitions
- reflection for editor inspection
- ECS events/messages for dialogue actions
- state schedules for menus/conversation mode if needed

## Procedural And Authored Placement

Placement should produce the same runtime records whether authored or generated.

Core data:

```rust
pub struct PlacementRecord {
    pub stable_id: StableEntityId,
    pub prefab: AssetPath,
    pub transform: Transform,
    pub chunk: ChunkId,
    pub seed: u64,
}
```

Flow:

1. authored placement loads from chunk manifest
2. procedural placement generates deterministic records from seed
3. both spawn through the same chunk lifecycle
4. save deltas override baseline placement

This avoids separate authored/procedural code paths.

## Editor And Tooling

Start with in-engine debug authoring, not a separate editor.

Tooling targets:

- chunk bounds and portals
- occludee/occluder proxies
- bone-attached skinned proxies
- light/probe volumes
- fog volumes
- VT page/residency debug
- audio geometry and Steam Audio probe batches
- persistent entity IDs
- placement records
- RPG data references

Useful Bevy systems:

- `bevy_reflect` for inspectable component data
- `bevy_gizmos` for visual authoring overlays
- Bevy UI for inspector panels
- asset hot reload for data iteration

Editor output should be normal engine assets/manifests, not special editor-only formats.

## Modding

Modding should be a data and asset override layer.

Priority order:

```text
base game assets
official patch assets
enabled mod assets in load order
save-game deltas
```

Bevy asset paths are useful, but Afterglow needs an engine-level resolver so mods can override:

- chunk manifests
- materials/textures
- dialogue/quest assets
- item definitions
- placement records
- audio material data

Do not let mods override raw Bevy entity IDs. Modded content should target stable IDs, asset paths, tags, or placement records.

## Save/Replication Compatibility

RPG/editor/modding data must obey the same durable schema rules:

- stable IDs for world objects
- asset IDs/paths for definitions
- explicit component deltas for mutable state
- tombstones for removed authored entities
- deterministic seeds for generated placement

If a system cannot save and replicate its state explicitly, it is not ready to become core gameplay.

## Implementation Order

1. Asset IDs and data types for factions/stats/skills/effects.
2. Stable-ID-aware placement records.
3. Chunk manifest schema includes placement, probes, fog, VT, audio, and proxies.
4. In-engine gizmo/debug UI for occlusion/light/fog/audio/VT authoring.
5. Dialogue/quest assets and runtime state components.
6. Procedural placement using deterministic seeds.
7. Asset override resolver for mod load order.
8. Save delta compatibility tests for authored and modded entities.
