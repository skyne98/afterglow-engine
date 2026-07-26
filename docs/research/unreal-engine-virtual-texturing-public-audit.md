# Unreal Engine virtual texturing — public-interface audit

Research date: 2026-07-25

## Evidence boundary

Unreal Engine's complete source is distributed under the Unreal Engine EULA in
Epic's access-controlled repository. The current GitHub identity cannot access
`EpicGames/UnrealEngine`, and no authorized local checkout is installed. This
audit deliberately did **not** use third-party public mirrors of that gated
source.

Accordingly, this is not represented as a complete source audit. It uses
Epic's official public UE 5.8 user documentation and generated C++ API pages,
plus UE 5.6 documentation where the corresponding current page was inaccessible
through the documentation frontend. Public API pages expose the current header
contracts and source paths, but not private implementation bodies.

Primary sources:

- [Streaming Virtual Texturing](https://dev.epicgames.com/documentation/en-us/unreal-engine/streaming-virtual-texturing-in-unreal-engine)
- [Virtual Texture Memory Pools](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-texture-memory-pools-in-unreal-engine?application_version=5.6)
- [Virtual Texturing Settings and Properties](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-texturing-settings-and-properties-in-unreal-engine?application_version=5.6)
- [`IVirtualTexture`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/RenderCore/IVirtualTexture)
- [`IVirtualTextureFinalizer`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/RenderCore/IVirtualTextureFinalizer)
- [`FVTProducerDescription`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/RenderCore/FVTProducerDescription)
- [`FAllocatedVTDescription`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/RenderCore/FAllocatedVTDescription)
- [`FVTRequestPageResult`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/RenderCore/FVTRequestPageResult)
- [`EVTRequestPageStatus`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/RenderCore/EVTRequestPageStatus)
- [`FVTProduceTargetLayer`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/RenderCore/FVTProduceTargetLayer)
- [`FVirtualTextureBuildSettings`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Engine/FVirtualTextureBuildSettings)
- [`UVirtualTexture2D`](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Engine/UVirtualTexture2D)

## Conclusion

Unreal's public contracts confirm that its **Streaming Virtual Texturing (SVT)**
is the closest production product reference to Afterglow:

```text
cooked texture/UDIM tiles
  -> pixel-visible GPU demand
  -> page request with explicit status and priority
  -> disk-capable producer
  -> layer-grouped page production
  -> safe physical-pool finalization
  -> page-table update
  -> material stack sampling
```

The strongest transferable design is not Unreal's broad engine framework. It is
the narrow producer protocol:

- request and production are separate operations;
- a request returns `Invalid`, `Saturated`, `Pending`, or `Available`;
- an opaque request handle connects request to production;
- all requested layers at one virtual address are grouped into one producer
  call;
- physical writes are centralized in finalizers at a known hazard-free point;
- page-table updates occur in that same controlled phase.

The public evidence is insufficient to verify the exact cooked chunk format,
I/O batching, feedback compaction, transcode cache, cancellation semantics,
private queue capacities, or page-table bit packing. Those require an authorized
source checkout before Unreal can be treated as a line-by-line implementation
reference.

## 1. Two different Unreal VT products

### Streaming Virtual Texturing (SVT)

SVT cooks authored texture assets into fixed-size tiles for disk streaming.
GPU-visible pixels determine demand, the CPU loads required tiles, and fixed GPU
physical pools cache them. It supports normal texture assets, virtual texture
lightmaps, and UDIM imports.

This is the product comparable to Afterglow's file-backed VT.

### Runtime Virtual Texturing (RVT)

RVT renders scene/material output into virtual pages on demand. It supports
runtime-produced base color, normal, roughness, specular, masks, and height,
plus optional prebuilt streaming low mips.

This is closer to Dagor/Wicked's generated terrain caches. Unreal intentionally
uses one generic producer/finalizer abstraction for both streamed and rendered
pages; the public `IVirtualTexture` description explicitly names disk streaming,
runtime compositing, and arbitrary generation as producer implementations.

The rest of this note focuses on SVT and uses RVT only where it exposes generic
runtime architecture.

## 2. Asset and cook boundary

A normal `UTexture2D` can opt into VT using project build settings.
`UVirtualTexture2D` adds local `FVirtualTextureBuildSettings` and derives from
`UTexture2D`; its public API participates in asynchronous cooked-platform-data
creation and exposes whether all layers share one physical space.

`FVirtualTextureBuildSettings` makes tile size and tile-border size explicit.
Tile size is rounded to a power of two; border size is clamped/aligned. Public
project defaults are:

- 128 useful texels per SVT tile;
- four border texels by default;
- project-wide feedback resolution factor;
- optional anisotropic filtering;
- configurable lossy source compression.

A larger border supports greater anisotropy but consumes more disk and physical-
pool memory. Trilinear filtering is stochastic and relies on temporal
accumulation rather than two deterministic physical lookups.

### UDIM blocks

Unreal imports numbered UDIM images as one logical VT. Different source UDIMs
may have different resolutions without paying the memory cost of upscaling the
small image to the largest source resolution.

`FVTProducerDescription` reveals the runtime representation: producers contain
uniform-size **blocks**, blocks form a larger grid, and UDIM sheets map to
individual blocks. When multiple producers/layers are allocated together their
blocks align; layers with fewer blocks wrap over the larger domain.

This is a useful distinction for Afterglow: source package layout need not force
all material layers to have identical original dimensions, but runtime virtual
addresses still need a deterministic shared block/page mapping.

## 3. Producer description

The UE 5.8 `FVTProducerDescription` contract includes:

- tile size and border size;
- dimensions, tile depth, maximum virtual level;
- block width/height and block-grid width/height;
- number of texture layers;
- per-layer pixel format, sRGB flag, and fallback color;
- physical-group count and each layer's physical-group index;
- `bRequiresSinglePhysicalPool`;
- `bPersistentHighestMip`;
- continuous-update and sampling-history flags;
- producer priority and debug identity/hash;
- optional completion notification.

This contract separates logical layers from **physical groups**. Multiple
logical channels can be produced together but placed according to compatible
format/pool policy. That is a better model than assuming all PBR channels must
share one atlas or one eviction identity.

`bPersistentHighestMip` formalizes pinned coarse coverage. Public pool telemetry
also says there is usually one locked page per virtual texture, so loading many
VT assets consumes fixed pool space even when those textures are not visible.

## 4. Request and production protocol

`IVirtualTexture` is the RenderCore producer interface. Epic describes it as a
way to produce tiles from disk streaming, runtime compositing, or any other
source.

### Request

```text
RequestPageData(
  command list,
  producer handle,
  layer mask,
  virtual level,
  virtual address,
  Normal | High priority
) -> FVTRequestPageResult
```

`FVTRequestPageResult` contains an explicit status and an opaque 64-bit handle.
The status enum has four states:

| Status | Public meaning |
|---|---|
| `Invalid` | no data will ever be available |
| `Saturated` | request was not started because some subsystem is at capacity |
| `Pending` | production started but data is not ready |
| `Available` | requested data is ready |

This is unusually valuable public evidence. Capacity rejection is a first-class
result, distinct from permanent invalidity and asynchronous pending work.
Afterglow has similar internal states but should preserve this clarity at every
bounded stage.

### Produce

```text
ProducePageData(
  command list,
  feature level,
  flags,
  producer handle,
  layer mask,
  virtual level/address,
  request handle,
  target layer locations
) -> optional IVirtualTextureFinalizer
```

The API requires prior request status `Available` or `Pending`. `Pending`
production is legal and may block until ready; `ProducePageData` is render-thread
only. That policy is acceptable inside Unreal's scheduling assumptions but is
not acceptable as an Afterglow sealed frame-path contract.

For one virtual level/address, Unreal attempts to call request/produce once with
**all required layers set in one layer mask**. This allows one disk decode or
procedural operation to generate related channels together. `FVTProduceTargetLayer`
provides the exact pooled render target/texture and page coordinate for each
output layer.

Public methods also expose:

- whether a page is streamed;
- local per-address mip bias;
- task-graph events per producer or request;
- completion notification after a system update;
- producer-specific debug dumping.

The producer handle itself packs an index and magic value into 32 bits, providing
a generational/stale-handle shape rather than exposing a raw pointer identity.

## 5. Finalization and publication

`IVirtualTextureFinalizer` is the clearest Unreal ownership lesson. Epic defines
a specific frame phase where finalizers fill physical textures without hazards
and page tables are updated.

It splits work into:

1. `RenderFinalize`: read-only access to existing VT physical pools, allowing a
   runtime/material producer to sample VTs while preparing a new page;
2. `Finalize`: write-only access to VT physical pools, where output pages are
   committed and existing physical pools cannot be sampled.

The public contract does not expose exact barrier or page-table command order,
but it establishes centralized hazard ownership and prevents arbitrary producers
from publishing mappings throughout the frame. This is conceptually aligned
with Afterglow's atlas-write-before-resident-page-table ordering.

An authorized source audit must still verify whether every streamed producer
publishes only after upload completion and how stale requests are rejected at
this boundary.

## 6. Page-table allocation

`FAllocatedVTDescription` describes page-table and physical layout. It includes:

- producer handle and producer-layer mapping for each allocated layer;
- tile/border size;
- layer count and dimensions;
- indirection texture size and maximum address-space size;
- private-space flag;
- duplicate-layer sharing;
- adaptive-level bias/allocation;
- optional forced space ID.

Official settings expose three page-table arrangements:

- globally shared page-table atlas;
- private page table per VT;
- packed page-table channels;
- for RVT, an optional sparse adaptive page table supporting domains larger
  than the normal table at additional sampling cost.

Public memory documentation warns that page-table memory is allocated on demand,
can grow over time, and generally is not released until all its contents are
released. It has no user size controls. This is production evidence of a memory
behavior Afterglow explicitly must not copy into sealed gameplay.

## 7. Physical memory pools

Unreal creates separate physical pools keyed by tile size and one or more layer
formats. Each pool:

- is allocated on first matching VT use;
- has a configured fixed byte size in cooked builds;
- chooses the largest square page grid below that byte limit;
- acts as an LRU cache;
- contains free, visible/recent, and locked pages.

When full, the least recently seen page is evicted. Pool HUD telemetry reports:

- total occupancy;
- occupancy from locked pages;
- residency mip bias.

Pools may be fixed/configured or transient editor estimates. Editor auto-grow can
record observed demand, but cooked builds must ship deliberate pool capacities;
runtime auto-grow is separately configurable and is not a substitute for a
bounded release policy.

### Oversubscription

At 100% visible residency Unreal reports that visible pages are being dropped,
causing repeated I/O and flicker. A pool can enable residency mip bias to reduce
demand. The maximum active bias from **any** pool is applied globally to all VT
sampling.

This is directly descended from RAGE's high/low-water overload policy, though
the public documentation does not expose hysteresis thresholds or adjustment
rate. It confirms that a production VT should degrade requested detail rather
than continuously thrash an impossible physical working set.

## 8. Feedback and reactive latency

Epic documents that the GPU determines tiles touched by visible pixels using
normal depth visibility; the CPU receives those requests and streams pages.
Feedback resolution is a project setting: higher resolution increases CPU/GPU
cost but may lower latency, particularly for materials using many VTs.

SVT remains reactive. Epic explicitly warns that the CPU learns about a tile
after a frame already needed it, so rapid movement can produce visible popping.
The public documentation does not disclose:

- feedback buffer packing;
- GPU compaction/deduplication;
- readback ring depth;
- request-frequency or coverage weighting;
- stale-frame selection;
- prefetch rules;
- per-frame feedback/request capacities.

Those are source-audit items, not facts inferable from the user documentation.

## 9. Material sampling and layer stacks

Each VT sample performs a page-table/stack lookup plus a physical-layer fetch.
Samples with the same UVs and sampler source share one **VT stack**, with up to
eight layers amortizing the translation lookup.

Epic's examples describe:

- two layers sharing UVs: one stack lookup plus two physical texture fetches;
- two layers using different UVs: two stack lookups plus two physical fetches.

SVT dimensions must be powers of two but need not be square. The smallest normal
mips are constrained by tile size. Four-texel default borders limit available
anisotropy; larger borders cost memory. Stochastic trilinear can introduce noise
without temporal AA.

For Afterglow, the transferable concept is to share translation work for linked
channels, not necessarily to force those channels into one physical residency
unit.

## 10. Diagnostics and tuning

Official diagnostics include:

- `stat virtualtexturing` for timing/page-table counters;
- `stat virtualtexturememory` for memory counters;
- `r.VT.Borders` for tile/mip visualization;
- `r.VT.Residency.Show` pool graphs;
- `r.VT.Residency.Notify` oversubscription warnings;
- `r.VT.DumpPoolUsage` per-asset page counts sorted by usage.

The dump identifies pool format and physical page extent—for example a normal
128-payload/four-border configuration reports 136×136 physical pages. The
public troubleshooting guide explicitly calls out output resolution, page size,
layer format/count, negative mip bias, and zero gradients as causes of pool
overload.

This is a stronger operational reference than the open-source prototypes: pool
pressure is observable by format and asset, and overload has an explicit quality
response.

## 11. What public evidence does not establish

Without authorized implementation source, do not claim any of the following:

- the current cooked VT chunk/container header or offset table;
- source codecs used for normal SVT tile chunks;
- whether adjacent pages are physically or file-order clustered;
- I/O request merging and cancellation details;
- exact feedback encoding, GPU compaction, or CPU deduplication;
- cache data structure complexity or true LRU implementation;
- request, task, transcode, upload, or finalizer capacities;
- transcode/upload-cache keys and lifetime;
- page-table texel packing or update-region algorithm;
- exact shader address translation and derivative math;
- stale producer/request handling beyond the generational producer handle;
- content/upload completion ordering inside every concrete producer.

The general SVT docs say authored textures are cooked and can use lossy
compression. The documented Crunch option applies specifically to RVT streaming
low mips; it is not evidence that all current SVT assets use Crunch.

## 12. Authorized source audit checklist

If Epic source access is provided, the next pass should read the complete
current versions of:

1. `RenderCore/Public/VirtualTexturing.h` — producer/allocation contracts;
2. RenderCore virtual-texture registration and allocated-VT implementation;
3. Renderer `Private/VT/` system, physical-space, page-map, feedback, upload,
   and scalability files;
4. Engine `Private/VT/` uploading/streamed producer and chunk manager;
5. texture build/cook code defining `FVirtualTextureBuiltData` and data chunks;
6. VT feedback and sampling `.ush/.usf` shaders;
7. RHI upload/barrier paths used by concrete finalizers;
8. tests, automation, and debug validation for stale handles, saturation,
   eviction rescue, and publication order.

The audit must pin an Epic revision because generated API pages identify UE 5.8
but do not provide implementation commit hashes.

## 13. Transferable lessons for Afterglow

### Adopt or preserve

1. **Explicit saturation result.** Capacity exhaustion is neither pending nor a
   permanent page failure.
2. **Separate request from production.** Scheduling/I/O can complete before a
   known physical destination is committed.
3. **One request per address with a layer mask.** Share source work where linked
   channels are encoded/generated together.
4. **Logical layers vs physical groups.** Channel coupling is explicit policy,
   not an atlas accident.
5. **Central finalization.** Only one phase owns physical writes and page-table
   publication hazards.
6. **Persistent highest mip.** Coarse coverage has an explicit reservation and
   measurable fixed cost.
7. **Pool-specific telemetry plus global overload response.** Detect impossible
   working sets and reduce quality rather than churn I/O.
8. **Generational producer identity.** Packed index+magic handles reject stale
   owners without pointer lifetime ambiguity.

### Do not copy

1. Page-table memory that grows on demand without a sealed hard capacity.
2. A render-thread production call that may block on pending work.
3. Runtime pool auto-growth as a shipping failure policy.
4. General dynamic task/event ownership in Afterglow's sealed hot path.
5. A global mip bias without explicit per-channel quality constraints.
6. Unreal's large abstraction surface where Afterglow needs small fixed pools,
   rings, direct indexes, and deterministic overflow.

## 14. Relative value as a reference

| Reference | Best use for Afterglow | Main gap |
|---|---|---|
| id Tech 5 / RAGE paper | complete software asset-VT architecture and policy | no source |
| Unreal public contracts | production ownership, producer states, layers/pools/finalization | private implementation unavailable |
| GameTechDev SamplerFeedbackStreaming | modern public asynchronous tile I/O and lifecycle source | D3D12 sparse/sampler-feedback mechanics |
| shlomnissan prototype | concise software page-table/atlas shader | not production/bounded |
| Wicked/Dagor | runtime-generated terrain demand and fallback policy | not file-backed asset VT |

The best current reference strategy is therefore composite: RAGE for the
algorithm, Unreal for production contracts, GameTechDev for public asynchronous
streaming source, and small software-atlas projects for shader mechanics.
