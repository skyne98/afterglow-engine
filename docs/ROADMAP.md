# Roadmap

Ordered implementation path for `afterglow-engine`. This roadmap is dependency-based. The detailed design direction lives in [main-engine-design.md](research/main-engine-design.md).

## Current Do-Now: Twitch Combat Multiplayer Foundation

This sequence ports the high-value engine-agnostic ideas from the R.E.P.O.
BEST-PARTS audit into Afterglow for twitch spellcasting/shooting, PvE-first
co-op, and future MMO-style non-competitive PvP. Keep the implementation small,
test-first, Lightyear-native, and server-authoritative where gameplay trust
matters.

- [x] Add frame-batched controller/body impulse accumulation: many gameplay
      systems can add forces in any order, one fixed-step drain applies and
      clears the clamped total for spells, bullets, explosions, traps, knockback,
      launchers, and boss hits.
- [x] Add a gameplay effect override stack with smooth timer-based blend-in/out
      for speed, gravity, look sensitivity, jump, stun/root/slow/haste, and
      combat-specific modifiers.
- [x] Keep the FPS demo as a local controller playground; multiplayer work now
      belongs in reusable Lightyear systems and the mock RPG harness instead of
      demo-specific networking code.
- [x] Formalize per-entity network sync strategy: physics-driven avatars,
      visual-rate camera/presentation smoothing, and buffered interpolation for
      arbitrary physics objects.
- [x] Implement buffered interpolation for arbitrary replicated physics objects
      and remote avatars using the documented sync strategy taxonomy.
- [x] Add server/master-authoritative physics object interaction events for PvE
      co-op: impact, break, grab/link/release, and kinematic remote observers.
- [x] Add chunk/area interest management for MMO-style replication scale before
      large-player-count PvP tests.
- [x] Add spring-based physics grabbing after impulse buffers and authority rules
      are in place.

## Phase 1: Playable Cell Foundation

Deep dive: [bevy-integration-world-runtime.md](research/bevy-integration-world-runtime.md), [bevy-integration-gameplay-audio-ui.md](research/bevy-integration-gameplay-audio-ui.md), [first-person-controller-feel.md](research/first-person-controller-feel.md).

- [x] Core app/plugin structure for engine systems
- [x] Context-aware input bindings with action phases and per-player device routing
- [x] First-person controller for dense immersive-sim spaces
- [x] Physics integration for player movement and interactable objects
- [ ] Core interaction model: use, pickup, doors, containers, triggers
- [x] Basic scene/cell loading with stable entity identity
- [x] Generic chunk/cell lifecycle state machine
- [x] Local-server single-player simulation path
- [x] Chunk/cell persistent state deltas
- [x] Save/load for one loaded cell

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

## Phase 5: Lightyear Multiplayer Rewrite

Deep dive: [lightyear-migration-plan.md](research/lightyear-migration-plan.md), [lightyear-leafwing-input.md](research/lightyear-leafwing-input.md), [mock-rpg-standin-plan.md](research/mock-rpg-standin-plan.md), [engine-rpg-harness.md](research/engine-rpg-harness.md), [lightyear-rewrite-simplification-plan.md](research/lightyear-rewrite-simplification-plan.md), [network-backend-abstraction.md](research/network-backend-abstraction.md). Historical rewind research remains in [server-rewind-component-history-plan.md](research/server-rewind-component-history-plan.md).

The previous custom networking stack is now legacy. Delete it instead of
maintaining a parallel transport/session/replication/prediction path.

- [x] Add semver-pinned `lightyear`, `lightyear_inputs_leafwing`, and `leafwing-input-manager` dependencies compatible with the current Bevy version
- [x] Replace generic string-based `PlayerCommand` input with a Leafwing `AfterglowAction` enum and entity-scoped `ActionState`
- [x] Add `AfterglowLightyearPlugin` boundary for client/server Lightyear setup and tick duration; concrete link entities and protocol registration remain follow-up work
- [x] Use Lightyear built-in transports first; custom Iroh/Steam transports were deleted from phase one
- [x] Remove FPS demo-specific multiplayer code after the experiment; keep the
      demo focused on local first-person controller regression coverage
- [ ] Register core replicated components/messages through Lightyear instead of `#[derive(Replicate)]` and custom snapshot/delta code
- [ ] Use Lightyear prediction for owned player entities and Lightyear interpolation for remote entities
- [x] Prove Lightyear `PreSpawned` reconciliation for transient predicted interaction entities in `engine-rpg-harness`
- [x] Prove Lightyear Avian lag-compensated historical collider queries in a focused prototype; keep it optional research, not the main engine path
- [x] Replace server rewind as the baseline with fixed server input delay, deterministic simulation, client prediction, and Lightyear reconciliation
- [x] Remove `ServerRewindPlugin` and the unused typed history-capture API from the engine surface
- [x] Port the late shield/death/corpse/loot/pickup/inventory correction oracle to the current `AfterglowNetworkPlugin` without engine rewind-history dependencies
- [x] Drive mock RPG late shield/death/pickup/inventory correction through real Lightyear client/server Crossbeam link entities and message registration
- [x] Prove mock RPG Lightyear Crossbeam replication and prediction/confirmation state across the late shield/death/pickup/inventory correction
- [x] Keep the FPS controller demo local-only; remove its local Lightyear runner, native `--connect` client launch, and native `--host` server launch
- [ ] Add reusable Lightyear local-server runner for demos/tests: headless server app, Crossbeam or localhost transport setup, clean shutdown, and deterministic fixed-tick stepping
- [ ] Add reusable replicated player identity/state, owned prediction, and correction outside the FPS demo
- [x] Add a testable in-engine development console core backed by `clap` subcommands, command history/scrollback resources, typed network requests, cvars, and unit-test execution helpers
- [x] Implement console tab autocomplete core: command/subcommand completion, cvar names and typed values, network endpoints, option names, descriptions, deterministic ordering, and completion tests for partial tokens/trailing spaces/unknown commands
- [x] Add Source-style console overlay UI on top of the existing console core: backtick toggle, text entry, command history navigation, scrollback, tab completion selection, and autocomplete descriptions
- [x] Remove the FPS demo console request consumer; console networking remains covered by the mock RPG harness
- [ ] Finish shared console networking beyond the Crossbeam harness: server start/stop/status semantics, live network stats, and latency simulation applied to real links
- [x] Drive the full mock RPG scenario suite through native Lightyear UDP/netcode client/server sockets
- [ ] Rewrite security, projectile, smoothing, stress, and interaction scenarios against Lightyear clients/server
- [x] Delete old custom network modules, old input module, old `afterglow-engine-macros`, old networking benches, and stale docs
- [ ] Add new Lightyear integration and fixed-delay harness benchmarks for 1k, 10k, and 100k entity pressure
- [ ] Re-evaluate Steam lobby/auth and Iroh only as Lightyear-compatible platform/admission layers after core multiplayer works

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
- [ ] `v0.4.0`: Lightyear multiplayer with Leafwing input, client prediction, interpolation, fixed server input delay, and deterministic authoritative simulation
- [ ] `v1.0.0`: feature-complete foundation for a small horror immersive sim
