# Roadmap

Ordered implementation path for `afterglow-engine`. This roadmap is dependency-based. The detailed design direction lives in [main-engine-design.md](research/main-engine-design.md).

## Current Do-Now: Lightyear Runtime Foundation

The current implementation track is the reusable Lightyear runtime layer. The
new `engine-rpg-harness` proves fixed input delay, Leafwing input, prediction,
PreSpawned interactions, and UDP/netcode transport in tests; the next work is to
turn those harness-proven patterns into reusable engine or game-facing APIs.
Architecture gaps here are missing reusable runtime/API pieces. Regression
coverage, benchmarks, and production-hardening checks are follow-ups, not counted
as architectural gaps.

- [x] Decide whether Lightyear protocol registration is engine-owned or
      game-owned. Engine-owned: `register_afterglow_lightyear_protocol`
      registers only core `HistoryTick` and `StableEntityId` protocol state.
      With the `lightyear` feature, `StableEntityId` is registered as a
      replicated Lightyear component so server-spawned replicated entities carry
      durable gameplay identity to clients. Lightyear still owns live session
      entity remapping. Entity input ownership uses Lightyear's existing
      `ControlledBy` / `Controlled` relationship plus Leafwing `InputMap`, not
      custom player/avatar marker components. Not auto-installed — callers opt
      in explicitly.
- [x] Add initial non-Steam session/matchmaking API slice:
      `AfterglowSessionPlugin` with `SessionRequest`/`SessionEvent` Bevy
      events, platform-neutral types (`SessionId`, `SessionMemberId`,
      `SessionConfig`, `SessionInfo`, etc.), and an in-memory
      `NonSteamSessionCatalog` provider. No Steam dependency, no Lightyear
      transport/link wiring, no custom player/avatar marker taxonomy. The API
      is designed so that a future Steam lobby backend can be added as an
      alternative provider behind the same event protocol.
- [x] Add initial session-to-Lightyear bridge slice:
      `AfterglowSessionLightyearBridgePlugin` reads `SessionEvent` messages
      and manages Lightyear link lifecycle: spawns in-process Crossbeam links
      for `SessionTransport::Local` and writes pending netcode startup
      parameters for `SessionTransport::DirectUdp`. Opt-in behind
      `feature = "lightyear"`; not auto-added by `AfterglowNetworkPlugin`.
      No Steam dependency, no controlled entity lifecycle, no custom transport
      abstraction.
- [x] Add player identity layer: `PlayerIdentity` (Native Ed25519 proof /
      Steam ticket passthrough) attached to session create/join requests,
      verified by the non-Steam provider, and bound to `SessionMemberId` for
      rejoin detection. Private keys stay on the client; the server only sees
      the public key.
- [x] Add networked NonSteam session provider: `ProviderEndpoint` in
      `Join`/`JoinByCode`/`Search`, `NonSteamSessionProvider` TCP listener,
      and `NonSteamSessionClient` remote request sender. Remote clients can
      now query and join a NonSteam listen-server by code + address.
- [x] Engine consumer that drains `PendingNetcodeStartup` and spawns real
      Lightyear netcode link entities.
- [x] High-level session API: `AfterglowSessionExt` with `app.session().host`,
      `host_with_endpoint`, `join_non_steam`, `join_steam`, `join_local`,
      `search_non_steam`, `leave`, `status`, and `is_in_session`.
- [ ] Add reusable controlled-entity lifecycle outside scenario-only harness
      code: assign, revoke, and rebind Lightyear `ControlledBy` / `Controlled`
      entities on join, disconnect, respawn, possession, and reconnect. This is
      lifecycle orchestration only; Lightyear plus Leafwing already own
      entity-scoped action routing through `InputMap` / `ActionState`.
- [ ] Promote or wrap the harness-proven `LightyearTestRig` into reusable
      dev/demo local-server infrastructure, or explicitly document it as a
      test-only crate. Include clean shutdown and deterministic fixed-tick
      stepping. Decide whether `mock-rpg-network-tests::LightyearNetworkedRpg`
      should be consolidated into this path or kept as a separate regression
      oracle.
