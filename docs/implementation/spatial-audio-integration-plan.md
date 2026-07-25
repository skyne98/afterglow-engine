# Engine audio integration plan (Steam Audio spatial acoustics)

**Status:** proposed; prototype validated, production integration not started  
**Date:** 2026-07-18  
**Scope:** Steam Audio 4.8.1 runtime acoustics, native CEF and public-web
targets, asset cooking, fixed-memory scheduling, and target-specific device sinks

Related documents:

- [`../api/steam-audio.md`](../api/steam-audio.md) — validated prototype API
  and benchmark results
- [`../research/steam-audio-browser.md`](../research/steam-audio-browser.md) —
  feasibility research and raw-result interpretation
- [`no-runtime-allocation-constant-time-budget-plan.md`](no-runtime-allocation-constant-time-budget-plan.md)
  — mandatory runtime discipline
- [`../api/ring-buffer.md`](../api/ring-buffer.md) — the only engine payload
  transport
- [`../api/frame-budget.md`](../api/frame-budget.md) — page-frame admission
  and telemetry
- [`box3d-physics-integration-plan.md`](box3d-physics-integration-plan.md) —
  deferred physics worker and shared structural-geometry cook
- [`editor-plan.md`](editor-plan.md) — deferred Blender/editor authoring,
  cooked-shape visualization, and collaboration decision gate

---

## 0. Resolved product decisions

These are user decisions, not benchmark hypotheses:

- `EngineAudio` is required and is the sole engine audio system/output pass.
- The service is one proper `#[rpc(worker = EngineAudioWorker)]` Rust worker on
  both native and web. Rust owns scheduling, fixed rings, generations,
  telemetry, streaming, Steam Audio FFI handles and final mixing policy. Thin
  target host glue only instantiates the worker/WASM, copies between its private
  memory and shared rings where the web architecture requires it, and wakes it;
  no parallel ad-hoc JavaScript or C++ worker service is permitted.
- It owns clips, streams, live voice chat, procedural producers, voice
  scheduling, Steam DSP and final mixing; all modes share one 128-voice pool.
- Hybrid reflections are the production target; parametric remains an explicit
  low-quality tier in the same system.
- CEF is the native target: it uses the generated native RPC client, a real OS
  worker, native Steam Audio with Embree, native memory, a bounded native PCM
  ring and a native device callback. It must never instantiate the audio service
  as WASM or a Web Worker. Only the
  public-web target uses WASM/obvhs and AudioWorklet.
- Acoustic tiles stream automatically around the listener during gameplay.
- The engine has no loading-screen phase; prefer offline cooking, then use fixed
  continuous streaming/admission after warm-up.
- Any fatal worker/Steam/output-ring/target-sink/device-contract fault disables
  all audio, emits silence and records a high-severity engine diagnostic.
- Frame order is corrected so game update precedes render preparation and audio
  publication observes the same current-frame transforms.
- `.big` has no pre-release backward-compatibility requirement; format bumps
  delete old readers and writers.
- Render-ahead uses the measured fixed eight-quantum depth proving zero
  underruns; latency remains an explicit acceptance metric.
- Runtime acoustic geometry is the shared prebuilt Box3D-style shape set:
  primitives, prebuilt convexes, static meshes/height fields and compounds.
  Moving compounds have fixed child slots; tick-boundary updates may move/resize
  shapes and enable/disable children. Runtime does not upload arbitrary new
  point/triangle topology; static-only shapes remain static.

## 0.1 Current implementation status (2026-07-19)

Phase 0 is in progress:

- `afterglow-audio-worker` is now the one generated Rust RPC service and owns
  fixed scheduling state, the Steam FFI wrapper, fatal state and PCM export;
- `afterglow-obvhs-tracer` is promoted into the workspace and linked into that
  same final Rust module (one Rust runtime/allocator);
- the public-web two-to-eight-quantum SAB, no-allocation AudioWorklet, dedicated
  thin host bridge, generated client and diagnostic page are implemented and
  unit-tested;
- the actual Steam module builds with all available build cores in fixed 256 MiB
  memory. The selected web profile initializes 16 voice/binaural slots and 4
  complete hybrid effects; native initializes 128/16. Active count is exposed
  separately from capacity;
- the normal-Worker render-ahead architecture is now rejected at the four-
  quantum cap under same-page hardware-WebGPU load; lowering it to 16 active
  wet voices did not remove normal-Worker scheduling stalls;
- a new Emscripten Wasm AudioWorklet gate runs Steam's per-quantum hybrid DSP on
  WebAudio's real-time-priority thread while the same page submits hardware-
  WebGPU work.

Gate 0 remains **red**. The native gate drives the generated OS-thread RPC
worker through Steam Audio/Embree and a bounded native ring into the physical
device. A historical four-quantum synthetic-scene 600-second device run had zero
underruns, but that depth is superseded by the real-assets result below. Final
`AppBuilder::on_ready` CEF composition and a foreground render-loaded rerun
remain open.

The old public-web Worker→final-PCM-ring path does **not** pass at four quanta.
At 16 active hybrid voices, same-page hardware-WebGPU produced 407 underruns and
a 15.97 ms Worker interval against 10.67 ms of ring coverage. The Emscripten
real-time Wasm AudioWorklet experiment subsequently proved Steam DSP can meet
the callback deadline, including lock-free concurrent simulation publication,
but it is **not the selected production architecture**: sharing Steam ownership
between a simulation pthread and an AudioWorklet creates a second web-only
lifecycle model.

The selected production architecture is the native-shaped pipeline on both
targets: typed RPC → one `EngineAudio` worker owning simulation, DSP and final
mixing → bounded final-PCM `afterglow-rpc::RingBuffer` → target sink. Native uses
an OS worker and CPAL callback; public web uses the generated WASM Web Worker and
a minimal allocation-free AudioWorklet PCM consumer. Only fixed capacities,
ring depth, memory backing and device sink differ.

The initial public-web profile is deliberately small: 16 total rendered voices,
of which at most **4 world-physical voices** each receive the complete supported
HRTF/direct/occlusion/transmission/reflection chain. Explicit 2D, listener-
relative and spatial-only voices use the remaining slots. Native retains 128
total and 16 complete world-physical voices. A web eight-quantum (21.33 ms)
final-PCM ring is the first acceptance candidate; reducing DSP alone cannot fix
normal-Worker scheduling stalls. Capacity pressure never downgrades a world
voice to partial acoustics.

The first 16/4/eight-quantum short run now passes: 38,900 callbacks under same-
page hardware-WebGPU with 104 Worker-owned simulation updates, zero underruns,
sequence errors, fatal errors or pump deadline misses, and 0.095/2.265 ms mean/
max pump time. A physical five-second capture had 474,852 non-zero samples, no
clipping and a 0.063 ms longest internal zero span. A subsequent scheduler run
used four complete world-physical voices plus a generic 2D `crossfadeTo`:
7,400 callbacks, one completed fade, zero rejects/stale handles/underruns/errors,
and a physical waveform with a 0.063 ms longest internal zero span. The real
sound-set web run then loaded five official SDK speech/noise/impulse WAVs (7.52
MiB resident PCM) and completed 11,244 callbacks with zero underruns or deadline
misses; physical capture's longest internal zero span was 0.083 ms. Evidence is
in `docs/benchmarks/steam-audio-unified-worker-web-profile-fox-workstation-2026-07-19.json`.

The native real-assets gate couples those five sounds to all three official
full-resolution Bistro scenes (1.05–2.83 million triangles), four complete
physical voices plus one dry voice, and one 512×2 reflection update per second.
Four quanta dropped 22–27 callbacks per 10-second scene because the sole worker
could not replenish the ring during full-scene simulation. Eight quanta produced
zero underruns, sequence errors, or silent frames across all 11,250 callbacks,
so eight is now the native default too. The checked upload path uses 512 KiB RPC
chunks for 23.4–71.4 MiB scene payloads. Full render geometry remains forbidden
for production web: its normal 256 MiB module did not complete Bistro Interior
within the bounded gate, reinforcing structural proxies plus tile streaming.
Evidence is in
`docs/benchmarks/steam-audio-real-assets-fox-workstation-2026-07-18.json`.

The public-web 60-second contention limit is satisfied. Gate 0 remains **red**
until final CEF composition passes and all real producer/allocation/failure
cases are covered. After that pass, production Wasm-AudioWorklet-DSP prototype code is
deleted while its evidence remains.

## 1. Decision

