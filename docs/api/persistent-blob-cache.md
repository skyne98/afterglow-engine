# Persistent blob cache API

`crates/afterglow-web/www/engine/persistent-blob-cache.ts` provides generic,
policy-free persistent byte storage. It does not know about textures, assets,
GPUs, paths, or codecs.

## Public surface

### `PersistentBlobCache.open(options, backend?)`

Opens or creates a bounded cache.

```ts
const cache = await PersistentBlobCache.open({
  namespace: 'content-and-device-hash',
  maxBytes: 1024 * 1024 * 1024,
  maxEntries: 65_536,
  writeQueueCapacity: 64,
});
```

Options are hard limits. A cache never grows past `maxBytes` or `maxEntries`;
`put()` returns `false` on capacity or queue exhaustion. Supplying `backend` is
primarily useful for tests or alternate platform storage.

### `get(key) -> Promise<Uint8Array | null>`

Hashes the caller-owned string key with SHA-256, performs fixed-capacity O(1)
index lookup, reads the value, and verifies its checksum. Missing, corrupt, and
backend-failed reads return `null`; counters distinguish the reason.

### `put(key, bytes) -> Promise<boolean>`

Queues a bounded asynchronous append. It returns `true` after payload and index
publication, or `false` on deterministic rejection/failure. Callers need not
block use of the bytes on cache publication.

### `clear() -> Promise<void>`

Truncates pack/index state. It rejects while writes are pending.

### `getStats() -> Readonly<PersistentBlobCacheStats>`

Updates and returns one stable telemetry object. It reports entries/bytes,
queue depth, hit/miss/write counts, capacity/queue rejection, corruption and
I/O errors, and read/write latency.

### `persistentCacheNamespace(parts) -> Promise<string>`

Length-prefixes the supplied identity parts and returns a full SHA-256 namespace
hex string. Consumers decide which policy fields belong in a namespace.

### `OpfsBlobBackend` and `IndexedDbBlobBackend`

Open prefers OPFS and falls back to IndexedDB. OPFS stores `values.pack` and
`values.index` under `OPFS/afterglow-cache/<namespace>/`. Chromium blocks OPFS
for CEF's secure custom `afterglow://` origin, so CEF uses the disk-backed
IndexedDB chunk backend automatically. Ordinary supporting HTTP(S) browsers use
OPFS. `getStats().backend` reports the selected mechanism.

`PersistentBlobBackend` is the small generic backend contract: `size`, `read`,
`append`, and `replace`.

## Storage and crash semantics

Values are appended to one pack. The index uses fixed 48-byte records containing
the full SHA-256 key, pack offset, length, and checksum. Payload append completes
before index append, so interrupted writes can only leave unreachable pack
suffixes. Partial index records are ignored at open. The complete SHA-256 key is
retained, avoiding hash-alias ambiguity.

The in-memory index is preallocated at `2 × maxEntries` and uses open addressing
with tombstones. Cache operations are budgeted slow paths; no cache API belongs
inside sealed frame computation.

The initial implementation is bounded and saturating: it rejects new values at
its hard limits rather than running frame-time eviction or compaction. `clear()`
or a new namespace performs deterministic replacement. Incremental compaction
and eviction can be added behind the same generic API without texture policy.

## Virtual-texture composition

The dungeon is only one consumer. It builds a namespace from source
ETag/Last-Modified/size,
cache schema, selected BC7/ASTC/RGBA format, transcoder/layout versions, and
WebGPU adapter identity. Page coordinates become ordinary cache keys. On hit,
`createPageDataProvider` skips both source range read and Basis transcode; on
miss it stores final GPU block bytes asynchronously after transcode.

If the source lacks ETag or Last-Modified identity, the dungeon disables the
persistent cache rather than risk stale bytes. GPU/device or format changes
select another namespace automatically.
