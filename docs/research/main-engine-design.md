# Main Engine Design

## Vision

`afterglow-engine` is an engine for retro immersive sim RPGs with modern systems underneath.

The target mood comes from:

- `Arx Fatalis`: dense first-person dungeon simulation, tactile magic, authored darkness
- `Thief 2`: readable stealth spaces, shadow-driven navigation, audio as gameplay
- `System Shock 2`: systemic first-person RPG play, diegetic interfaces, hostile interiors
- `Daggerfall`: scale, travel, procedural reach, faction-heavy RPG structure
- `Morrowind`: alien open-world exploration, hand-authored places, strong material identity

The first production target is horror. The end state is broader: open-world retro immersive sim RPGs that can support stealth, survival horror, dungeon crawling, systemic RPG play, and networked shared worlds.

## Non-Negotiable Goals

### 1. Retro Immersive Sim First

The engine should prioritize:

- first-person presence
- dense interactive interiors
- readable stealth lighting
- physical object interaction
- inventory and equipment-driven problem solving
- audio propagation as gameplay
- doors, locks, traps, containers, fluids, switches, magic, and environmental state
- old-school RPG data: factions, stats, skills, conditions, effects, dialogue, quests

The rendering and world systems should serve systemic play before cinematic presentation.

### 2. Open World By Design

World streaming is a core system, not a late optimization.

The world is divided into chunks that can load and unload independently:

- terrain chunks
- interior cells
- dungeon segments
- static prop batches
- navigation data
- light/probe data
- virtual texture page ranges
- audio geometry
- Steam Audio ray-tracing data
- gameplay state
- network interest regions

Chunks must support authored placement and generated placement. The engine should work for a single dense dungeon and for a Daggerfall-scale world made from many streamed regions.

### 3. Fully Modern GPU-Driven Rendering

The renderer should be modern internally even when the art direction is retro.

Core direction:

- GPU-driven visibility
- AABB/Hi-Z occlusion culling
- indirect draws where practical
- bindless-style material/texture access where supported
- compute-built visibility lists
- clustered or tiled lighting for many dynamic lights
- GPU-friendly world chunk upload and eviction
- generic software virtual texturing for streamed surface detail
- explicit render debug counters

The first culling target follows the Spartan-style AABB/Hi-Z plan in [gpu-driven-culling-bevy-integration.md](gpu-driven-culling-bevy-integration.md).

### 4. Fully Multithreaded Runtime

The engine should assume heavy background work:

- chunk IO
- asset decode
- mesh/material preparation
- navigation updates
- Steam Audio propagation data
- light/probe baking or incremental updates
- visibility preparation
- network replication packing
- save/load serialization

Main-thread-only systems should be treated as temporary. Game simulation must remain deterministic enough to support fixed-input-delay multiplayer and future rollback experiments if they are ever reopened.

### 5. Multiplayer First And Ready

The engine should be shaped like `Source` / `Source 2`: usable for single-player, but architected around networked state from the beginning.

Required direction:

- stable entity identity separate from local ECS IDs
- server-authoritative simulation by default
- client prediction for player movement and core interactions
- interpolation for remote entities
- interest management tied to world chunks
- snapshot/delta replication
- fixed-delay input history for networked gameplay; rollback-friendly history only where future research proves it useful
- deterministic gameplay commands for doors, inventory, weapons, spells, and use actions
- replicated components as a visually separate truth layer; normal Bevy components derive runtime, animation, UI, audio, and editor state from that truth
- replicated truth is mutated by ordered Bevy systems reading validated command/messages; correction-sensitive presentation uses entity-backed cues
- save-game format that is compatible with network replication data

Single-player should run as a local server, not as a separate architecture.

## Lighting Direction

### Retro PBR

The material model is modern PBR, but the lighting language is old-school and low frequency.

Retro PBR means:

- modern base color, roughness, metallic, normal, emissive, and occlusion material inputs
- diffuse lighting mostly from lightmaps, probes, vertex colors, or Gouraud-style low-frequency terms
- limited high-frequency per-pixel lighting
- optional silhouette parallax occlusion mapping for close-up hero materials
- controlled specular/reflections for metal, wet stone, glass, polished wood, gems, and magic effects
- shadows and AO gate shiny details so specular does not break the Arx/Thief/Morrowind mood

The goal is not fake retro shading. The goal is physically plausible materials lit through a restrained, readable, old-school lighting model.

SPOM should be opt-in and controlled. It is for carved stone, grates, vents, wet cobbles, ornate doors, roots, bones, and metal panels, not a default replacement for geometry or normal maps. The implementation plan is in [silhouette-parallax-occlusion-mapping.md](silhouette-parallax-occlusion-mapping.md).