Promote the validated Steam Audio prototype as the engine's **required and only
audio system**, but do **not** promote the benchmark program or its full-render
geometry policy. Every runtime creates one `EngineAudio` resource. It owns audio
assets, decoding/streaming, the fixed voice scheduler, spatial and dry DSP, final
mixing, one output ring, and one target-specific device sink: a native callback
on CEF and AudioWorklet on public web. Music, UI, dialogue, procedural audio,
ambience and voice chat are producer modes feeding the same
128-voice pool; no engine audio path bypasses it and no parallel legacy mixer is
retained.

The public-web production path is:

```text
resident/streamed/live/procedural PCM producers
                         |
explicit pre-cooked acoustic tiles and shape templates
                         |
                         v
fixed-memory EngineAudio Web Worker
voice scheduling + Steam simulation + direct/HRTF/hybrid DSP
Rust obvhs CWBVH8 + two persistent Steam pthreads + final stereo mix
                         |
              bounded render-ahead PCM ring
                         v
minimal allocation-free AudioWorklet device sink
                         |
                         v
Web Audio stereo output
```

The CEF production path replaces the Web Worker/obvhs/pthreads with the generated
native RPC client, native OS worker, Steam Audio/Embree/native threads and a
native PCM ring and native device callback. No EngineAudio WASM module, Web
Worker or AudioWorklet participates in CEF audio.

Steam's hybrid reflection output contains an opaque `IPLReflectionEffectIR` that
cannot safely cross independent WASM instances. Simulation and every Steam DSP
effect therefore stay in one EngineAudio Worker/context. The Worker renders
final stereo PCM ahead of the device; the AudioWorklet only drains complete
quanta, applies final master gain, reports underruns, and outputs samples.

The browser path is exclusive to public web builds. The native target uses the
existing native `afterglow-rpc` backend: the generated client starts
`EngineAudioWorker` on a real OS thread from the shell's native bootstrap
boundary, links native Steam Audio/Embree, and feeds the selected target sink.
The shell is a generic host—the application composes the native service through
its bootstrap hook. The native target must not load the EngineAudio WASM module
or create an EngineAudio Web Worker.

> Note: `afterglow-cef` has been removed. The native bootstrap hook previously
> provided by `AppBuilder::on_ready` must be re-provided by `afterglow-shell`;
> see `docs/implementation/shell-promotion-plan.md`.

The selected initial target profiles are:

- one required `EngineAudio` system and one shared scheduler on every target;
- native: 128 total rendered voices, 16 complete world-physical voices and eight
  final-PCM quanta (21.33 ms);
- public web: 16 total rendered voices, 4 complete world-physical voices and
  eight final-PCM quanta (21.33 ms);
- resident clips, streamed audio, live voice chat and procedural PCM normalized
  into the same target pool;
- 512 global listener rays, two bounces and a 500 ms order-0 hybrid tail;
- one bounded reflection simulation cadence owned by the same Worker (initial
  web gate: 1 Hz), two Steam simulation threads and four-lane WASM SIMD.

These are hard capacities. Every admitted world-physical slot traverses the same
scheduler/mixer and output ring.

Full Bistro geometry is retained only as a diagnostic stress module. Its
package-worst 512×2 p99 of 27.88 ms and 182 MiB Exterior CWBVH prove that the
full render mesh is not a production web asset. The fixed-1.5-GiB Bistro module
must never become a normal engine artifact.

---

## 2. Non-negotiable constraints

1. **One payload mechanism per target, one framing.** Native CEF uses
   `afterglow-rpc::native` rings in native memory between native orchestration,
   its OS worker and native device sink. Public web uses compatible fixed SPSC SAB rings
   for page↔Worker and Worker↔AudioWorklet payloads. `postMessage` is only a
   public-web wake-up after bootstrap. CEF must not substitute the web backend.
2. **No callback-time allocation.** Neither target device callback allocates.
   The public-web AudioWorklet `process()` creates no objects, arrays, closures,
   strings, promises, typed arrays, views, maps, or sets. It performs no logging,
   exceptions, `postMessage`, `Atomics.wait`, fetch, module compilation, or
   effect creation. The native callback follows the equivalent native ban and
   never performs RPC, waits, locks, logging, allocation or Steam simulation.
3. **No sealed worker allocation.** Geometry arenas, Steam Audio objects,
   tracer storage, voice/stream tables, pthreads, rings, decoder buffers and DSP
   scratch are bounded during bootstrap/warm-up. Gameplay streaming fills and
   retires fixed slots incrementally; there is no `LoadingScreen` phase. Tracked
   allocators must report zero general allocation in sealed scheduling,
   simulation, DSP and geometry-update calls.
4. **Fixed capacities and deterministic overflow.** Every source, dynamic
   acoustic instance, command, pose frame, result frame, and telemetry record
   has a declared limit and a typed failure. No queue grows and no stale result
   is applied to a reused handle.
5. **Structural proxies only.** Shipping acoustic scenes are explicit authored
   assets. The cooker never silently interprets an arbitrary render mesh as
   acoustic geometry.
6. **Simulation never blocks the page frame.** The page publishes the newest
   bounded control state and continues. The audio Worker schedules simulation
   only while preserving its fixed PCM render-ahead target.
7. **Device deadlines outrank acoustic freshness.** The worker reduces
   low-priority hybrid wet work before the output ring can underrun. Neither the
   native sink nor the public-web AudioWorklet runs Steam DSP or waits.
8. **Runtime simulation with offline acceleration.** No baked acoustic probes
   are required, but static tiles, shape templates and traversal nodes are
   prebuilt by the pipeline wherever possible. Runtime supports bounded
   instances and structural updates of that finite shape set.
9. **One engine audio system.** Every sound is a voice in one fixed scheduler
   and one final mixer. Resident, streamed, live and procedural sources are PCM
   producers, not separate audio subsystems or passes.
10. **No authored production C++ wrapper.** Browser orchestration is TypeScript;
    shared service/kernel code is Rust. C/C++ in the production link is vendored
    Steam Audio and its pinned dependencies. The current C++ benchmark wrappers
    stay in the prototype until replaced, then remain diagnostic-only.
11. **No compatibility implementation.** Once the promoted path passes its
    gates, delete superseded prototype copies and old formats rather than
    maintaining parallel engine APIs.

---

## 3. What is and is not being promoted

### Promote

- the custom Steam scene callback contract;
- the local inclusive-edge two-sided ray/triangle behavior;
- medium-build obvhs CWBVH8;
- the SIMD128 four-child traversal kernel;
- indexed acoustic-mesh ingestion and validation;
- the strict two-pthread Steam simulation pool;
- the 128-voice/64-reflection/512×2 measured baseline;
- parametric DSP as the measured low-quality baseline while hybrid becomes the
  production target;
- the fixed SAB ring and payload-free wake model;
- all correctness, allocation, and benchmark fixtures.

### Replace before production

- standalone benchmark HTML and global functions;
- C++ service wrappers;
- hand-copied prototype deployment directories;
- material-name heuristics;
- a single atomic Y-only door;
- benchmark-specific command and response words;
- benchmark-only DSP loops, replaced by a bounded render-ahead Worker mixer;
- whole-file Bistro fetch and a fixed 1.5 GiB module;
- ad hoc build downloads and untracked generated artifacts.

### Explicit non-goals for the first shipping slice

- directional order-1 reflection output;
- GPU convolution or AMD TrueAudio Next;
- baked probe generation;
- arbitrary runtime point-cloud/triangle uploads or deforming acoustic meshes;
- changing compound child topology after construction;
- multiple listeners or multiple output `AudioContext`s;
- ad-hoc CEF browser IPC or a WASM audio-service fallback on CEF;
- automatic production proxies inferred from render-material names.

---

## 4. Ownership and execution domains

| Domain | Owns | Does not own |
|---|---|---|
| Page engine | generational clip/stream/voice handles, ECS binding, control rings, mandatory lifecycle and critical diagnostics | acoustic geometry, Steam handles, PCM mixing |
| PCM producers | resident clip reads, bounded asset/dialogue/music decode, procedural blocks and live voice-chat decode/capture boundaries | independent mixing or destination output |
| EngineAudio Worker | voice scheduling, acoustic tile residency, static/dynamic traversal, Steam scene/simulator/effects, HRTF/hybrid DSP, final stereo render-ahead mix | page rendering or device ownership |
| Native Steam threads (CEF) / Emscripten pthreads (web) | persistent reflection simulation jobs in the target worker memory domain | independent queues or engine messages |
| Native device sink (CEF) | drain the native final-PCM ring and report device telemetry | Steam DSP, scene traversal or browser IPC |
| AudioWorklet (public web only) | drain the SAB final-PCM ring, master gain, device output and underrun telemetry | clips, scheduling, scene traversal, Steam DSP, fetch or parsing |
| Asset cooker | proxy validation, materials, traversal-ready tiles/templates and packed binary emission | runtime policy or unreviewed simplification |

