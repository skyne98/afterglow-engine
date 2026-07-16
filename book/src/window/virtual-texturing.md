# Virtual Texturing

Afterglow's texture path uses a shared physical atlas instead of allocating one
GPU texture per asset. Textures are divided into 128x128 payload pages with a
four-texel border. The shader translates virtual UVs through a page table and
falls back to a resident coarser mip while detailed pages stream in.

This bounds texture memory and lets the runtime prioritize only visible pages.
The atlas format is selected for the GPU: BC7 on supported desktop adapters,
ASTC where available, and RGBA as the compatibility fallback.

> **Current status:** the complete path is a correctness prototype. A July 2026
> vertical-slice audit found progressive long-session lag from superlinear cache
> operations, redundant page-thread worker polling, linear page-directory
> lookup, unbounded staged work, and movement-heavy GPU/feedback costs. Do not
> treat short stationary benchmarks as production stability evidence yet. The
> first remediation pass removed the array LRU, indexed page directories once,
> consolidated worker polling, and bounded admissions; persistent scheduling
> and the remaining audit items are still in progress.

Enable the path by installing a `VirtualTextureStore` on `AssetStore`; subsequent
`loadTexture()` calls delegate to it. After Three.js initializes the textures,
call `attachRenderer()` so updates become small GPU subregion writes. Each frame,
poll the store and submit the previous globally identified feedback results with
`processFeedback()`. `VirtualTextureFeedbackPass` provides the reduced-resolution
`RG32Uint` render target and asynchronous readback. Fragment derivatives select
the mip directly in that target's viewport and supports an explicit quality
bias (the dungeon uses `-1.5`). The dungeon submits this pass every four frames,
retaining 36 Hz residency updates on a 144 Hz display without forcing a GPU
readback every frame. The feedback pass retains each page's nearest-to-center
sample and pixel coverage. The store adds capacity fitting, progressive quality
selection, priority admission, bounded scheduling/upload budgets, and fixed
second-chance clock residency.
Resident lookup/touch is O(1); it does not scan or rebuild an LRU as the atlas
fills. Feedback expansion and capacity fitting reuse preallocated numeric
scratch records rather than constructing per-frame maps and channel objects. A
fixed persistent scheduler keeps requests that exceed one frame's budget.
Twenty-two fixed-array lanes prioritize exact quality rungs—low→middle before
middle→high before high→ultra—then center/large-coverage pages before small edge
pages. The whole missing mip chain enters those lanes in one feedback update, so
quality no longer waits for another GPU readback after every rung. Waiting
requests age upward to prevent starvation. Pages absent
from two newer feedback snapshots expire (about 56 ms at 36 Hz), and newly
important work may preempt a strictly worse non-pinned in-flight load. Thus pages
behind a turning camera stop consuming the bounded queue quickly. Atlas/page-table commits
are governed by the central `VirtualTextureTuning` resource. It starts at two
completed pages and 0.20 ms per frame, tightens after repeated overloaded rAF
samples while work is queued, then probes one step after each clean 15-active-
frame window toward the configured device caps. Evidence survives short empty
backlog gaps, so this calibrates during bootstrap and short gameplay bursts use
the discovered throughput. A bad promoted cap rolls back to the independently
validated two-page / 0.20 ms baseline and waits through a cooldown, so powerful
GPUs can discover higher sustained throughput without continuous oscillation.
Excess completed work remains in a
fixed ready ring for a later frame rather than
causing a full-cache presentation burst. Physical slots are acquired only after bytes are ready, so slow reads/transcodes do not
evict useful pages. At most 64 jobs and 8 MiB of expected output may be pending;
stage budget exhaustion is reported rather than allowing an unbounded burst. Because color and data
channels share one linear atlas, material nodes explicitly decode albedo with
Three.js `sRGBTransferEOTF`; normal and packed masks remain linear. Roughness
and AO are packed offline into mask R/G, reducing each material from four
streamed pages to three and sharing one shader sample. PBR
materials resolve one fallback level that is resident in all four page tables,
then sample every channel at that level, preventing mixed-mip material shading.
Evicting one logical material page removes its resident albedo, normal, and mask
siblings together.

