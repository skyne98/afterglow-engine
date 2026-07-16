# Per-device transcoded texture cache

**Investigated:** 2026-07-16

> **Implementation status:** The first production slice is implemented as the
> generic, policy-free `PersistentBlobCache`: a bounded append-only pack, fixed
> SHA-256 index, checksums, crash-safe publication order, OPFS with IndexedDB
> fallback, and stable telemetry. VT composes device/source/format namespaces
> over it. Fixed-array O(1) LRU eviction and crash-safe two-generation
> asynchronous compaction maintain hard limits without frame-time work.

## Question

Should Afterglow repeatedly transcode portable UASTC/Basis virtual-texture pages
at runtime, or persist the selected GPU-native result for the current device?
How do other engines handle this?

## Conclusion

Yes: Afterglow should retain portable UASTC in shipped `.big` containers and add
a bounded, persistent **derived texture cache** keyed by the source build,
selected GPU format, transcoder ABI, and device signature. A cache hit should
skip Basis transcoding and return the stored 136×136 BC7/ASTC/RGBA page directly
to the existing priority/upload pipeline.

This is a hybrid of the two common industry models:

- Web engines commonly ship Basis/KTX2 once and transcode at runtime after
  detecting device support.
- Native engines commonly derive target-platform texture data during import or
  cooking and retain/reuse that derived output rather than repeating work on
  every launch.

Afterglow ships one portable web asset set through CEF and browsers, so a
per-device derived cache preserves universal distribution while gaining the
repeat-load behavior of a native cooked pipeline.

## What other engines do

| Engine / standard | Behavior | Persistent derived GPU cache? |
|---|---|---|
| Khronos KTX2 / `KHR_texture_basisu` | The standard explicitly expects engines to transcode the universal payload at runtime into a block format supported by the platform. | Container/standard does not prescribe a persistent post-transcode cache. |
| Three.js `KTX2Loader` | `detectSupport(renderer)` selects a supported target and a worker pool transcodes at load time. Its `_taskCache` is a `WeakMap` keyed by the current `ArrayBuffer`, preventing duplicate work for that in-memory buffer. | No built-in persistent post-transcode cache; normal browser/HTTP caching retains the KTX2 source. |
| Babylon.js KTX2 decoder | Selects a transcoder module/target from the universal representation and formats supported by the current device, then transcodes at runtime, normally off-thread. | No documented built-in persistent post-transcode cache. |
| Unity | Imports/cooks textures for explicit target-platform formats. Crunch adds distribution compression over DXT/ETC and is decompressed at runtime to the already selected GPU family. Platform overrides can produce different artifacts per build target. | Yes in the import/build pipeline; shipping players normally consume target-derived data rather than universally transcoding every launch. |
| Unreal Engine | Derived Data Cache stores expensive derived asset data while developing. Cooked builds package target data and do not normally use DDC at runtime. | Yes during derivation/cooking; shipping data is already target-specific. |
| Godot | The importer generates VRAM-compressed platform variants (for example S3TC/BPTC and ETC2/ASTC), and platform support chooses among imported forms. Source comments explicitly require importing desktop and mobile formats in priority order. | Yes in `.godot/imported`/exported artifacts, rather than a universal runtime transcode on every launch. |

Runtime transcoding is therefore normal for portable web delivery, but repeating
it on every launch is not a requirement. Native-style derived caching is a good
fit for Afterglow's persistent CEF installation.

## Sources

- Khronos KTX overview: Basis Universal produces compact textures that can be
  transcoded to GPU formats at runtime: <https://www.khronos.org/ktx/>
- Khronos `KHR_texture_basisu`: engines are expected to transcode the universal
  texture into a platform-supported block-compressed format at runtime:
  <https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_texture_basisu/README.md>
- Three.js `KTX2Loader` API (`detectSupport`, worker limit):
  <https://threejs.org/docs/pages/KTX2Loader.html>
- Three.js loader source map showing the in-memory `_taskCache` keyed by an
  `ArrayBuffer`: <https://app.unpkg.com/three-stdlib@2.35.2/files/loaders/KTX2Loader.cjs.map>
- Babylon.js KTX2 documentation/source: target module is selected from the
  universal representation and device-supported compressed formats:
  <https://github.com/BabylonJS/Documentation/blob/master/content/features/featuresDeepDive/materials/using/ktx2Compression.md>
- Unity texture compression fundamentals: target GPU formats are prepared by
  the import/build pipeline; Crunch is layered over DXT/ETC and decompressed at
  runtime: <https://docs.unity3d.com/6000.3/Documentation/Manual/texture-compression-fundamentals.html>