Each domain has its own fixed `EngineMemoryDomain` telemetry. The logical owner
is still the engine's single `EngineMemory` policy; the fact that WASM memories
and SABs are physically separate is explicit in stats.

Each target uses one fixed worker memory domain covering Steam simulation/DSP,
voice state, resident clips, stream buffers, acoustic tile arenas and final PCM
rings: tracked native arenas on CEF and fixed initial/maximum shared memory on
public web. Exact sizes are selected by the integrated render-ahead gates and
never grow. The CEF sink owns only fixed native views; the AudioWorklet owns
only small fixed JS/SAB views. Neither sink owns Steam. Assets or tiles exceeding
budgets fail admission without disabling already valid content.

---

## 5. Workspace and source layout

Create three workspace crates:

```text
crates/afterglow-audio/
  public handles, capacities, durable acoustic-asset format, wire records
crates/afterglow-steam-audio-sys/
  minimal pinned Steam Audio FFI and link discovery
crates/afterglow-audio-worker/
  voice/simulation/DSP service, obvhs tracer, tile/shape instances, native tests
```

Create authored web modules under:

```text
crates/afterglow-web/web/src/engine/audio/
  engine-audio.ts           sole public audio resource and lifecycle
  audio-memory.ts           fixed page pools and persistent views
  audio-wire.ts             fixed hot-ring codec
  audio-worker.ts           mandatory voice/simulation/DSP/mix Worker
  audio-worklet.ts          minimal final PCM-ring device sink
  reflection-scheduler.ts   deterministic Worker-side quality policy
  index.ts                  public exports
```

Add:

```text
scripts/build-audio.ts
```

The script builds the Rust Emscripten artifacts, assembles generated Emscripten
JavaScript with the authored TypeScript worker/worklet bundles, and emits only
generated deployment files. Authored TypeScript continues to import authored
`.ts` modules. Generated `.js` is never edited.

`afterglow-steam-audio-sys` isolates unsafe declarations and library/version
checks. `afterglow-audio-worker` owns all safe resource lifetime ordering. The
obvhs tracer moves into this worker crate rather than remaining a public C ABI;
Steam callbacks call the Rust tracer directly. Keep a small C ABI only where the
Emscripten-generated module needs exports.

The native feature uses Steam Audio's Embree scene and is mandatory for CEF. The
web feature uses the custom obvhs scene and is exclusive to public web builds.
Both implement the same service-level input/output records; a target contract
test must reject any CEF bundle or startup path containing the EngineAudio WASM
service or EngineAudio Web Worker.

---

## 6. Reproducible third-party build

Pin in source control:

- Steam Audio 4.8.1;
- obvhs 0.3.2;
- Emscripten 4.0.23, unless a separately recorded upgrade is approved;
- Valve's exact MySOFA, PFFFT, and zlib source revisions;
- Rust nightly commit used for `wasm32-unknown-emscripten`;
- all fixed-output source hashes and license files.

Move network acquisition out of `build.sh`. Add fixed-output Nix derivations for
sources and deterministic static libraries. A normal build must never clone a
branch or download an unverified archive. `scripts/build-audio.ts` receives
toolchain/library paths from the Nix shell and fails on version drift.

Produce one **EngineAudio module** with fixed shared memory, SIMD128, atomics,
bulk memory and a strict two-worker pthread pool. It contains the tracer, Steam
simulator, direct/binaural/hybrid/parametric effects, voice scheduler, PCM
buffers and final mixer. Exports are limited to bounded service, asset/stream
admission, control, telemetry and render-loop entry points.

The AudioWorklet is authored TypeScript with no Steam WASM instance. It receives
persistent views over the final stereo PCM SAB ring at bootstrap and performs no
module fetch/compile in `process()`. This removes opaque-IR transfer and
worklet-WASM initialization from the device callback boundary.

Audit Emscripten pthread traffic. The two Steam worker threads must be created
before seal and remain alive. After seal, jobs must use shared memory/futexes;
there must be no recurring Emscripten `postMessage` payload protocol. Add a
browser diagnostic that counts worker messages after initialization and fails
if anything other than declared wake-ups occurs.

Add all emitted EngineAudio glue/WASM, pthread helper, PCM producer artifacts
and worklet bundle to `web/contracts/web-artifacts.json` as generated artifacts
with roles.
Extend `scripts/build-web.ts --check` to hash and drift-check them. Do not copy a
prototype directory into `www/`.

Produce `THIRD_PARTY_NOTICES` entries and complete a static-link license audit
before release. Bistro assets remain benchmark-only and retain their CC-BY 4.0
attribution; they are not engine distribution assets.

---

## 7. Structural proxy and acoustic asset contract

### 7.1 One authored structural source, specialized cooked outputs

The canonical production input is **glTF/GLB**, not a new model format. For a
modular asset, explicit structural/collider nodes may live directly beside the
render nodes in the same `model.glb`; the cooker strips them from render output
and routes them by metadata. For a large level, export a companion
`level.structural.glb` so structural tiles can cook and stream independently.
Both forms use the same schema. Render triangles are never the implicit collision
or acoustic contract.

KISS does not mean forcing every consumer to use identical triangles or one
runtime BVH. Each primitive has an explicit usage mask such as `physics`,
`audio`, or both. A wall/floor shell will commonly serve both; an acoustic-only
ceiling or a physics-only trigger may serve one. Offline cookers extract
specialized physics and acoustic blobs from this one authored source. Their
workers then own separate optimized structures because they have different
materials, queries, update rates, memory, and address spaces. We accept small
cooked duplication rather than introduce cross-worker geometry ownership.

Shared dynamic nodes use the same stable authored ID in every cooked output, so
a door or shutter can resolve once to physics and acoustic handles and receive
consistent transforms without string lookup after seal.

Every primitive used by audio must declare acoustic material coefficients
through an `AFTERGLOW_acoustics` glTF extension or equivalent validated extras:

```json
{
  "absorption": [0.10, 0.20, 0.30],
  "scattering": 0.05,
  "transmission": [0.01, 0.01, 0.01]
}
```

Every node is explicitly one of:

- `static`: immutable world/tile geometry;
- `moving`: a prebuilt shape or compound instance with runtime pose/size state;
- `exclude`: author-visible geometry intentionally omitted.

It also declares its consumer usage and shared structural shape kind: box,
sphere, capsule, convex hull, static triangle mesh, height field, or compound.
Physics-specific material/filter metadata and audio-specific material metadata
remain separate properties on the same source shape. Moving nodes and compound
children have stable authored IDs.

A moving compound has fixed prebuilt child slots/topology. Runtime may
activate/deactivate a child and change child/whole pose, primitive dimensions or
prebuilt-hull scale at simulation tick boundaries. Movement is a normal value
update; resizing is a bounded structural update that refits bounds and, for
physics, recomputes affected mass/contact state. Runtime does not upload new
point clouds/triangle meshes or continuously deform topology in version one.
Static triangle meshes, height fields and Box3D baked compounds remain static;
movable complexity is represented by convex/primitive compounds.

Missing roles/usage, missing metadata for a selected consumer, NaN/infinite
values, invalid dimensions/scale, unsupported topology, skinning/morph targets,
duplicate IDs, mutable static-only shapes, or capacity excess are cook errors.
There is no material-name fallback in the production command.

The proxy should retain walls, floors, ceilings, large terrain, closed volumes,
doors, shutters, and other surfaces that materially change occlusion,
transmission, or decay. Decorative bevels, foliage cards, props, trim,
tessellation, and invisible render duplicates are excluded unless a measured
acoustic case requires them.

#### Blender and glTF transport

The industry workflow is dedicated simplified DCC geometry plus an importer
convention. Unreal commonly uses `UBX_`/`UCP_`/`USP_`/`UCX_` names in its FBX
pipeline; Godot accepts collision suffixes from glTF scenes. Core glTF 2.0 can
carry the nodes, meshes, transforms, and `extras`, but does not itself assign
collider semantics. `OMI_collider` is archived, while the separately hosted
`KHR_implicit_shapes` and `KHR_physics_rigid_bodies` work remains release-
candidate/draft rather than a ratified dependency.

