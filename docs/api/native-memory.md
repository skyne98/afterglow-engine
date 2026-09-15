# Native shared byte admission

Source: `crates/afterglow-memory/src/lib.rs`.

`EngineMemory` is a shared native byte-admission resource.
It is not a general allocator and does not change the public-web memory policy.

## Operations

- `EngineMemory::new(limit)` creates a shared counter with a fixed byte limit.
- `reserve(bytes)` returns a `Reservation`, or `None` without a charge.
- `Reservation::bytes()` gives the charge.
- Dropping the reservation releases its charge.
- `stats()` gives `limit`, `used`, and `peak` bytes.
- `installed_bytes()` reads installed physical RAM.
- `process()` returns the shared native process resource.

The process limit is half of installed RAM, rounded down to 64 MiB blocks.
The artist does not configure this limit.
Atomic admission prevents concurrent owners from exceeding the same counter.
A reservation must remain live until its associated storage is no longer live.

## Paint and canvas consumers

The native paint cache reserves metadata capacity and owner scratch before tile growth.
Each resident tile has a separate reservation.
When shared admission rejects growth, the cache reuses an unpinned LRU tile through SQLite writeback.
Live pins still prevent eviction.
The store reserves capacity for its bounded maps, growth and commit overlap, buffers, and cache headroom before recovery reads.

Native canvases use the same process resource, not another half-RAM limit.
Canvas replacement reserves new CPU and GPU capacity before it replaces CPU pixels.
Source GPU charges remain live through previously submitted work.
Cold snapshots retain their reservation through immutable image clones.
There is no fixed canvas-count limit or artist configuration.

## Evidence limits

These counters describe admitted storage and conservative headroom, not measured process RSS.
They do not intercept V8, SQLite, Vello, graphics-driver, or OS allocations.
The GPU charge includes atlas headroom but does not give an exact atlas-allocation count.
The SQLite cache size remains a target, not an allocator limit.
Process memory and long-duration checks must accompany the admission tests.
Do not claim a hard process-memory or GPU-driver bound from the counter alone.

The six memory tests, shared-cache pressure check, canvas admission test, and both ten-minute 16K runs passed.
Native reservations peaked at 2,499,842,048 bytes under a 32,413,581,312-byte limit.
The final three-minute native RSS floor increased by 118,784 bytes. The browser process-tree PSS floor decreased.
These different process measures are not directly comparable.
See `../benchmarks/native-paint-shell/final-validation.md` for the threshold, raw evidence, and latency limits.