- Unreal Engine Derived Data Cache; cooked builds do not normally use DDC:
  <https://dev.epicgames.com/documentation/en-us/unreal-engine/using-derived-data-cache-in-unreal-engine>
- Godot texture importer source, including priority-ordered desktop/mobile VRAM
  formats: <https://github.com/godotengine/godot/blob/master/editor/import/resource_importer_texture.cpp>
- Basis Universal transcoder configuration; transcoding generally remaps blocks
  without a full texel decompress/recompress cycle:
  <https://github.com/BinomialLLC/basis_universal/wiki/How-to-Use-and-Configure-the-Transcoder>

## Afterglow cache identity

The selected target format is the primary namespace, but format alone is not a
sufficient invalidation key. Use:

```text
cache_schema_version
container_build_id
asset/page identity (texture ID, mip, x, y, tail)
source page encoding/version
transcoder ABI/version
target GPU format (BC7, ASTC, RGBA)
slot dimensions and border layout
adapter signature
```

The adapter signature should record the available WebGPU `GPUAdapterInfo`
fields (`vendor`, `architecture`, `device`, and `description`) plus the selected
format. GPU-native BC7/ASTC blocks are standardized and often remain valid when
the GPU changes but the selected format does not. Nevertheless, namespacing by
adapter signature follows the requested conservative behavior and protects
against device-specific workarounds. Old namespaces can be reclaimed by normal
cache eviction.

A driver version is not required for block compatibility. If a driver-specific
workaround changes emitted data, increment the cache schema/transcoder ABI or
include an explicit workaround ID.

The current `.big` v5 directory has coordinates and offsets but no immutable
container build ID. A production cache that can skip even the source range read
needs the pipeline to emit a content-derived build ID in the container header.
Without it, hashing the UASTC page after reading can safely skip transcoding but
cannot skip the source read.

## Storage layout

Do not create one filesystem file per page. A complete nine-channel dungeon can
contain tens of thousands of ~18.5 KiB BC7 pages, and per-file metadata would be
wasteful.

Use a bounded append-only pack plus a compact fixed-record index:

```text
cache/<schema>/<container-build>/<adapter-format>/
  pages.pack
  pages.index
  manifest
```

Each index record maps the numeric page identity to pack offset, byte length,
checksum, and last-access generation. Writes append payload first, then publish
an atomic index/manifest replacement. Interrupted writes remain unreachable.
Compaction is incremental and budgeted. A size limit and high/low water marks
provide deterministic eviction; 1–2 GiB is a reasonable configurable desktop
default, not a hard API decision.

## Platform backends

### CEF and browser

The implemented shared cache prefers OPFS. Chromium rejects OPFS access for
CEF's secure custom `afterglow://` origin, so CEF automatically uses a
persistent IndexedDB chunk backend; supporting HTTP(S) browsers use OPFS. Both
own append/index loading, publication, byte limits, and metrics without
introducing texture policy. An alternate native
Rust `PersistentBlobBackend` can be added later without changing the public
cache or VT consumer. It must not write inside the confined game asset root.
Browser quota/eviction is
not guaranteed, so every miss must transparently fall back to UASTC range read
and transcode. The source `.big` remains authoritative.

## Runtime flow

1. Select BC7/ASTC/RGBA and collect adapter signature during renderer bootstrap.
2. Open the matching cache namespace and load its bounded numeric index.
3. For a prioritized VT page, check the in-memory cache index in O(1).
4. On hit, asynchronously read the final GPU block payload and enter the existing
   ready-upload ring.
5. On miss, range-read UASTC, transcode in the worker pool, return the result,
   and enqueue a budgeted cache append independently of GPU upload.
6. Never delay presentation waiting for a cache write.
7. Keep current stale cancellation, quality/center priority, and adaptive upload
   pacing unchanged.

## Required telemetry and tests

Track hits, misses, corrupt entries, bytes read/written, cache size/high-water,
append backlog, compaction work, evictions, and hit/miss latency independently
from source read/transcode/upload telemetry.

Regression coverage must include:

- warm-launch hits perform zero Basis transcodes;
- GPU/format/build/schema changes miss the old namespace;
- interrupted append and corrupt checksum recover as misses;
- stale/canceled pages do not force cache publication;
- fixed capacities and byte limits reject deterministically;
- cache writes never block the presentation path;
- repeated warm traversal plateaus in cache size and heap usage.
