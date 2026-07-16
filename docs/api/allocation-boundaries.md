# Allocation boundaries

Gameplay hot regions may not allocate engine-authored storage. The following
operations are explicit boundaries because browser, Three.js, worker codec, or
game-facing APIs necessarily allocate.

| Boundary | Effect classification | Why allocation remains | Admission/measurement |
|---|---|---|---|
| Asset HTTP range fetch | `budgeted` | `fetch`, `Response.arrayBuffer`, browser networking | VT serving-layer ranges are bounded by 64 pages / 8 MiB in flight; pending bytes/counters |
| Worker RPC convenience Promise | `gameFacing` | JS Promise and owned response envelope | 256 task slots; 32 completions/poll; capacity rejection |
| Persistent blob cache | `budgeted` | SHA-256, OPFS snapshots/writables, value buffers | Caller hard byte/item limits; fixed index and bounded write queue; stable telemetry |
| Basis transcode | `budgeted` | Codec output and postcard response vectors | Shared 64-job ring over 2–4 independent one-in-flight workers; output bytes counted by VT admission |
| Image/model parse | `budgeted` | `Blob`, `createImageBitmap`, GLTF/Three objects | Fixed AssetStore IDs and completion ring; loading/warm-up only |
| Feedback readback | `budgeted` | Three/WebGPU asynchronous readback buffer | One outstanding readback; two retained maps and pooled requests |
| Renderer pipeline compile | `bootstrap` | Browser/Dawn pipeline implementation | Declared variant warm-up; post-seal pipeline monitor |
| Timestamp resolution | `diagnostic` | Query result mapping and Three maps | Once per trace second, never per frame |
| Debug snapshot/formatting | `diagnostic` | Arrays, strings, Three serialization | On explicit request; soak uses stable telemetry instead |
| Reactive refs/effects | `gameFacing` | Closures and dependency Sets | Excluded from engine no-allocation ownership |

`engine-allocation-effects.json` classifies every authored engine module and
every sealed region. CI rejects missing/stale classifications and unpermitted
calls from `none` regions into budgeted boundaries.

## Measured behavior

The 2026-07-16 sequential atlas baseline recorded `usedJSHeapSize` deltas of
+2.3 MiB (cold startup), −2.1 MiB (half), +5.8 MiB (full), and −3.9 MiB
(churn). Positive and negative deltas demonstrate GC noise and are not treated
as allocation counts. More importantly, pending bytes/queue depths returned to
zero, no queue grew beyond configured capacity, no long task was observed, and
full/churn work completed without monotonic heap growth across those scenarios.
Long-duration plateau validation remains a release gate.

Rust `TrackingAllocator::assert_no_alloc` seals selected ring operations. JS
syntax/effect lint proves only engine-authored source policy; browser/Three
internals are measured through heap, long-task, pipeline, and GPU telemetry.
