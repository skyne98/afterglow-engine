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
readback every frame; the store adds only minimum capacity fitting, deduplication,
bounded scheduling/upload budgets, and fixed second-chance clock residency.
Resident lookup/touch is O(1); it does not scan or rebuild an LRU as the atlas
fills. Feedback expansion and capacity fitting reuse preallocated numeric
scratch records rather than constructing per-frame maps and channel objects. A
fixed persistent scheduler keeps requests that exceed one frame's
budget and expires requests absent from sixteen newer feedback snapshots
(about 444 ms at the dungeon's 36 Hz feedback rate). Physical
slots are acquired only after bytes are ready, so slow reads/transcodes do not
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

The `vt-dungeon` example is a minimal first-person corridor using three scanned
8K PBR materials. A demo script extracts their albedo, OpenGL normal, roughness,
and AO PNGs, runs the generic asset pipeline once, and caches the resulting
Basis `.big` container under `/tmp`. At runtime AssetLoader range reads,
TextureWorker transcoding, GPU feedback, and linked material sets keep matching
pages resident for all three physical channels while `MeshStandardNodeMaterial` retains
Three.js's PBR shader. The shared atlas expands to the largest whole page grid
supported by the active GPU:

```sh
DISPLAY=:0 ./scripts/run-vt-dungeon.sh
DISPLAY=:0 ./scripts/test-vt-dungeon-gpu.sh
# With the dungeon running, capture per-second telemetry and frame timing:
./scripts/soak-vt-dungeon.sh 600 traverse vt-traverse-10m.log
./scripts/baseline-vt-atlas.sh full vt-atlas-full.log
```

Click for mouse look; use **WASD**, **Shift**, **R**, and **1–3** for movement,
sprint, reset, and deterministic viewpoints. Automated clients use
`window.__afterglowVtDungeon` for exact poses, collision-aware movement, frame
stepping, idle waits, allocation-free telemetry reads, snapshots, and named
scenarios. Soak modes are `stable`, `traverse`, and deliberately hostile
`thrash`; atlas baselines are `cold`, `half`, `full`, and `churn`. The desktop
session must remain unlocked.

On fox-laptop, the full baseline reached all 3,600 slots with a 6.955 ms maximum
rAF interval. A subsequent 1,014-eviction churn run averaged 6.970 ms, peaked at
20.850 ms, and missed one 17 ms threshold; failed loads, queue overflow, long
tasks, and GPU errors remained zero. Full-state WebGPU timestamp queries measured
0.149 ms for the main context and 0.018 ms for feedback.

Corrected 10/30/60-minute real-GPU soaks held a 6.950 ms mean across stable,
traversal, and eight-way per-frame teleport modes (863,264 total frames). All
runs ended with zero pending work, failed loads, queue overflow, long tasks,
GPU errors, or post-seal pipelines. GC-floor heap repeatedly returned to about
77–79 MiB. Timestamp tracking is disabled during long soaks because Three r185
retains per-frame diagnostic timestamp keys; short captures clear them after
resolution.