The `.big` v5 asset format stores each mip as a contiguous row-major block:
one block offset, its page-grid dimensions, and compact per-page sizes. Runtime
expands those sizes once into typed offset/size arrays, allowing direct page
indexing and individual range reads without a serialized object per page. The
nine-channel dungeon header is 123,768 bytes, down from 764,192 bytes in v4.
The offline pipeline accepts arbitrary positive PNG/JPEG dimensions—including
rectangular and non-power-of-two sources—filters their mips, and emits 128x128
payloads with four-texel neighbor borders. Partial edge pages clamp only at the
real image boundary. Levels from
64x64 through 1x1 are packed into one permanently resident mip-tail slot rather
than wasting one physical slot per tiny level. Every complete slot is encoded
independently as UASTC Basis offline, then transcoded on demand to BC7, ASTC, or
RGBA by an optimized runtime texture Web Worker. Payloads travel through the
shared-memory ring transport, so transcoding never blocks rendering on the page
thread.

## Demos

Both demos use the production `VirtualTextureStore`, shader, scheduler, and
packed page tables—there is no separate demo cache.
The `vt-demo` CEF example displays a 262,144×262,144 terrain texture (256 GiB
logical RGBA), while generating only requested bordered pages:

```sh
nix-shell shell.nix --run "cargo build --example vt-demo -p afterglow-cef"
nix-shell shell.nix --run "./target/debug/examples/vt-demo --ozone-platform=x11"
```

Run the automated real-GPU regression with `DISPLAY=:0 ./scripts/test-vt-gpu.sh`.
It validates feedback rendering/readback and compressed atlas subregion uploads
on the active adapter. On fox-laptop, stable rendering, panning, and continuous
streaming held 59.97 FPS with zero drops across each 600-frame measurement;
extreme per-frame teleport/cache-thrash tests dropped one frame per 600.

Use **WASD** to pan, the mouse wheel to zoom, **P** for a one-texel view, and
**O** for the full overview.

The `dungeon` example is a minimal first-person corridor using three scanned
8K PBR materials. A demo script extracts their albedo, OpenGL normal, roughness,
and AO PNGs and runs the generic asset pipeline once to produce the ignored
`www/dungeon.big` deployment asset. At runtime exact serving-layer
`fetch + Range` reads and a fixed two-to-four `TextureWorker` pool stream pages
without the former page-side AssetLoader latency. Final GPU blocks are also
stored through the generic persistent cache under a source/format/adapter
namespace; warm hits skip both range reads and Basis transcodes. GPU feedback and linked
material sets keep matching pages resident for all three physical channels while
`MeshStandardNodeMaterial` retains
Three.js's PBR shader. The shared atlas expands to the largest whole page grid
supported by the active GPU:

```sh
DISPLAY=:0 ./scripts/run-dungeon.sh
DISPLAY=:0 ./scripts/test-dungeon-gpu.sh
# With the dungeon running, capture per-second telemetry and frame timing:
./scripts/soak-dungeon.sh 600 traverse vt-traverse-10m.log
./scripts/baseline-vt-atlas.sh full vt-atlas-full.log
```

Click for raw mouse look; use **WASD**, **Shift**, **P**, **R**, and **1–3** for
movement, sprint, prewarmed close-range POM toggle, reset, and deterministic
viewpoints. The POM tier uses official resident 16-bit displacement from the
same scanned stone materials while the 8K PBR channels remain virtual. Automated clients use
`window.__afterglowDungeon` for exact poses, collision-aware movement, frame
stepping, idle waits, allocation-free telemetry reads, snapshots, and named
scenarios. Soak modes are `stable`, `traverse`, and deliberately hostile
`thrash`; atlas baselines are `cold`, `half`, `full`, and `churn`. The desktop
session must remain unlocked.

On fox-laptop, the full baseline reached all 3,600 slots with a 6.955 ms maximum
rAF interval. A subsequent 1,014-eviction churn run averaged 6.970 ms, peaked at
20.850 ms, and missed one 17 ms threshold; failed loads, queue overflow, long
tasks, and GPU errors remained zero. Full-state WebGPU timestamp queries measured
0.149 ms for the main context and 0.018 ms for feedback.

The corrected close-wall streaming path bypasses the former page-side
AssetLoader latency and uses four texture workers on fox-laptop. Forty-eight new
physical PBR pages settled in 283.29 ms; a larger unseen view showed its first
page at 79.32 ms and first 12 coarse-priority pages at 132.75 ms. Mean page
admission-to-ready latency fell from 445.81 ms to 26.25 ms, with zero failed
loads or overflows.

Corrected 10/30/60-minute real-GPU soaks held a 6.950 ms mean across stable,
traversal, and eight-way per-frame teleport modes (863,264 total frames). All
runs ended with zero pending work, failed loads, queue overflow, long tasks,
GPU errors, or post-seal pipelines. GC-floor heap repeatedly returned to about
77–79 MiB. Timestamp tracking is disabled during long soaks because Three r185
retains per-frame diagnostic timestamp keys; short captures clear them after
resolution.
