# Persistent blob store

## Public API

`PersistentBlobStore` is a generic, policy-free bounded byte store. It does not
know texture, model, cache, save-slot, editor, or networking semantics.

```ts
const store = await createPlatformPersistentBlobStore("game-save", {
  maxItems: 64,
  maxBytes: 256 * 1024 * 1024,
  maxValueBytes: 32 * 1024 * 1024,
  maxInFlightOperations: 2,
  maxInFlightBytes: 64 * 1024 * 1024,
}, telemetry);

await store.get(key, maxBytes);
await store.putAtomic(key, bytes);
await store.remove(key);
await store.clear();
store.getStats();
await store.close();
```

Keys are opaque ASCII identifiers containing letters, digits, `.`, `_`, or `-`
(up to 128 bytes). Namespaces use the same set and are limited to 64 bytes.
Consumers construct keys and choose save cadence, conflict resolution, undo,
cloud synchronization, and invalidation policy.

## Capacity and failure

The constructor requires hard item, total-byte, value-byte, in-flight-operation,
and in-flight-byte capacities. Admission reserves item/byte deltas before I/O;
concurrent writes cannot collectively exceed configured totals. A key permits
one operation at a time. Exhaustion returns `PersistentBlobStatus` and never
grows a queue.

`getStats()` returns one stable object containing current totals, in-flight
counts, high-water marks, rejects, operation counts, and I/O errors. Unified
telemetry appends `storage/blob.read` and `storage/blob.write` spans plus exact
read/write byte counters.

## Backends

- **Native shell:** `NativePersistentBlobBackend` uses the generated
  `BlobStorageClient` over the native RingBuffer bridge. `afterglow-shell`
  composes one `BlobStorageWorker` as a real OS worker. Values transfer in
  sequential 512 KiB chunks below the 1 MiB response-ring ceiling. Native files
  use two checksummed generations plus an atomically renamed one-byte pointer;
  file and containing-directory synchronization occur before success.
- **Public web:** `WebPersistentBlobBackend` owns only the generated
  `BlobStorageClient`. Its dedicated `storage-worker.ts` consumes the standard
  request/response RingBuffers; `postMessage` carries init and wake-ups only.
  OPFS is visible exclusively inside that Worker. Values transfer in sequential
  512 KiB chunks into one of eight fixed transaction slots. Two checksummed
  generations and atomic pointer publication retain the previous valid value
  after an interrupted slot or pointer write.
- `MemoryPersistentBlobBackend` exists only as a deterministic test adapter.

Native storage root selection is `AFTERGLOW_STORAGE_ROOT`, then
`$XDG_DATA_HOME/afterglow-engine/storage`, then
`$HOME/.local/share/afterglow-engine/storage`. Namespace isolation occurs below
that root. Native list responses are capped at 4,096 entries.

## Mutable virtual textures

`VirtualTextureSystem.saveMemoryTexture(handle, key, store)` writes a portable,
versioned sparse snapshot. `loadMemoryTexture(...)` fully decodes and restores
an unpublished source before registering a handle. Corrupt, truncated,
duplicate, incompatible, or over-capacity snapshots cannot mutate a live
texture.

Snapshots contain descriptor interpretation, dimensions, address mode, mip
filter, default texel, logical revision, and canonical mip-zero sparse pages.
They deliberately exclude derived mips, borders, atlas slots, page tables, GPU
format state, and residency. Derived data is regenerated after load. The format
has a whole-snapshot CRC32 in addition to the backend generation checksum.

## Allocation boundary

Persistence is `budgeted`: promises, OPFS/native file APIs, encoded snapshots,
result objects, and owned response buffers allocate in the storage worker or
explicit caller slow path. It never runs inside a `none` frame stage. Item/byte
admission, eight worker transaction slots, 512 KiB RPC chunks, RingBuffer
capacity, and telemetry make those allocations explicit and bounded.