Provide a small Afterglow Blender panel over normal custom properties. Blender's
official glTF exporter writes enabled custom properties to `extras`, so artists
can export collider nodes directly in a model GLB or as `*.structural.glb`
without a custom geometry exporter. The
panel exposes validated enums/fields for usage, static/dynamic/trigger role,
box/sphere/capsule/convex/trimesh shape, physics material/filter, acoustic
material coefficients, and stable authored ID. Metadata is canonical; object
names and collections are only human-readable organization. Keep field meanings
and meter units aligned with the proposed Khronos schemas where practical so a
future ratified extension can be mapped in the cooker.

Static level shells may use low-detail concave triangle meshes. Moving bodies
use primitives, fixed-topology compounds, or convex hulls; moving concave
triangle meshes are rejected. The cooker canonicalizes Blender proxy transforms
into dimensions plus poses/scales and validates every shape-specific scale rule.

### 7.2 Cooker command

Add:

```sh
afterglow-pipeline acoustic-scene \
  level.structural.glb level.big --asset world/acoustics
```

The command:

1. selects only primitives whose structural usage includes `audio`;
2. validates all selected nodes, transforms, indices, and finite material
   coefficients;
3. partitions static geometry into traversal-ready listener-streaming tiles;
4. preserves moving primitive/convex/compound templates, fixed child slots and
   shared authored IDs;
5. deduplicates exact materials;
6. computes tile, template and conservative transformed bounds;
7. builds explicit little-endian traversal-node arrays offline;
8. enforces configured counts and byte limits before writing;
9. writes a deterministic report with counts, bytes, materials and source hash;
10. adds the tile directory/templates/blobs as generic assets in `.big`.

The physics cooker consumes the same `*.structural.glb`, selects `physics`
primitives, and emits its own collision representation and material/filter
metadata. It does not parse the acoustic blob, and the audio worker does not
parse the physics blob.

For Box3D, glTF remains the durable authoring format while Box3D bytes are derived
cook output. Map authored shapes directly:

- `box` → `b3MakeBoxHull`;
- `sphere` and `capsule` → their native primitives;
- `convex` point geometry → bounded `b3CreateHull`;
- static concave geometry → `b3CreateMesh` with welding, shared-edge
  identification, and the measured SAH/median split policy;
- large static tiles/buildings → `b3CompoundDef` and `b3CreateCompound`.

The latter is Box3D's intended offline path: `b3ConvertCompoundToBytes` produces
an immutable static compound containing an internal AABB tree; runtime
`b3ConvertBytesToCompound` fixes pointers in the writable buffer without cloning,
and `b3CreateBakedCompoundShape` inserts the whole compound as one broad-phase
shape. Dynamic and kinematic compounds instead attach multiple normal shapes to
one body; baked compounds are static-only.

Treat Box3D compound bytes as target-specific, recookable derived artifacts—not
a replacement model format. The current serialization is a direct C memory
layout with internal pointer fixups and `B3_COMPOUND_VERSION`; it is not
sufficiently documented as portable between native 64-bit and wasm32 ABIs. Wrap
chunks with the pinned Box3D commit/version, target ABI, endianness, and layout
fingerprint, and either produce separate native/wasm chunks or prove byte
compatibility in tests. Box3D is still described by its author as alpha, and
current compound-layout regressions make commit pinning and malformed/alignment
validation mandatory.

Do not add an acoustic-specific storage mechanism to the container. Add generic
`AssetType::Blob`, `BigWriter::add_blob`, and caller-provided raw-byte loading.
Bump the `.big` format version, update the Rust and TypeScript parsers in the
same change, and delete old-version reading and writing. The engine is
unreleased; there is no compatibility path. Acoustic semantics live in the blob
consumer.

The first blob can use `Compression::None`. Do not misuse meshopt vertex
compression for arbitrary bytes. Add generic blob compression only after it is
measured and represented honestly in `Compression`.

### 7.3 Durable binary format

Use a little-endian fixed header and section directory, not Rust struct layout or
an unbounded serde object. The header includes:

- `AGAC` magic and format version;
- exact header and total byte lengths;
- source SHA-256;
- flags and meter scale;
- scene bounds;
- tile directory/count/bounds and per-tile vertex/triangle/node counts;
- material count;
- moving shape/template/compound child counts;
- section offset/size pairs;
- configured minimum runtime format version.

Sections contain explicit traversal-ready tile nodes, packed positions/indices,
material IDs and seven-float materials; analytic primitive records; prebuilt
convex data; fixed-topology compound child records; initial poses/scales; and a
bootstrap-only UTF-8 stable-ID table. All offset arithmetic uses checked
64-bit operations. The loader validates exact section non-overlap, alignment,
counts, every index/material ID, finite values, and configured limits before any
large allocation or Steam object creation.

At bootstrap, string dynamic IDs resolve to generational numeric handles. No
string lookup occurs after seal.

### 7.4 Initial production limits

Use provisional hard bounds until the proxy sweep selects tighter defaults:

| Item | Provisional maximum |
|---|---:|
| Static triangles | 250,000 |
| Static vertices | 300,000 |
| Acoustic materials | 64 |
| Moving shape/compound instances | 32 provisional |
| Compound children total | 256 provisional |
| One prebuilt convex/template payload | 8,192 source triangles before cook |
| Blob bytes | 32 MiB |

These limits are not derived quality targets. Phase 2 must benchmark 10K, 50K,
100K, 200K, and 250K proxies and then lower defaults if 256 MiB or simulation
budgets require it.

### 7.5 Proxy quality gate

For each representative scene, compare the full render mesh reference against
proxy candidates over a checked-in deterministic sample set of listener/source
poses and door states. Record:

- direct visibility classification mismatch;
- three-band transmission absolute error;
- three-band RT60 median and p95 absolute error;
- invalid/constant IR count;
- CWBVH bytes and build time;
- direct and 512×2 reflection p50/p95/p99;
- cooked and transferred bytes.

Start review with provisional limits of 2% weighted visibility mismatch, 0.05
transmission MAE, 0.10 s median RT60 error, and 0.25 s p95 RT60 error. These are
engineering screens, not established perceptual truth. A listening review and
representative game scenes must approve the final thresholds. Check raw results
into `docs/benchmarks/`.

Automatic simplification may be added later as an offline diagnostic that emits
this comparison report. It must not become an unreviewed shipping default.

---

## 8. Tracer promotion, tiles and runtime shape instances

Refactor the tracer around traversal-ready tiles and the shared structural shape
set:

```text
TraceTileArena
  fixed slots containing pipeline-built nodes/triangles/material IDs
TraceShapeTemplate
  analytic box/sphere/capsule or prebuilt convex data
TraceCompoundTemplate
  fixed child slots/topology and local broad phase
TraceScene
  fixed loaded-tile and moving-instance pools
  fixed top-level broad phase
  copied material table
```

Retain the validated indexed checks and inclusive-edge two-sided triangle test.
The pipeline emits explicit target-stable node arrays; gameplay tile admission
validates and copies one complete blob into a reserved slot, then atomically
publishes it. No gameplay CWBVH construction or arbitrary point/triangle upload
is required in version one.

Replace the Y-only door with fixed generational shape/compound instances. At a
simulation tick boundary, runtime may move/rotate an instance, resize analytic
primitive dimensions, scale a prebuilt convex, and activate/deactivate or
transform fixed compound children. Topology and mesh/convex points remain
immutable. Updates refit only affected compound/top-level AABB paths. Rays
transform into local space; hit positions, distances and inverse-transpose
normals transform back correctly. Invalid/singular shape state returns a typed
status and retains the last valid state.

The top-level broad phase covers resident static tiles and moving instances.
Closest-hit queries preserve the nearest result across both; any-hit
short-circuits. Define deterministic tile, object, child and triangle identities
from stable authored IDs plus generations.

Steam callbacks can execute on both persistent pthreads. Scene transforms are
immutable for the duration of `iplSimulatorRunDirect` or
`iplSimulatorRunReflections`; control frames received during a call remain in
the ring until the call returns. No callback reads page-owned memory.

Required tests include:

- flattened/indexed parity;
- every malformed index/material/count/header case;
- shared-edge and negative-zero regression;
- static/dynamic nearest-hit ordering;
- rotated/translated/scaled primitives, convexes and compounds;
- primitive/child resize and enable/disable at tick boundaries;
- inverse-transpose normal and hit-distance correctness;
- tile load/unload and broad-phase refit over repeated motion/resize;
- concurrent callback stress under two Steam threads;
- stale dynamic handle rejection;
- zero allocation for queries, transform updates, and sealed simulation loops;
- scalar/SIMD differential testing over randomized rays.

