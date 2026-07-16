# Persistent Cache

Afterglow exposes a generic persistent byte cache. It is not a texture cache:
textures, meshes, shaders, games, and tools can all compose their own namespace
and key policy over the same API.

```ts
const namespace = await persistentCacheNamespace([
  'my-cache-v1', contentBuildId, deviceClass,
]);
const cache = await PersistentBlobCache.open({
  namespace,
  maxBytes: 1024 * 1024 * 1024,
  maxEntries: 65_536,
  writeQueueCapacity: 64,
});

const cached = await cache.get('asset:42:chunk:7');
if (!cached) {
  const bytes = await generateBytes();
  void cache.put('asset:42:chunk:7', bytes);
}
```

The cache prefers Origin Private File System storage. Chromium denies OPFS to
CEF's secure custom `afterglow://` origin, so CEF automatically uses the
persistent IndexedDB chunk backend; ordinary supporting HTTP(S) browsers use
OPFS. `getStats().backend` reports the selection. Each namespace still presents
one append-only value pack and one fixed-record index, rather than a file per value. Payloads
are published before index records, checksums detect corruption, partial index
records are ignored, and all capacities are hard limits.

`getStats()` reports physical/live size, queue depth, hits/misses, writes, LRU
evictions, compactions/reclaimed bytes, maintenance, rejection/corruption, and
I/O latency. `clear()` resets a namespace when no writes or maintenance are
pending. At capacity, fixed-array O(1) LRU eviction reduces live data to 75%
low water and one asynchronous maintenance task compacts into an inactive
pack/index generation. An atomic manifest switch publishes it only after
completion; no frame callback scans, sorts, evicts, or compacts.

## Virtual textures

Virtual texturing is a consumer, not part of the storage mechanism. The dungeon
namespaces final GPU blocks with:

- source ETag, modification identity, and size;
- cache/transcoder/page-layout versions;
- selected BC7, ASTC, or RGBA format;
- WebGPU adapter vendor, architecture, device, and description.

A device or format change therefore selects another cache automatically. A warm
cache hit bypasses the source `.big` range read and Basis transcode, then enters
the same priority scheduler and frame-paced GPU upload path as a miss. Cache
writes are asynchronous and never delay presentation.

The source container remains authoritative. If it has no stable ETag or
Last-Modified identity, persistent derived caching is disabled rather than
serving potentially stale output.

On fox-laptop, a cold run persisted 297 BC7 pages (5.49 MB). The next CEF
process loaded the same view with zero `.big` page reads and zero Basis
transcodes: 365 cache hits, no errors, and 8.21 ms average IndexedDB cache-read
latency. Three independent warm-cache GPU regression launches passed all nine
viewpoints.