- [ ] Wire shared console networking requests (`connect`, `server start/stop`,
      `net stats`, latency simulation) to real Lightyear links beyond the mock
      RPG Crossbeam oracle. Parsing and request emission exist; the missing
      piece is an engine-side Lightyear-link consumer.
- [ ] Verification follow-up, not an architectural gap: add remaining Lightyear
      regression coverage for authority invariants,
      server-derived lifecycle cases such as projectile expiry and stale-hit
      rejection, and smoothing/correction behavior. The architecture is already
      server-authoritative: clients send input only, and server gameplay systems
      decide authoritative spawn/despawn/interact outcomes. Stress,
      adversarial, interaction, PreSpawned, combat, RPG, native input, and UDP
      variants already exist in `engine-rpg-harness`.
- [ ] Add Lightyear/fixed-delay benchmarks for 1k, 10k, and 100k entity
      pressure.
- [ ] Decide the role of `delta` and `delta-lightyear`: integrate them into a
      current engine path, move them to prototypes/research, or retire them.
      `delta-lightyear` currently does not depend on Lightyear despite the name.

## Completed Combat/Controller Foundation

This sequence ported high-value engine-agnostic ideas from the R.E.P.O.
BEST-PARTS audit into Afterglow for twitch spellcasting/shooting, PvE-first
co-op, and future MMO-style non-competitive PvP.

- [x] Add frame-batched controller/body impulse accumulation: many gameplay
      systems can add forces in any order, one fixed-step drain applies and
      clears the clamped total for spells, bullets, explosions, traps, knockback,
      launchers, and boss hits.
- [x] Add a gameplay effect override stack with smooth timer-based blend-in/out
      for speed, gravity, look sensitivity, jump, stun/root/slow/haste, and
      combat-specific modifiers.
- [x] Keep the FPS demo as a local controller playground; multiplayer work now
      belongs in reusable Lightyear systems, `engine-rpg-harness`, and focused
      regression harnesses instead of demo-specific networking code.
- [x] Formalize per-entity network sync strategy: physics-driven avatars,
      visual-rate camera/presentation smoothing, and buffered interpolation for
      arbitrary physics objects.
- [x] Implement buffered interpolation for arbitrary replicated physics objects
      and remote avatars using the documented sync strategy taxonomy.
- [x] Add server/master-authoritative physics object interaction events for PvE
      co-op: impact, break, grab/link/release, and kinematic remote observers.
- [ ] Rebuild chunk/area interest management for MMO-style replication scale
      after the world/chunk API exists again. The old `InterestMap` was deleted;
      no replacement network API is currently exposed.
- [x] Add spring-based physics grabbing after impulse buffers and authority rules
      are in place.

## Phase 1: Playable Cell Foundation

Deep dive: [bevy-integration-world-runtime.md](research/bevy-integration-world-runtime.md), [bevy-integration-gameplay-audio-ui.md](research/bevy-integration-gameplay-audio-ui.md), [first-person-controller-feel.md](research/first-person-controller-feel.md).

- [x] Core app/plugin structure for engine systems
- [x] Context-aware input bindings with action phases and per-player device routing
- [x] First-person controller for dense immersive-sim spaces
- [x] Physics integration for player movement and interactable objects
- [ ] Core interaction model: use, pickup, doors, containers, triggers
- [ ] Basic scene/cell loading with stable entity identity. `StableEntityId`
      exists; cell manifests and scene loading are planned, not current API.
- [ ] Generic chunk/cell lifecycle state machine
- [x] Local-server single-player simulation path
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

## Phase 5: Lightyear Multiplayer Reuse And Integration