---

## 9. Rings and wire protocol

Use separate rings by ownership and traffic class. This prevents a high-rate pose
stream from blocking lifecycle or telemetry.

| Producer | Consumer | Ring | Traffic |
|---|---|---|---|
| Page | EngineAudio Worker | commands | asset/voice lifecycle, transport, master/quality control, tile-index open |
| Page | EngineAudio Worker | poses | latest listener/spatial-voice/shape/compound state |
| PCM producer | EngineAudio Worker | PCM | bounded decoded live/procedural/stream blocks when not read directly by the Worker |
| EngineAudio Worker | Page | events | completion, fatal failure, residency/voice stats and diagnostics |
| EngineAudio Worker | target device sink | stereo PCM | complete sequence-stamped final output quanta |
| Target device sink | EngineAudio Worker | consumption | read progress/underrun state using target ring indices and atomics |
| Target device sink | orchestration | telemetry | callback count, ring depth, underruns and sink faults |

Lifecycle calls may use the generated `#[rpc]` postcard contract during
bootstrap. Sealed hot frames and streaming use fixed binary records to avoid promises,
allocating decoders, and per-source RPC calls.

Every hot record starts with:

```text
u16 protocol_version
u16 record_kind
u32 byte_length
u32 sequence
u32 scene_generation
```

Numeric fields are explicitly little-endian. Records are fixed-size or have a
fixed-capacity count followed by packed entries. The reader rejects an unknown
version/kind, impossible length, count excess, non-finite value, stale scene
generation, stale source generation, and non-monotonic sequence. A malformed
record increments telemetry and cannot index outside a reserved array.

A source pose entry contains packed source handle, flags, position, forward,
user gain, directivity parameters, range, and priority class. The listener pose
contains position, forward, and up. Dynamic-instance entries contain a handle
and rigid transform. The page emits at most one latest-state control snapshot
per rendered frame.

A final PCM record contains sequence, sample frame, channel/quantum counts,
active quality tier and interleaved or planar stereo samples. It is complete
before publication. The sink consumes exactly one quantum per callback; an
empty/corrupt/stale frame writes silence, increments underruns, and transitions
the mandatory audio system to fatal failure according to the declared policy.

Initial capacities:

- four page→Worker pose/state snapshots;
- two to four final stereo quanta selected by the smallest-stable-latency gate;
- fixed per-producer stream rings sized by declared voice/codec policy;
- 64 records in each lifecycle command/event ring;
- 256 fixed telemetry records.

Consumers drain all complete snapshots and apply only the newest valid one.
Producers never overwrite unread bytes. If a latest-state ring is full, the new
snapshot is dropped, the prior valid state remains active, and a counter is
incremented. Lifecycle overflow is a hard typed failure. No queue catches up by
running an unbounded number of old simulations.

Generate or golden-test the Rust and TypeScript layout. At minimum, Rust writes
fixtures consumed by Bun tests and TypeScript writes fixtures consumed by Rust
tests. Assert every offset, size, enum value, wrap case, and stale-generation
case.

---

## 10. Public TypeScript API and ECS binding

The public resource should be small and capacity-explicit:

```ts
enum AudioProducerKind {
  ResidentClip,
  AssetStream,
  LiveVoice,
  ProceduralStream,
}

interface EngineAudioConfig {
  maxVoices: number;               // fixed target profile: native 128, web 16
  maxPhysicalVoices: number;       // complete chain: native 16, web 4
  maxResidentClipBytes: number;
  maxStreams: number;
  maxAcousticTiles: number;
  maxMovingShapes: number;
  maxCompoundChildren: number;
  renderAheadQuanta: number;       // native 8; accepted web candidate 8
  reflectionHz: number;            // fixed bounded target cadence
  rays: number;                    // 512 default
  bounces: number;                 // 2 default
  reflectionQuality: Hybrid | Parametric;
}

class EngineAudio {
  static create(options: EngineAudioCreateOptions): Promise<EngineAudio>;
  openAcousticWorld(containerUrl: string, assetName: string): Promise<AcousticWorldStats>;
  loadSound(assetName: string): Promise<AudioSoundHandle>;
  unloadSound(handle: AudioSoundHandle): EngineAudioStatus;
  bindListener(entity: number): EngineAudioStatus;

  // Fire-and-forget: the engine owns the voice through completion.
  playAt(sound: AudioSoundHandle, position: Vec3, options?: PlayOptions): EngineAudioStatus;
  playAttached(sound: AudioSoundHandle, entity: number, options?: PlayOptions): EngineAudioStatus;
  play2d(sound: AudioSoundHandle, options?: PlayOptions): EngineAudioStatus;
  playSpatialOnly(sound: AudioSoundHandle, position: Vec3, options?: PlayOptions): EngineAudioStatus;
  playListenerRelative(sound: AudioSoundHandle, offset: Vec3, options?: PlayOptions): EngineAudioStatus;

  // Controllable instances use identical placement semantics and return a
  // packed generational voice handle.
  spawnAt(sound: AudioSoundHandle, position: Vec3, options?: PlayOptions): AudioVoiceHandle;
  spawnAttached(sound: AudioSoundHandle, entity: number, options?: PlayOptions): AudioVoiceHandle;
  spawn2d(sound: AudioSoundHandle, options?: PlayOptions): AudioVoiceHandle;

  // Generic voice automation. `crossfadeTo` atomically creates the incoming
  // voice with the outgoing voice's placement and returns its handle.
  crossfade(from: AudioVoiceHandle, to: AudioVoiceHandle, seconds: number): EngineAudioStatus;
  crossfadeTo(from: AudioVoiceHandle, sound: AudioSoundHandle, seconds: number): AudioVoiceHandle;
  pause(handle: AudioVoiceHandle, fadeOutSeconds?: number): EngineAudioStatus;
  resume(handle: AudioVoiceHandle, fadeInSeconds?: number): EngineAudioStatus;
  stop(handle: AudioVoiceHandle, fadeOutSeconds?: number): EngineAudioStatus;
  setVolume(handle: AudioVoiceHandle, volume: number, seconds?: number): EngineAudioStatus;
  setPriority(handle: AudioVoiceHandle, priority: number): EngineAudioStatus;
  setMasterVolume(volume: number, seconds?: number): EngineAudioStatus;
  setShapeState(handle: AcousticShapeHandle, state: AcousticShapeState): EngineAudioStatus;
  resumeFromUserGesture(): Promise<EngineAudioStatus>;
  readStats(out: EngineAudioStats): void;
  dispose(): void;
}
```

Exact names may change during implementation, but the ownership rules may not:

- clip, voice, stream and shape handles are packed index+generation integers;
- module/context/output graph creation occurs during bootstrap/warm-up;
- clips, streams, voices, tiles and shape state use fixed admission pools during
  sealed gameplay;
- all public audio methods return typed statuses for stale handle, capacity,
  ring full or fatal audio state;
- stats fill caller-owned stable records/typed arrays;
- there is one listener, one 128-voice pool, one worker mixer, one final PCM ring,
  one target sink and one stereo output;
- no engine-managed source connects around the sink to the destination.

Resident clips, long asset streams, dialogue, ambience, live voice chat and
procedural generators differ only as bounded PCM producers. The API follows the
prominent Unreal/FMOD split: `play*` is fire-and-forget and `spawn*` returns a
controllable handle. A `Sound` asset owns routing, variation, default gain/pitch,
loop eligibility, priority and concurrency policy; ordinary call sites provide
only the sound plus placement.

Placement names are semantic guarantees, not quality hints. `playAt`/
`playAttached` always request the complete supported world-acoustic chain:
distance attenuation, air absorption, directivity where authored, HRTF,
occlusion, transmission and reflections. `play2d` is explicitly non-positional;
`playSpatialOnly` and `playListenerRelative` explicitly bypass environmental
simulation. Capacity policy may reject, virtualize or steal a whole physical
voice, but must never silently emit a partially simulated downgrade. The API
research and source comparison are in
`docs/research/game-audio-api-ergonomics.md`. EngineAudio owns timing, loop/seek
state, gain ramps, voice stealing, Steam effects and final mixing.

Volume changes are scheduled commands, never game-frame lerps. `setVolume`
starts a sample-clock ramp from the voice's currently evaluated volume;
interrupting a ramp starts the replacement from that exact value without a
step. `stop(handle, seconds)` ramps to silence and releases the slot only after
completion. `setMasterVolume` uses the same mechanism at the final mixer.

