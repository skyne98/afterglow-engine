# Native large-canvas paint repair

## Status

The repair and its defined final validation pass are complete on the tested Linux/RTX 3090 system.
Task `becb57128` completed `scripts/test-paint-final.sh` with exit code 0.
It passed 108 Rust tests, 94 TypeScript tests, builds, GPU/recovery checks, and both ten-minute 16K runs.
See `docs/benchmarks/native-paint-shell/final-validation.md` for results, reproduction, and evidence limits.

This is not a native/web performance-parity claim.
Native completion across the initial 256 strokes measured 97.379 ms mean and 7,274.183 ms maximum.
Web completion measured 28.191 ms mean and 169.575 ms maximum.
The aggregate timings do not identify the cause of the native maximum.
A separate latency investigation remains necessary.

## Accepted decisions

- Keep one generated native RPC paint owner on an OS thread.
- Use a bounded native thread pool for independent tile jobs.
- Grow RAM allocations as needed, up to 50% of installed RAM.
  This is a ceiling, not an initial allocation or a target for forced allocation.
- Use disk after the resident memory budget is insufficient.
- Limit scratch storage to 64 GiB, with a 16 GiB free-space reserve.
- Use SQLite transactions through a native Rust binding.
  Keep the byte-store mechanism separate from paint tile and history policies.
- Keep crash-recovery data for completed strokes and document changes.
- At startup, offer Restore or Discard. Do not silently delete a recovered drawing.
- Keep the public-web worker and IndexedDB paths.

The user also requested automatic memory selection, no artist configuration, and maximum practical freedom.
No fixed canvas-count limit was selected.
These decisions replace the previous memory-only native paint policy.

## Completed implementation

1. `afterglow-storage-worker::byte_store` supplies bounded values, operation counts, input bytes, database capacity, and atomic transactions.
   The existing public blob service and its format remain unchanged.
   A connection-owned VFS bounds combined database and rollback-journal file extents before growth.
   It accepts only the bundled Unix/Windows parent VFS and exact database/journal paths.
2. `afterglow-memory` supplies one automatic process admission resource.
   Paint, cache/store capacity, canvases, snapshots, and source GPU headroom use this resource.
   The native UI no longer applies the public-web 2 GiB ceiling.
   Admission grows beyond 4,096 tiles and reuses unpinned disk-backed tiles when another reservation cannot fit.
3. The native document owner uses a pinned LRU cache, immutable SQLite tile versions, and completed-document roots.
   It stages at most 32 tiles per transaction and retains at most 40 undo changes.
   Incomplete work does not replace the completed recovery root.
   Metadata retains layer identities, layer/group order and properties, background, and display EOTF.
4. Native tile dispatch uses a fixed Rayon pool with at most 16 OS threads.
   Jobs keep exclusive tile pins until completion.
   Serial/parallel pixel and smudge-history regressions passed.
5. Native canvases retain CPU pixels, one pending region, and a persistent source GPU texture.
   Initial publication and resize copy the full raster. Later publication copies changed rows only.
   Cold CPU capture creates an immutable snapshot with a live reservation.
   Texture retirement retains its charge until submitted work completes.
   Vello still copies the full changed source texture into its GPU atlas.
6. Confirmed rollback restores the previous completed drawing before the adapter resumes commands.
   Remaining samples from the rejected stroke do not change that drawing.
   Unconfirmed restoration and invalid readback remain fatal rather than publishing damaged pixels.
7. Restore/Discard controls block document input until the decision and pixel publication complete.
   Restore uses saved dimensions. New and Discard explicitly authorize atomic replacement.
   Native bootstrap supplies a trusted storage root. RPC input cannot select filesystem paths.

## Verification

| Check | Result |
|---|---|
| Rust tests | 108 passed, including the explicit hardware GPU check |
| TypeScript tests | 94 passed, 3,470 assertions |
| TypeScript, WASM, Vite, native release | Passed |
| Generated-web build and drift | Passed |
| GPU region pixels, capture, resize, retirement | Passed exact same-image-identity comparisons |
| Physical Restore/Discard after process termination | Passed, with saved composition and history after Restore |
| 16,384 × 16,384, radius 60, ten minutes per target | Native 5,593 strokes, web 6,173 strokes, no engine errors |
| Growth beyond the initial block | Both targets retained 5,120 tiles under a 6,144-tile admitted limit |
| Native/web sampled pixels | 131,072 bytes matched exactly across eight display rows |
| Memory-floor comparison | Both targets passed the final three-minute floor-growth check |
| Native shared reservation peak | 2,499,842,048 bytes under a 32,413,581,312-byte limit |
| SQLite file-extent peak | 201,658,944 bytes under 64 GiB |
| mdBook and diff check | Passed |

Store tests include reopen, malformed data, transaction limits, database/journal exhaustion, rollback, and subsequent writes.
Child-process tests terminate before and after atomic commit.
Cache tests use a deliberately small shared allowance and retain exact pixels after eviction and restoration.
Native worker tests inject paging failure into a stroke beyond the former capture limit and retain a usable command queue after rollback.
The final physical recovery run used private temporary storage and verified its pointer window before every click.

## Measurement limits and remaining performance work

Reservations describe admitted capacity and conservative headroom, not process RSS or a general allocator intercept.
They do not directly count V8, SQLite, Vello, graphics-driver, or OS allocations.
The native RSS floor increased by 118,784 bytes in the final comparison window.
The browser process-tree PSS floor decreased. Those measures are not interchangeable.
The floor gate permits the larger of 32 MiB or five percent growth.
Ten minutes does not prove an unlimited-session memory bound.

The combined SQLite limit concerns logical file extents, not filesystem metadata or block-allocation overhead.
The free-space check cannot prevent another process from consuming disk space afterward.
Neither process-exit checks nor quota tests simulate every physical storage failure.

The pixel comparison covers eight display rows, not the complete document.
The native and headless-web rAF cadences differ.
rAF is frame-production timing, not GPU presentation or input-to-pixel latency.
The native 7.274-second stroke-completion maximum needs investigation before a smooth-input or performance-parity claim.

## Earlier evidence

- `docs/benchmarks/native-paint-shell/large-canvas-live-1788764384/`: original history-capture failure.
- `docs/benchmarks/native-paint-shell/recovery-check/`: initial physical recovery check.
- `docs/benchmarks/native-paint-shell/raster-baseline/`: full-copy baseline and controlled persistent-texture experiment.
- `docs/benchmarks/native-paint-shell/raster-regions/`: initial runtime region measurements.
- `docs/benchmarks/native-paint-shell/recovery-regions-ready/`: recovery after region integration.

The full-copy 4K commit measured 14.826 ms mean for a 16 KiB change.
The final region run measured 0.00596 ms mean and 0.026 ms p99 CPU commit time.
These separate runs do not establish identical frame conditions or an end-to-end speedup.
