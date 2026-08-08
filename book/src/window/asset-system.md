# The Asset System

afterglow-engine uses positional, range-capable sources on both targets. Public
web loads cooked containers through HTTP Range requests. The native shell uses
confined filesystem sources and real OS workers; it never instantiates engine
services as WASM.

## Separation of responsibilities

| Layer | Responsibility |
|---|---|
| `afterglow-assets` | Path confinement, `AssetSource`, fixed source caches/tables, and byte-range parsing. No texture or container policy. |
| `afterglow-assets-worker` | Generated asynchronous `load`, `size`, and `read` service for JS-visible native bytes. |
| `BigContainer` | Immutable BIG header/index and exact raw-asset ranges. Owns no workers or renderer state. |
| `OwnedWorkerPool` | Generic fixed worker lifetime, startup rollback, and reverse shutdown. |
| `EngineAssets` | Public composition owner for one cooked container, its engine-owned services, asset store, model system, and virtual-texture system. |
| `VirtualTextureSystem` | Sole public texture namespace; opaque handles compose static, procedural, and mutable sources over internal format pools. |

Games and demos use `EngineAssets`; they do not construct RPC transports,
select worker scripts, depend on numeric worker IDs, or manage worker shutdown.

## Streaming sources

```rust
pub trait AssetSource {
    fn len(&self) -> u64;
    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize>;
    fn etag(&self) -> Option<String>;
}
```

Native `FsSource` uses positional `pread`, so reads have no shared cursor.
`AssetRoot` canonically confines paths. `AssetSourceCache` retains a fixed set
of open descriptors; it cannot grow with runtime history.

`AssetSourceTable` assigns bootstrap-opened sources numeric generational
handles. Runtime consumers use `{source, offset, length}` rather than resolving
or allocating path strings. Clearing the table invalidates old generations.

## Opening engine assets

```ts
import { EngineAssets } from './engine/assets/index.ts';

const engineAssets = await EngineAssets.open({
  containerPath: 'world.big',
  format,
  transcodeQueueCapacity: 16,
  maxPendingPages: 16,
  maxPendingBytes: 2 * 1024 * 1024,
  urgentBatchDeadlineMs: 1,
  focusBatchDeadlineMs: 16,
  peripheralBatchDeadlineMs: 64,
  maxHeaderBytes: 2 * 1024 * 1024,
  telemetry: runtime.telemetry,
});

const header = engineAssets.container.header;
const assets = await engineAssets.createAssetStore(64, 8);
const models = await engineAssets.createModelSystem({
  maxModels: 256,
  maxPendingOptimizations: 8,
  maxResidentCpuBytes: 256 * 1024 * 1024,
  completionsPerPoll: 2,
  ratios: [1, 0.5, 0.25, 0.1],
  targetError: 0.02,
  geometryArena: { buckets: [{
    slots: 1024,
    maxVertices: 65536,
    maxIndices: 196608,
    maxGroups: 8,
    indexKind: 'u32',
    attributes: [
      { name: 'position', itemSize: 3, kind: 'f32' },
      { name: 'normal', itemSize: 3, kind: 'f32' },
      { name: 'uv', itemSize: 2, kind: 'f32' },
    ],
    morphAttributes: [],
  }] },
});
const virtualTextures = engineAssets.createVirtualTextureSystem({
  maxTextures: 4096,
  maxMutablePageRefreshesPerPoll: 2,
  device,
  pools: [
    { format: 'bc7-rgba-unorm-srgb', capacities: {
      maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024,
    }, tuning },
    { format: 'rgba8unorm', capacities: {
      maxPendingPages: 16, maxPendingBytes: 2 * 1024 * 1024,
    } },
  ],
});
```

The engine selects worker topology: native uses one OS texture worker per
physical core, capped by 16 admitted pages, while public web remains capped at
two to four WASM workers. `workerCount` exists only as a test/profile override.
The texture waiting capacity plus active workers must cover every admitted VT
page. Invalid capacities fail before I/O. Header parsing completes before any
worker starts. Partial startup rolls back in reverse order, and `close()` is
idempotent while still attempting every owned service after an error.

`BigContainer` is the authoritative container surface. `EngineAssets` does not
republish aliases for its header, source, path, or raw loader. Internally,
BIG decoding, deployment ranges, the immutable VT page directory, deadline
batching, transcode dispatch, and final page decoding live in separate focused
modules.

## Native texture pages

Native Basis pages do not travel through JavaScript:

```text
VT scheduler
  → {source, offset, length, format}
  → native TextureWorker
  → confined pread
  → Basis transcode
  → final BC7/ASTC/RGBA page
  → atlas upload
```

The application bootstrap registers named native services. TypeScript asks the
shell for the ordered `texture` or `meshopt` manifest instead of embedding
worker IDs. The reference native bootstrap publishes `min(physical cores, 16)`
workers—16 on the current 16-core/32-thread host. Each texture worker opens the
BIG source once, retains its own numeric source handle, and reads into one
preallocated 4 MiB input scratch.

Only the final GPU-format page enters V8 today. Direct worker-to-atlas upload is
a later measured optimization, not a requirement for correctness.

## JS-visible native bytes

Headers, resident textures, and raw models use the generated native asset
worker. Reads are split into at most 512 KiB ring payloads and return V8-owned
arrays. There is no reusable native arena whose release depends on garbage
collection.

The async response ring applies backpressure when full and retries the
completion. Dropping a completion is forbidden because it would leave the
matching JavaScript promise pending forever.

## Public web

Public web uses Fetch Range requests. Non-contiguous VT reads can share one
bounded multipart response:

- at most 256 spans;
- at most 4 MiB per complete response;
- at most two responses / 8 MiB in flight;
- strict multipart `Content-Range` validation.

The public-web provider preserves scheduler order. The explicit diagnostic/tool
`createSourceSortedPageReader()` uses the same immutable `VtPageDirectory`, can
source-sort spans, and restores caller order. It is intentionally not the live
scheduling policy.

Web texture transcoding uses generated WASM Web Workers. This is exclusively the
public-web fallback and is never selected by the native shell.

## Asset store

`AssetStore` has fixed asset and completion capacities. Bootstrap registration
returns numeric `AssetId`s. Promise callbacks only enqueue fixed completion
records; `poll()` publishes a bounded number per frame. Generation tokens
prevent canceled or evicted reads from publishing stale results. It has no
optional virtual-texture owner and no texture-loading policy switch: resident
textures use the resident texture API and virtual textures use
`VirtualTextureSystem`.

`ModelSystem` uses the same fixed generational ownership primitive as the new
texture namespace. Cooked disk LODs and runtime RAM geometry share handles and
atomic revisions. Runtime primitives—including skinned and morphed geometry—go
through meshoptimizer's attribute-aware simplifier; all attributes and morph
targets follow one compact remap while every LOD shares the complete skeleton.
A mandatory ModelSystem-owned `GeometryArena` publishes those complete revisions
into configured fixed, prewarmed Three-compatible slots without exposing
renderer internals to games. Incompatible layouts or exhausted slots reject the
whole revision while retaining the previous publication.

## Failure policy

- Source/container identity or header corruption fails bootstrap.
- Missing or corrupt page data records a permanent page fault and retains the
  resident coarse/tail fallback.
- Queue capacity is admission/backpressure, not a page failure.
- Every queue, source table, worker pool, and completion path has an explicit
  fixed capacity and stable telemetry.

See `docs/api/asset-system.md` for the complete checked API reference.