There is no special music player, deck, score channel, or music-only transition
path. Music, ambience, engines, dialogue and every other controllable sound are
ordinary voice handles. `crossfade(from, to, seconds)` applies an equal-power
transition to any two existing voices. The convenience
`crossfadeTo(from, sound, seconds)` atomically reserves and prebuffers a new
voice, inherits `from`'s placement/binding, starts it at silence on the same
sample, fades the old voice out, releases the old slot on completion, and
returns the new handle. Its sound asset may select a different bus/default gain;
only placement is inherited. If admission or prebuffering fails, it returns the
invalid handle and leaves `from` untouched.

Crossfades use equal-power curves by default; ordinary volume ramps use a smooth
monotonic curve. Repeated automation replaces prior automation from current
evaluated gains. All durations are seconds and zero means an internally
de-clicked immediate change. No timer, Promise, closure or per-frame command is
created by a ramp.

Microphone capture and network codec APIs are
browser/network boundaries, but decoded remote PCM enters `LiveVoice`; they do
not create a second output pass.

Add fixed ECS bindings rather than a frame scan:

- listener entity → one numeric binding;
- voice slot → optional entity ID and voice handle (`SpatialMono` requires an
  entity; dry modes do not);
- acoustic shape/compound handle → game/physics-controlled tick-boundary state.

Creating/destroying a binding updates dense fixed slot arrays in O(1). World
transforms are read through a small `WorldTransformReader` interface implemented
by `RenderAdapter`; audio does not import Three.js objects. Stale/despawned
entities deactivate their slots deterministically.