Deep dive: [lightyear-migration-plan.md](research/lightyear-migration-plan.md), [lightyear-leafwing-input.md](research/lightyear-leafwing-input.md), [mock-rpg-standin-plan.md](research/mock-rpg-standin-plan.md), [engine-rpg-harness.md](research/engine-rpg-harness.md), [lightyear-rewrite-simplification-plan.md](research/lightyear-rewrite-simplification-plan.md), [network-backend-abstraction.md](research/network-backend-abstraction.md). Historical rewind research remains in [server-rewind-component-history-plan.md](research/server-rewind-component-history-plan.md).

The previous custom networking stack is now legacy. Delete it instead of
maintaining a parallel transport/session/replication/prediction path. Remaining
active reusable-runtime work is tracked in **Current Do-Now** above; this section
records completed proof points and deferred platform/admission work.

- [x] Add semver-pinned `lightyear`, `lightyear_inputs_leafwing`, and `leafwing-input-manager` dependencies compatible with the current Bevy version
- [x] Replace generic string-based `PlayerCommand` input with a Leafwing `AfterglowAction` enum and entity-scoped `ActionState`
- [x] Add `AfterglowLightyearPlugin` boundary for client/server Lightyear setup and tick duration; concrete link entities and protocol registration remain follow-up work
- [x] Use Lightyear built-in transports first; custom Iroh/Steam transports were deleted from phase one
- [x] Remove FPS demo-specific multiplayer code after the experiment; keep the
      demo focused on local first-person controller regression coverage
- [x] Delete old `#[derive(Replicate)]` and custom snapshot/delta network replication paths
- [x] Prove Lightyear `PreSpawned` reconciliation for transient predicted interaction entities in `engine-rpg-harness`
- [x] Prove Lightyear Avian lag-compensated historical collider queries in a focused prototype; keep it optional research, not the main engine path
- [x] Replace server rewind as the baseline with fixed server input delay, deterministic simulation, client prediction, and Lightyear reconciliation
- [x] Remove `ServerRewindPlugin` and the unused typed history-capture API from the engine surface
- [x] Port the late shield/death/corpse/loot/pickup/inventory correction oracle to the current `AfterglowNetworkPlugin` without engine rewind-history dependencies
- [x] Drive mock RPG late shield/death/pickup/inventory correction through real Lightyear client/server Crossbeam link entities and message registration
- [x] Prove mock RPG Lightyear Crossbeam replication and prediction/confirmation state across the late shield/death/pickup/inventory correction
- [x] Keep the FPS controller demo local-only; remove its local Lightyear runner, native `--connect` client launch, and native `--host` server launch
- [x] Add a testable in-engine development console core backed by `clap` subcommands, command history/scrollback resources, typed network requests, cvars, and unit-test execution helpers
- [x] Implement console tab autocomplete core: command/subcommand completion, cvar names and typed values, network endpoints, option names, descriptions, deterministic ordering, and completion tests for partial tokens/trailing spaces/unknown commands
- [x] Add Source-style console overlay UI on top of the existing console core: backtick toggle, text entry, command history navigation, scrollback, tab completion selection, and autocomplete descriptions
- [x] Remove the FPS demo console request consumer; console networking remains covered by the mock RPG regression oracle
- [x] Drive functionally equivalent RPG scenario coverage through native Lightyear UDP/netcode client/server sockets in `engine-rpg-harness`
- [x] Delete old custom network modules, old input module, old `afterglow-engine-macros`, old networking benches, and stale docs
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

- [ ] `v0.1.0`: local controller, physics interaction foundation, dev console, fixed-delay Lightyear harness, and the remaining reusable multiplayer runtime APIs
- [ ] `v0.2.0`: rebuilt dense playable cell with use/pickup/doors/containers/triggers, scene/cell loading, chunk lifecycle, persistence deltas, and save/load
- [ ] `v0.3.0`: retro PBR, many lights, fog, SPOM, AABB/Hi-Z visibility, and rendering diagnostics
- [ ] `v0.4.0`: chunk streaming, VT prototype, animation proxies, Steam Audio hooks, UI/HUD, and stealth/RPG interaction layers
- [ ] `v1.0.0`: feature-complete foundation for a small horror immersive sim
