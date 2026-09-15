# Persistent Storage

Afterglow exposes `PersistentBlobStore`, a bounded byte mechanism rather than a
texture cache or save-game policy. Games choose namespaces, keys, save cadence,
conflict handling, and cloud behavior.

```ts
const blobs = await createPlatformPersistentBlobStore("my-game", {
  maxItems: 64,
  maxBytes: 256 * 1024 * 1024,
  maxValueBytes: 32 * 1024 * 1024,
  maxInFlightOperations: 2,
  maxInFlightBytes: 64 * 1024 * 1024,
});
```

Every operation has typed capacity/I/O status. The store tracks fixed item and
byte limits, reserves concurrent writes before dispatch, and returns stable
high-water telemetry. Atomic replacement retains the previous complete value if
a write is interrupted.

Both targets use the generated `BlobStorageClient` and bounded RingBuffer
payloads. The native shell composes one native filesystem OS worker. Public web
spawns one dedicated storage Worker; only that Worker can access OPFS.
`postMessage` initializes and wakes it but never carries storage payloads.
Sequential 512 KiB chunks, eight fixed transactions, and two checksummed
file generations bound writes and retain the prior valid value after interruption.

Mutable virtual textures can be saved and loaded explicitly. Their portable
sparse snapshot stores only canonical mip-zero pages and source interpretation;
derived mips, borders, page tables, and atlas residency are regenerated. A
corrupt or over-capacity snapshot is rejected before a texture handle is
published.

## Native transactions for multiple values

The native `afterglow-storage-worker` crate also contains `byte_store::ByteStore`.
This Rust component commits bounded groups of byte values in one SQLite transaction.
One native worker owns the connection. This is not a new service transport.
The existing blob service and its file format remain unchanged.

The operations are `open`, `open_bounded`, `get`, `keys_after`, `apply`, `stats`, and `peak_file_bytes`.
`keys_after` reads at most 1,024 keys into caller storage, in unsigned numeric order.
Each batch starts after the previous batch's last key. Separate batches do not form a snapshot.
The caller supplies value, transaction, database, and cache capacities.
Database-full errors retain the previous transaction and do not permanently disable the store.

The database-size limit excludes the rollback journal.
`open_bounded` also checks combined database and journal file growth before OS writes.
It uses the bundled Unix or Windows SQLite VFS, one connection, DELETE journaling, and memory-only temporary storage.
`peak_file_bytes` gives its combined high-water charge.
The file limit excludes filesystem metadata and block-allocation overhead.
The cache setting does not establish a hard total-memory limit.
Native paint paging and recovery use this store.
The final quota/recovery regressions and both ten-minute 16K drawing checks passed.
See `docs/benchmarks/native-paint-shell/final-validation.md` for results and limits.
See [`docs/api/transactional-byte-store.md`](../../../docs/api/transactional-byte-store.md) for the source-checked contract.