### Virtual Texturing

Virtual texturing is the planned generic texture streaming layer.

Direction:

- software page-table indirection, not hardware sparse textures as a baseline
- physical tile caches per compatible texture class
- low-resolution feedback pass for page requests
- chunk-aware page prefetching
- fallback mips always resident
- bounded page uploads per frame
- debug views for feedback, page table, cache, residency, and misses

This should let large worlds use many unique surfaces without forcing all texture data into VRAM. The implementation plan is in [software-virtual-texturing.md](software-virtual-texturing.md).

### Global Illumination

Primary GI target:

- DDGI for dynamic low-frequency bounce and chunk-friendly probe volumes

Secondary targets:

- baked or semi-baked lightmaps for stable authored interiors
- vertex colors/Gouraud-style diffuse for retro readability and cheap distant geometry
- optional SSGI later, behind a quality setting

DDGI should be cheap enough to use in horror interiors and scalable enough for open-world cells.

### Fog And Atmosphere

Volumetric fog should be cheap and gameplay-readable.

Direction:

- low-resolution froxel or slice-based fog
- local fog volumes for rooms, caves, sewers, crypts, and outdoor weather
- light-linked fog only where it matters
- strong art controls for density, color, height, noise, and visibility distance

Fog is part of stealth, horror, and world identity, not just a post-process effect.

## World Architecture

### Chunk Model

Each chunk should own or reference:

- renderable instances
- collision and physics data
- navmesh/nav graph fragments
- occluder proxies
- virtual texture residency hints
- DDGI/light probe data
- baked lightmaps or vertex lighting data
- audio propagation geometry
- Steam Audio scene data and probe batches
- gameplay entities
- persistent state deltas
- replication interest metadata

Loading a chunk should not imply all data is resident on the GPU immediately. Runtime systems should stage data by priority.

### Interior And Exterior Continuity

The engine should support:

- dense interior cells
- exterior terrain
- underground spaces
- portals, doors, stairs, lifts, and teleport transitions
- seamless or masked transitions depending on game design

Interiors are first-class. The open world exists to connect authored simulation spaces, not to replace them.

## Rendering Priorities

1. Deferred or hybrid deferred path for many lights.
2. Spartan-style AABB/Hi-Z occlusion culling.
3. Chunk-aware render extraction and GPU buffer residency.
4. Clustered/tiled lighting.
5. DDGI probe volumes.
6. Cheap volumetric fog.
7. Retro PBR material controls.
8. Software virtual texturing for open-world material scale.
9. Silhouette POM for controlled close-up material relief.
10. Optional SSGI.
11. Higher-end reflection features only after the core mood works.

Occlusion is proxy-authored. Meshes that want to be culled provide one or more occludee proxy boxes. Meshes that want to hide other objects provide one or more occluder proxy boxes. Skinned meshes may attach proxies to bones so torso, limb, head, equipment, or rigid animated parts rotate with the current pose.

The renderer should expose debug views for every major decision: chunk residency, occluders, occludees, HZB, light clusters, probes, fog, virtual texture pages, SPOM cost, and material channels.

## Simulation Priorities

1. Stable player controller.
2. Usable interaction model.
3. Inventory/equipment.
4. Doors, locks, containers, triggers, and scripted state.
5. Steam Audio ray-traced propagation hooks.
6. Save/load.
7. Multiplayer replication.
8. AI perception using light, sound, faction, and line of sight.
9. Magic/effects system.
10. Large-world chunk persistence.

Systems should be data-oriented enough to run across many chunks and entities without turning every object into bespoke script code.

## Design Constraints

- Prefer simple systems that compose.
- Every authored gameplay object should have clear networking and persistence semantics.
- Every render feature needs a debug view.
- Every chunk-owned resource needs a load/unload path.
- Horror readability beats physically perfect lighting.
- Retro mood beats maximal shader complexity.
- Open-world scale must not compromise dense interior simulation.
- Single-player and multiplayer should share the same simulation path.

## Near-Term Engine Shape

The first coherent vertical slice should be:

- one streamed dungeon or town cell
- first-person controller
- interactive doors, containers, and pickups
- several dynamic lights
- retro PBR materials
- one VT-backed wall/floor material prototype
- one SPOM hero material prototype
- authored occluder/occludee proxy boxes, including a bone-attached skinned proxy test
- AABB/Hi-Z visibility debug overlay
- cheap fog volume
- Steam Audio ray-traced occlusion/propagation test
- local-server single-player simulation path
- chunk save/load

That slice exercises the main architecture without requiring the full open-world RPG feature set.
