# Roadmap

Ordered implementation path for `afterglow-engine`. This roadmap is dependency-based. The detailed design direction lives in [main-engine-design.md](research/main-engine-design.md).

## Phase 1: Playable Cell Foundation

Deep dive: [bevy-integration-world-runtime.md](research/bevy-integration-world-runtime.md), [bevy-integration-gameplay-audio-ui.md](research/bevy-integration-gameplay-audio-ui.md).

- [ ] Core app/plugin structure for engine systems
- [ ] First-person controller for dense immersive-sim spaces
- [ ] Physics integration for player movement and interactable objects
- [ ] Core interaction model: use, pickup, doors, containers, triggers
- [ ] Basic scene/cell loading with stable entity identity
- [ ] Local-server single-player simulation path
- [ ] Chunk/cell persistent state deltas
- [ ] Save/load for one loaded cell

## Phase 2: Retro PBR Render Baseline

Deep dive: [bevy-integration-rendering.md](research/bevy-integration-rendering.md).

- [ ] Retro PBR material model and debug views
- [ ] Deferred or hybrid deferred path for many dynamic lights
- [ ] Clustered/tiled light assignment
- [ ] Authored occludee/occluder proxy components and debug draw
- [ ] Spartan-style AABB/Hi-Z occlusion culling visibility pass
- [ ] SPOM prototype for one close-up hero material
- [ ] Cheap volumetric fog volume prototype

## Phase 3: Streaming And Residency

Deep dive: [bevy-integration-world-runtime.md](research/bevy-integration-world-runtime.md), [bevy-integration-rendering.md](research/bevy-integration-rendering.md).

- [ ] Chunk graph for interiors, terrain, dungeon segments, and towns
- [ ] Chunk-aware render extraction and GPU buffer residency
- [ ] Chunk load/unload lifecycle for render, physics, gameplay, nav, and audio data
- [ ] Software virtual texturing prototype with feedback, page table, physical cache, and chunk prefetch
- [ ] Virtual texture residency debug UI
- [ ] World-state persistence across loaded and unloaded chunks

## Phase 4: Animation, Audio, And Stealth Systems

Deep dive: [bevy-integration-gameplay-audio-ui.md](research/bevy-integration-gameplay-audio-ui.md).

- [ ] Animation system with conservative skinned occludee proxies
- [ ] Bone-attached occludee/occluder proxy support for skinned meshes
- [ ] Steam Audio ray-traced occlusion/propagation hooks
- [ ] AI perception using light, sound, faction, and line of sight
- [ ] UI/HUD and diegetic interface framework
- [ ] Inventory, equipment, conditions, and gameplay effects

## Phase 5: Multiplayer-Ready Runtime

Deep dive: [bevy-integration-world-runtime.md](research/bevy-integration-world-runtime.md).

- [ ] Server-authoritative simulation path
- [ ] Snapshot/delta replication
- [ ] Interest management tied to chunks
- [ ] Client prediction for player movement and core interactions
- [ ] Interpolation for remote entities
- [ ] Replication-compatible save/load data
- [ ] Multiplayer test scene using the same systems as single-player

## Phase 6: Open-World RPG Layer

Deep dive: [bevy-integration-world-runtime.md](research/bevy-integration-world-runtime.md), [bevy-integration-rendering.md](research/bevy-integration-rendering.md), [bevy-integration-gameplay-audio-ui.md](research/bevy-integration-gameplay-audio-ui.md), [bevy-integration-rpg-editor-modding.md](research/bevy-integration-rpg-editor-modding.md).

- [ ] Open-world chunk streaming across terrain, interiors, dungeons, and towns
- [ ] RPG data model: factions, stats, skills, dialogue, quests
- [ ] Procedural and authored placement pipeline
- [ ] Editor/tooling for chunks, lighting, occluders, probes, VT pages, and gameplay state
- [ ] Virtual texture authoring/import tools
- [ ] Modding support through data and asset override layers
- [ ] DDGI probe volumes and baked/vertex low-frequency lighting support
- [ ] Optional SSGI and higher-end reflection features after the core mood works

## Release Shape

- [ ] `v0.1.0`: dense playable cell with movement, physics, interaction, local-server simulation, and save/load
- [ ] `v0.2.0`: retro PBR, many lights, fog, SPOM, and AABB/Hi-Z visibility debug path
- [ ] `v0.3.0`: chunk streaming, VT prototype, animation proxies, Steam Audio hooks, and UI
- [ ] `v0.4.0`: multiplayer-ready simulation with replication, prediction, interpolation, and chunk interest
- [ ] `v1.0.0`: feature-complete foundation for a small horror immersive sim