Coordinate contract: meters, right-handed, +Y up, listener forward derived from
the engine transform (Three's local -Z). Convert exactly once in the page
publisher and test canonical orientations.

---

## 11. Frame-loop integration

The current runtime prepares renderer state before `EngineFrameClient.update`,
which means game-authored transform changes become visible on the next prepare.
Do not bolt audio onto that ambiguous boundary. Split orchestration into an
ingest half and a publish half:

```text
1. EngineMemory.beginFrame / FrameBudget.beginFrame
2. WorkerPoll
3. VirtualTexture
4. StructuralCommands
5. PoseBatches
6. GameUpdate
7. RenderPrepare (hierarchy/world transforms/GPU upload)
8. AudioControl (publish the same prepared world poses)
9. RenderPass
```

Replace the monolithic `prepareAfterglowFrame` with named begin/finish functions
or an equivalent runtime-owned sequence, update all callers, and delete the old
entry point. This makes rendering and audio observe the same current-frame world
state.

Expand `FrameStage` to include `GameUpdate`, `AudioControl`, and `RenderPass`.
Update the default deadline arrays, API docs, book, tests, and every explicit
configuration in one change. `WorkerPoll`, `GameUpdate`, `RenderPrepare`,
`AudioControl`, and `RenderPass` are required stages whose overruns are recorded.
Acoustic reflection computation is **not** a page frame stage; only bounded ring
publication and polling are.

Add one required, first-class `EngineAudioSystem` slot rather than a generic
subsystem registry or multiple audio-pass list:

```ts
interface EngineAudioSystem extends RenderWorkerInput {
  warm(): Promise<void>;
  seal(): void;
  publish(frame: Readonly<RenderFrame>, transforms: WorldTransformReader): void;
  dispose(): void;
}
```

The runtime owns lifecycle ordering and registers its worker polling exactly
once. `EngineRuntimeOptions` contains exactly one `audio` system; it is not an
optional array and production has no null/bypass implementation. Headless unit
tests provide a fixed test double. Construction and registration happen in `Bootstrap`; module/effect/output-ring
creation and initial warm-up happen in `Warmup`; sealed publication is
synchronous and allocation-free.

Extend `EngineMemoryConfig` with explicit spatial-audio capacities for source
handles, control snapshots, event slots, and page-side byte storage. Create its
fixed arrays through `EngineMemory`, not privately and invisibly. The simulation
and worklet expose corresponding worker-domain configurations and the same
telemetry names.

`EngineMemory` has no `LoadingScreen` phase; retain that contract across the new
runtime/worker APIs and documentation. After initial bootstrap/warm-up, all
clips, streams and acoustic
tiles admit continuously through fixed pools, bounded bytes/operations and stale-
aware cancellation. `EngineAudio` automatically maintains a listener-centered
fixed tile neighborhood from the opened acoustic directory. A complete tile is
published atomically; failed admission retains current resident tiles and emits
visible telemetry. World replacement streams generations rather than stopping
the runtime.

---

## 12. Simulation scheduling

The EngineAudio Worker owns one continuous bounded render loop. It drains the
newest control/PCM producer state, schedules voices, runs direct simulation when
source/listener state changed, runs reflection simulation at admitted cadence,
executes Steam DSP, and publishes final stereo quanta until the configured
render-ahead target is full. Target-sink consumption advances ring state and
wakes the worker through native atomics/unparks on CEF or SAB atomics on public
web, never payload messages.

Use a monotonic integer sample clock as the canonical playback timeline. Page
frame cadence does not drive audio scheduling. The Worker never renders beyond
its fixed ring capacity or issues unbounded catch-up work. Reflection simulation
runs only when enough final PCM is buffered to preserve the output deadline.

Reflection admission is deterministic:

1. ignore inactive, out-of-range, or invalid sources;
2. bucket by 0–7 game priority;
3. bucket by a fixed quantized distance band within priority;
4. retain currently assigned reflection slots within a fixed hysteresis margin;
5. fill remaining slots in priority/distance/slot-index order;
6. reset and crossfade a reassigned reflection effect before wet output.

Implement buckets as fixed arrays and counters. Scanning at most 128 slots in the
Worker is bounded; no sort, `Map`, `Set`, splice, or growing list is permitted.
Source slot index is the final stable tie-breaker.

A significant listener/source/dynamic-instance change may request a 60 Hz burst,
but a token bucket bounds burst duration and duty cycle. Initial policy: at most
four 60 Hz reflection updates followed by steady 30 Hz admission. Measure and
tune this; do not let continuous motion silently become permanent 60 Hz.

Do not dynamically lower ray count every frame. The selected 512×2 point is an
RT60 stability knee, while lower rays and one bounce degraded the estimator.
Initial deterministic deadline-protection order is:

1. reuse the previous valid reflection result;
2. defer reflection simulation until render-ahead recovers;
3. reduce reflection cadence from 30 to 20, then 15 Hz;
4. reduce hybrid reflected voices from 64 to 48, then 32;
5. disable wet reflection for the lowest priority bands;
6. retain direct/HRTF/dry processing and report degraded quality.

Parametric reflection is an explicit configured low-quality tier within the same
mixer, not an automatic failure fallback.

A one-bounce emergency tier may be exposed only after its audible behavior is
explicitly accepted. Never increase to 1,024×2 automatically; full Bistro showed
that tier misses even 30 Hz.

---

## 13. Worker DSP and target device sinks

Before playback, the EngineAudio Worker creates every Steam object and scratch
buffer: up to 128 direct+binaural voice effects, up to 64 hybrid/parametric
reflection effects, voice/decoder state, parameter ramps, mono/stereo scratch,
final mix and the fixed output ring. Exact hybrid memory/CPU fit is a Gate 0
result; memory never grows.

For each rendered quantum, the Worker:

1. applies bounded lifecycle/control/producer updates at the sample boundary;
2. advances resident/streamed/live/procedural voices on one sample clock;
3. applies direct parameters, binaural direction and gain for spatial mono;
4. applies assigned hybrid early-convolution/late-parametric wet DSP, or the
   configured parametric low tier;
5. mixes dry mono/stereo voices through the same gain/priority path;
6. mixes every voice into one stereo quantum with click-free ramps;
7. publishes the complete sequence-stamped quantum to the PCM ring;
8. repeats only until the selected render-ahead depth is full.

Parameter and reflection-slot changes crossfade over fixed samples. Missing
simulation retains the previous valid parameters while deadline policy reduces
wet work. A stream underrun follows a declared per-voice silence/fade policy and
is counted; an output-ring underrun is a fatal system failure.

At each device callback, the target sink:

1. validates the next complete PCM frame/sequence;
2. copies stereo samples to native device output on CEF or browser output on web;
3. applies only the precomputed master ramp if needed;
4. advances/atomically notifies ring consumption;
5. writes fixed numeric sink telemetry;
6. returns without allocating, waiting, messaging, RPC, or running Steam DSP.

The CEF callback operates solely on persistent native ring/output views. The
public-web callback is the minimal AudioWorklet and never invokes WASM.

The sink supports the browser quantum length up to a declared preallocated
maximum. Gate 0 tests 44.1 and 48 kHz and at least 128-/256-frame buffers where
Chromium permits them. An unsupported quantum or empty/corrupt final ring emits
silence, records a critical fault and disables EngineAudio; there is no loading-
screen reinitialization path.

Web Audio's output arrays are an unavoidable browser boundary. Authored callback
code creates no additional views; all SAB/typed views are persistent from
construction.

`AudioContext.resume()` remains subject to the browser user-gesture rule.
Initialization may create a suspended graph during warm-up; the game calls
`resumeFromUserGesture()` from its start interaction. Context suspension,
sample-rate mismatch, device loss, and worklet module failure become typed
states. There is no hidden legacy spatializer, dry mixer, destination bypass or
partial fallback. Failure of the EngineAudio worker, Steam context, PCM output ring or target
sink disables all engine audio, emits silence thereafter, and
records a high-severity engine diagnostic.

---

## 14. Telemetry and failure behavior

Expose fixed numeric telemetry without formatting in hot paths.

### Page

- active/high-water/overflow voice, clip, stream, tile and shape slots;
- active reflection assignments;
- pose/control ring depth, high-water, and drops;
- stale handle/entity count;
- current quality/degradation state;
- worker/worklet lifecycle state.

### EngineAudio Worker

- asset bytes and validated counts;
- CWBVH nodes, owned bytes, and build milliseconds;
- active static/dynamic geometry and refit count;
- current/mean/max and fixed-histogram direct/reflection time;
- run/defer/reuse counts and effective cadence;
- source/reflection counts per run;
- malformed/stale/ring-full counters;
- allocator current/high-water/after-seal violations;
- reported pthread count and tracer lane count.

### AudioWorklet

- current/max and fixed-histogram callback time;
- callback deadline/overrun and output underrun count;
- PCM ring depth/high-water/sequence/malformed counters;
- unsupported quantum and fatal sink state.

Diagnostics copy these numbers into caller-owned structures outside the callback
and frame hot paths. String formatting, console output, JSON, and percentile
sorting occur only on an explicit diagnostic slow path.

Failure policy:

- individual asset/tile/voice capacity rejection is a typed admission failure
  and leaves valid active audio unchanged;
- any EngineAudio worker, Steam context, output PCM ring, target sink or device-
  contract failure atomically enters `AudioFailed`, disables every voice,
  outputs silence and records a high-severity engine diagnostic;
- no dry, HRTF-only, browser-node or legacy fallback continues after fatal
  audio failure;
- source-stream starvation is a per-voice error until it violates the final
  output deadline; output underrun is fatal;
- device/context suspension pauses the sample clock and resumes explicitly;
- world/voice generations reject all stale records.

---

## 15. Validation matrix and release gates

### Gate 0 — native CEF and public-web render-ahead feasibility (must happen first)

Build both target implementations before promoting public APIs: native
`afterglow-rpc` OS worker + native Steam Audio/Embree + native device sink for
CEF, and WASM Worker + obvhs + minimal AudioWorklet for public web. Prove on
each target:

- native initializes 128 total/16 complete world-physical voices; public web
  initializes 16 total/4 complete world-physical voices in fixed memory;
- resident, streamed, procedural and simulated live-voice PCM producers converge
  on one sample clock and final mix path;
- hybrid IR/DSP remains in the worker and non-zero stereo PCM reaches the actual
  target device sink through the RingBuffer;
- worker render/DSP loops and both device callbacks allocate nothing after seal;
- native consumption uses native atomics/unparks; public-web consumption uses
  SAB atomics without callback `postMessage`;
- target-specific 60-second 48 kHz/128-frame render+audio contention runs at the
  fixed eight-quantum depth on both targets has zero accepted-gate output underruns;
- measured end-to-end latency remains acceptable for the target profile;
- callback deadline misses, output underruns and fatal audio faults are zero;
- native heap/worker memory and browser heap/WASM memory, plus rings, voices,
  streams and effects, plateau on their respective targets.

If a fixed target profile fails, redesign its bounded capacity/depth policy
rather than adding adaptive runtime modes. The existing projected Worker DSP numbers are
not a substitute.

### Gate 1 — promoted tracer/service parity

- all existing seven Rust prototype tests port and pass;
- callback output matches the prototype over golden/randomized rays;
- indexed ingestion rejects all malformed inputs;
- SIMD disassembly still contains the required `f32x4` operations;
- every run reports two persistent simulation threads and four tracer lanes;
- tracked allocator proves zero sealed query/simulation allocation;
- generated module uses 256 MiB, not 1.5 GiB.

### Gate 2 — structural acoustic proxy

- at least one small deterministic fixture, Dungeon, and all three Bistro scene
  variants have explicit candidate proxies;
- error and scaling reports described in section 7.5 are checked in;
- selected proxy limits fit fixed memory with measured margin;
- full-render references and proxies both produce valid, varying IR;
- no production cook command accepts untagged render geometry.

### Gate 3 — rings, lifecycle, and frame integration

- wrap/full/empty/stale/generation and malformed wire tests pass in Rust and Bun;
- clip/voice/shape/tile pool operations are O(1), generational, fixed-capacity
  or bounded incremental;
- runtime tests prove exact new stage order and removal of `LoadingScreen`;
- rendering and audio publish the same current-frame world transform;
- listener-centered tile streaming, cancellation, atomic publication and world-
  generation replacement run during `GameplaySealed` without general allocation;
- post-seal message audit sees only allowed atomic/ring wake-ups;
- web artifact and authored-TS checks pass.

### Gate 4 — acoustic correctness

Use deterministic golden scenes:

- empty space: direct visible, no false hit;
- single wall: exact occlusion and three-band transmission;
- shared triangle edge: no acoustic crack;
- room materials: expected RT60 ordering and non-constant response;
- hybrid output: non-zero early convolution, late parametric decay and smooth
  configured transition/overlap;
- moving/resized primitive and fixed-topology compound: visibility/decay change
  with bounded refit and no topology rebuild;
- tile stream/unstream boundaries produce no false gap in the admitted
  neighborhood;
- stale voice, shape, tile and world generations are ignored;
- voice/reflection-slot reassignment crossfades without clicks;
- listener canonical orientations: left/right/front HRTF output is correct.

Capture impulse responses or rendered WAV fixtures where stable and compare
energy/envelope tolerances rather than bit identity across platforms.

### Gate 5 — render-loaded laptop acceptance

On the Ryzen 7 6800U, unlocked and using the validated CEF WebGPU stack, run
Dungeon rendering and spatial audio simultaneously. Record at least five fresh
launches plus 10-, 30-, and 60-minute soaks.

Initial acceptance targets:

- hardware WebGPU remains AMD/RDNA2 with no fallback/device loss;
- 60 Hz render p99 remains at the display interval and sustained scenarios have
  no unexplained presentation regression;
- selected production proxy 512×2 simulation p99 is at most 10 ms under render
  contention while final PCM remains ahead of the device;
- the selected eight-quantum depth has zero native-device or AudioWorklet
  deadline misses and zero output underruns;
- measured end-to-end audio latency remains acceptable and engine render-ahead
  contributes at most 21.33 ms at 48 kHz/128 frames;
- no invalid/constant IR, malformed record, queue overflow, fatal audio fault or
  after-seal allocator violation;
- native heap/worker memory and public-web heap/WASM memory, rings, timers,
  tasks, clips, streams, voices, tiles and effects plateau;
- continuous tile streaming and world-generation replacement stay within fixed
  arenas/budgets;
- every PCM producer kind, source/listener/compound motion+resize,
  suspension/resume and repeated start/stop is covered.

The 10 ms simulation target is intentionally below the 16.667 ms burst period;
it is not inferred from full Bistro. If a representative reviewed proxy misses
it, reduce geometry or reflection slots before raising the target.

### Gate 6 — distribution

- `cargo test --workspace` and target-specific tests pass;
- clippy is clean;
- Bun tests and hot-allocation lint pass;
- `bun scripts/build-web.ts` and `--check` agree;
- mdBook builds;
- Nix builds are reproducible from fixed hashes;
- third-party notices and Steam Audio redistribution terms are complete;
- generated artifact sizes and total distribution delta are recorded;
- API docs, book, runtime capacities, allocation boundaries, and AGENTS are in
  sync.

---

## 16. Implementation sequence

Each phase is one reviewable vertical step. Do not start later phases while an
exit criterion is red.

### Phase 0 — de-risk native CEF and public-web render-ahead output

1. Build one `#[rpc(worker = EngineAudioWorker)]` Rust service/context that
   schedules test voices, owns the Steam Audio FFI, runs direct/HRTF/hybrid DSP
   and mixes final stereo PCM. Both targets use generated typed clients and
   normal RPC framing; do not create a disposable JavaScript/C++ protocol.
2. Build the CEF path first-class: generated native client, OS worker, native
   Steam Audio/Embree, native final-PCM ring and native device callback, started
   only from `AppBuilder::on_ready`.
3. Build the public-web path: WASM/obvhs Worker and minimal TypeScript
   AudioWorklet over a fixed SAB stereo ring.
4. Implement shared sample-clock/render-ahead policy and target-specific atomic
   consumption wakes.
5. Exercise resident, streamed, procedural and simulated live-voice producers.
6. Add worker/sink telemetry, injected impulses and end-to-end latency markers.
7. Run 60-second contention tests independently at eight quanta on native and
   web; retain fixed bounded memory.

**Exit:** Gate 0 passes. No public API or asset-format change yet.

### Phase 1 — promote toolchain, FFI, and tracer

1. Add fixed-output Nix inputs and licenses.
2. Create the three audio crates and workspace entries.
3. Generate/minimize Steam Audio FFI.
4. Move indexed obvhs construction and tests into the worker crate.
5. Implement traversal-ready tile slots and fixed structural shape/compound
   instances with bounded pose/resize/refit.
6. Build the single EngineAudio simulation/DSP module reproducibly.
7. Port parametric fixtures, add hybrid fixtures and compare outputs/performance.

**Exit:** Gates 1 and the build portions of Gate 6 pass.

### Phase 2 — add generic blob assets and acoustic cooking

1. Add `AssetType::Blob`, `.big` version bump, writer/reader/runtime support.
2. Define and test the fixed `AGAC` format.
3. Implement `acoustic-scene` with explicit material/role validation.
4. Add malformed/fuzz/property tests for all checked arithmetic.
5. Author fixture, Dungeon, and Bistro candidate proxies.
6. Run the proxy correctness/scaling sweep and select real default limits.

**Exit:** Gate 2 passes and docs describe the final cooker/API.

### Phase 3 — implement the EngineAudio Worker

Current partial implementation: the worker owns a fixed generational scheduler
and a 64-slot resident bank. Warm-up RPC accepts strictly validated mono/stereo
48 kHz WAV PCM16/24/32/float32 into bounded Rust-owned PCM (32 MiB web, 256 MiB
native); the Steam mixer reads those stable buffers directly, loops on the
sample clock, and automatically releases completed one-shots. Cooked Sound
metadata/BIG reads and the remaining producer kinds are still open.

1. Add bootstrap/control/pose/producer/event/final-PCM rings.
2. Open the acoustic tile directory and continuously Range-fetch into fixed tile
   slots using listener-centered admission/cancellation.
3. Construct one Steam context, simulator, 128-voice scheduler, 64 hybrid slots,
   final mixer and fixed target threads: native threads on CEF, pthreads on web.
4. Implement resident/asset-stream/live/procedural producers on one sample
   clock, direct/reflection scheduling, motion bursts and render-ahead output.
5. Add world/tile/shape/clip/stream/voice generations and fixed telemetry.
6. Seal the allocator and run long scheduling/streaming/service-loop tests.
7. Audit post-seal native unpark behavior and public-web Emscripten messaging/
   atomic wake behavior.

**Exit:** one Worker produces bounded, valid final stereo PCM while tiles and all
producer kinds stream.

### Phase 4 — implement both production device sinks

1. Replace both Phase 0 sinks with the same final fixed PCM/telemetry framing.
2. Implement the native CEF sink over native ring storage and the public-web
   AudioWorklet over SAB storage; allocate all persistent views at construction.
3. Implement sequence validation, copy, master ramp, atomic consumption notify,
   silence/fatal behavior and telemetry on both targets.
4. Add variable-quantum/device/context failure tests and callback allocation
   checks for both sinks.
5. Repeat Gate 0 at production voice/hybrid/tile capacities on both targets.

**Exit:** both target sink/worker pairs pass underrun/deadline/latency gates.

### Phase 5 — integrate EngineMemory, ECS, and runtime phases

1. Add audio capacities/pools to `EngineMemory`.
2. Add `WorldTransformReader` and adapter implementation.
3. Split frame ingest/publish order and expand `FrameStage`.
4. Add the single `EngineAudioSystem` lifecycle slot.
5. Implement mandatory `EngineAudio`, clip/stream/voice/shape handles and entity
   bindings.
6. Preserve the no-`LoadingScreen` phase contract and add continuous tile/asset
   generation replacement.
7. Update every runtime/frame-budget/phase test and consumer; delete old
   orchestration.

**Exit:** Gate 3 passes and a minimal engine demo uses only public APIs.

### Phase 6 — integrated correctness and performance

1. Add deterministic wall/room/door demos and automated checks.
2. Integrate the selected Dungeon proxy.
3. Run real output through CEF while Dungeon renders.
4. Exercise all source capacities and degradation tiers.
5. Run five-launch benchmarks and bounded 60-second contention tests.
6. Check in raw JSON/WAV evidence and update recommendations from measured data.

**Exit:** Gates 4 and 5 pass.

### Phase 7 — productionize and remove prototype duplication

1. Add canonical generated artifacts and CI/Nix build commands.
2. Complete notices, book, API docs, capacity tables, and troubleshooting.
3. Move useful stress benchmarks under a diagnostic package.
4. Delete duplicated production-candidate code from
   `prototype/steam-audio-wasm`; retain only clearly labeled research harnesses
   and full-Bistro stress tooling.
5. Update the research KB with final shipping measurements.

**Exit:** Gate 6 passes; the engine path is canonical.

### Phase 8 — additional native non-browser hosts, only when demanded

CEF native support is already mandatory in Phases 0–7. This optional phase only
adapts the same native service and sink interfaces to additional non-CEF hosts;
it must not introduce another service protocol or weaken CEF's native target
boundary.

**Exit:** a separate measured host use case exists.

---

## 17. Documentation changes required with implementation

Update in the same commits that change behavior:

- `docs/api/steam-audio.md` — promoted API, ownership, capacities, formats;
- `docs/api/asset-system.md` and `docs/api/assets.md` — generic blob access;
- `docs/api/frame-budget.md` — new stages and frame order;
- `docs/api/engine-memory.md` — audio pools and worker domains;
- `docs/api/runtime-capacities.md` — every audio cap and overflow behavior;
- `docs/api/allocation-boundaries.md` — Web Audio arrays/module bootstrap;
- `book/src/reference/dynamic-audio.md` — user setup and source binding;
- asset, runtime, frame-budget, build, testing, and troubleshooting chapters;
- `AGENTS.md` — only after measured defaults and commands become canonical;
- `docs/benchmarks/` — raw proxy, worklet, contention, and soak evidence.

Until the phases complete, documentation must continue to label this as a
validated prototype and proposed integration, not an available engine API.

---

## 18. Definition of done

Engine audio is integrated only when a game can:

1. cook bounded traversal-ready acoustic tiles and shared shape/compound
   templates into its normal `.big` package;
2. create the required sole `EngineAudio` system and one shared 128-voice pool;
3. play resident, streamed music/dialogue/ambience, live voice chat and
   procedural voices through one scheduler/mixer;
4. run direct/HRTF/hybrid (plus explicit parametric low-tier) Steam DSP in the
   render-ahead worker and feed the native CEF sink or public-web AudioWorklet;
5. automatically stream listener-centered acoustic tiles during gameplay;
6. move/resize prebuilt primitives/convexes and fixed-topology compounds at tick
   boundaries without arbitrary runtime mesh input or general allocation;
7. fail all audio to silence with a high-severity diagnostic on any fatal
   worker/Steam/output-ring/target-sink/device-contract fault;
8. run alongside hardware WebGPU rendering with zero callback misses/output
   underruns, imperceptible measured latency and plateaued memory/queues;
9. reproduce all artifacts/evidence from pinned inputs and pass source,
   protocol, asset, allocation, native, browser, soak, book and artifact gates;
10. do all of the above without loading the Bistro stress module, adding ad-hoc
    CEF browser IPC, running the web audio service on CEF, retaining
    `LoadingScreen`, or retaining a parallel audio path.
