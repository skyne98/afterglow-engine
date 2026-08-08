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
