# Allocation boundaries

Gameplay hot regions may not allocate engine-authored storage. The following
operations are explicit boundaries because browser, Three.js, worker codec, or
game-facing APIs necessarily allocate.

| Boundary | Effect classification | Why allocation remains | Admission/measurement |
|---|---|---|---|
| Asset HTTP range fetch | `budgeted` | `fetch`, `Response.arrayBuffer`, browser networking | VT serving-layer work is bounded by 16 pages / 2 MiB admitted plus two / 8 MiB bulk responses; pending bytes/counters |
| Worker RPC convenience Promise | `gameFacing` | JS Promise and owned response envelope | 256 task slots; 32 completions/poll; capacity rejection |
| Persistent blob cache | `budgeted` | SHA-256, OPFS/IndexedDB I/O, value buffers, maintenance Promise | Caller hard byte/item limits; fixed LRU/index, bounded write queue, 75% low-water two-generation compaction |
| Relative pointer input | `budgeted` | Browser event objects and pointer-lock permission Promise | One prebound passive handler; authored movement callback allocates nothing; raw event with deterministic fallback |
| Native audio device setup | `bootstrap` | CPAL/ALSA/PipeWire stream and host-owned buffers | Fixed 48 kHz stereo stream; callback only drains persistent native PCM views, scales, and updates atomics |
| Public-web audio setup | `bootstrap` | `AudioContext`, module load, browser output arrays | Fixed SAB ring and persistent views; AudioWorklet callback performs no allocation, messaging, waiting, or WASM |
| R16 displacement load | `bootstrap` | `fetch`, `ArrayBuffer`, typed-array view, Three/WebGPU texture creation | Exact dimensions/byte length; three resident maps; fail closed without `float32-filterable` |
| Surface detail / POM | `none` hot path | GPU fragment work only; resident height textures allocated at bootstrap | 8–32 bounded view layers + 8 bounded light-shadow steps, no radial fade; prewarmed base/POM material references |
| Basis transcode | `budgeted` | Codec output and postcard response vectors | Shared 64-job ring over 2–4 independent one-in-flight workers; output bytes counted by VT admission |
| Image/model parse + meshopt | `budgeted` | GLTF/Three objects, copied worker arguments/results, replacement index buffers | Fixed AssetStore IDs/completion ring; bootstrap/warm-up only; skinned optimization preserves vertex identity |
| VT material construction | `bootstrap` | Three node/material/uniform objects and shader pipelines | Fixed visible/feedback pair; both variants prewarmed before seal |
| Feedback readback | `budgeted` | Three/WebGPU asynchronous readback buffer | One outstanding readback; two retained maps and pooled requests |
| Renderer pipeline compile | `bootstrap` | Browser/Dawn pipeline implementation | Declared variant warm-up; post-seal pipeline monitor |
| Native rAF callback registration | `gameFacing` | User/Three callback references are retained until the presentation turn | Fixed 1,024-slot bootstrap queue; O(1) request/cancel, deterministic overflow and high-water telemetry; no growing `Map` or per-frame callback snapshot allocation |
| Native `scheduler.yield()` | `gameFacing` | Standards-shaped continuation necessarily creates a Promise and deno lazy op | Explicit Scheduler subset; one bounded Tokio opportunity per host turn, no timer/microtask loop; used by Three pipeline warm-up before gameplay seal |
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
