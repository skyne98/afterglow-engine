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

Install a `VirtualTextureStore` on `AssetStore` to route subsequent
`loadTexture()` calls into VT storage. The returned shared atlas is not a normal
`material.map`: use a VT-aware node material. `createVirtualGltfMaterialPair`
provides matched visible/feedback glTF base-color, optional normal, and optional
metallic/roughness materials for static, instanced, skinned, and morphed
geometry. `VirtualGltfBinding` joins cooked layouts by stable material indices
retained from `GLTFParser`, not names, and owns bounded replacement, exclusive
imported-texture disposal, shared-texture retention, and feedback
material/visibility restoration. UV channels, KHR texture transforms, and
repeat/clamp/mirror sampling are shared by visible and feedback materials;
linear and all-nearest samplers are supported, while mixed filtering or
asymmetric wrapping fails at bootstrap. Differently sized channels sample and
feed back independently. Aligned channels use one albedo feedback stream, but
residency and eviction are independent: albedo defaults to the requested mip,
normal/emissive to one level coarser, and masks/roughness/AO to two levels
coarser. Materials can override those integer biases during bootstrap. After Three.js initializes the textures,
call `attachRenderer()` so updates become small GPU subregion writes. Packed
page-table writes reuse fixed upload scratch instead of creating typed-array
views. Atlas page bytes must use an owned `ArrayBuffer`; incompatible shared
views are rejected at the WebGPU boundary. Each frame,
poll the store and submit the previous globally identified feedback results with
`processFeedback()`. Applications register fixed `FeedbackRenderable` records
with `VirtualTextureFeedbackCoordinator`, which owns target capacity, cadence,
warm-up, renderer-state restoration, and disposal. Multi-channel readbacks are
published atomically through `processFeedbackBatch()` only after every pass in
the logical snapshot completes, so one late channel cannot cancel another.
`VirtualTextureFeedbackPass` is the coordinator's low-level reduced-resolution
`RG32Uint` target and asynchronous readback primitive. It tracks the exact feedback-
to-physical-pixel scale after resize, so shader derivatives are converted back
to physical screen pixels before mip selection. Quality bias zero therefore
keeps approximately one to two source texels per screen pixel across arbitrary
texture dimensions, DPR, UV tiling, and feedback resolution; quality bias no
longer compensates for pass scale. Dungeon runs feedback every eight frames.
The pass retains each page's nearest-to-center sample and pixel coverage. The store adds capacity fitting, progressive quality
selection, priority admission, bounded scheduling/upload budgets, and fixed
second-chance clock residency.
Resident lookup/touch is O(1); it does not scan or rebuild an LRU as the atlas
fills. Feedback expansion and capacity fitting reuse preallocated numeric
scratch records rather than constructing per-frame maps and channel objects. A
fixed persistent scheduler keeps requests that exceed one frame's budget.
Sixty-six fixed-array lanes prioritize channel class first—albedo, then
normal/emissive, then scalar data. Inside each class, exact quality rungs restore
low→middle before middle→high before high→ultra, then center/large-coverage pages
before small edge pages. Aging cannot cross a channel-class boundary. The whole missing mip chain enters those lanes in one feedback update, so
quality no longer waits for another GPU readback after every rung. Waiting
requests age upward to prevent starvation. Pages absent
from two newer feedback snapshots expire (about 56 ms at 36 Hz), and newly
important work may preempt a strictly worse non-pinned in-flight load. Thus pages
behind a turning camera stop consuming the bounded queue quickly. Atlas/page-table commits
are governed by the central `VirtualTextureTuning` resource. Its configuration
is partial, so setting only `atlasMaxDimension` retains every upload default.
The cap is rounded down to a whole 136-texel slot grid; zero selects the device
limit. The tuner starts at two completed pages and 0.20 ms per frame, tightens
after repeated overloaded rAF
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
streamed pages to three and sharing one shader sample. Each PBR channel resolves its own requested and fallback mip from its own page
table. Every channel retains a pinned tail, so partially streamed materials stay
valid while color detail arrives first. Eviction removes only the selected
physical page; mixed channel mips are intentional.

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

The canonical model VT demo runs on `EngineRuntime`, `RendererHost`,
`BigAssetSession`, stable-index material bindings, and atomic feedback. Its cook
extracts image pages, preserves sampling metadata in an ignored extension, and
removes browser image payloads from the runtime GLBs; the current `.big` shrank
from roughly 633 MiB to 463,702,085 bytes. Press **1** for the first animated rig
or **2** for Spooky Iluha's
Sci-Fi Dragon Warrior: 18 skinned meshes, an Idle clip, and 45 independent
virtual material images from 512² through 4096². The cook embeds its external
`.gltf`/`.bin`/texture package before stripping image buffer views. Each model is
range-packed into `.big`; images become `<model>#image-N` paged/UASTC textures.
The session-owned `AssetStore` parses both and sends
each material group's index range through the meshopt cache/overdraw worker. Only triangle order changes: joints, weights,
normals, tangents, UVs, morph targets, bind matrices, and animation tracks keep
their original vertex identity. The same animated `SkinnedMesh` is rendered
with prewarmed visible and feedback materials, so page requests follow the
current deformation rather than a bind-pose proxy. Animated meshes cast and
receive a bounded 2048² directional PCF shadow, and the floor receives their
silhouette; reduced VT-feedback renders skip redundant shadow work. Use **W/S**
to zoom and **A/D** to orbit with inertia; **B** shows the active skeleton.
The real-GPU regression covers both rigs and reports zero post-seal pipelines.

All demos use the production `VirtualTextureStore`, shader, scheduler, and
packed page tables—there is no separate demo cache. The canonical `vt-demo`
uses `EngineRuntime`, `RendererHost`, `VirtualMaterialBinding`, and the feedback
coordinator, and its three-launch GPU regression passes every trajectory.
It displays a 262,144×262,144 terrain texture (256 GiB
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
namespace; warm hits skip both range reads and Basis transcodes. Aligned PBR
channels share one albedo feedback stream while loading and evicting
independently. The scheduler gives diffuse pages strict priority over
normal/emissive and scalar-mask pages; `MeshStandardNodeMaterial` samples each
channel's best resident fallback independently while retaining Three.js's PBR
shader. The dungeon profile caps the shared atlas at 53×53 slots
(`53 * SLOT_SIZE`, 2,809 pages) instead of expanding to the adapter's maximum
2D texture dimension. This bounds the active display working set and avoids
shared-memory bandwidth saturation on integrated GPUs:

```sh
DISPLAY=:0 ./scripts/run-dungeon.sh
DISPLAY=:0 ./scripts/test-dungeon-gpu.sh
# With the dungeon running, capture per-second telemetry and frame timing:
./scripts/soak-dungeon.sh 600 traverse vt-traverse-10m.log
./scripts/baseline-vt-atlas.sh full vt-atlas-full.log
```

Dungeon is a canonical `EngineRuntime` consumer using `RendererHost`,
`BigAssetSession`, the feedback coordinator, and bounded input/diagnostics. No
visual demo retains a global bridge or architecture-baseline exception.

Click for raw mouse look; use **WASD**, **Shift**, **P**, **R**, and **1–3** for
movement, sprint, prewarmed close-range POM toggle, reset, and deterministic
viewpoints. The POM tier uses official resident 8-bit R8 displacement from the
same scanned stone materials while the 8K PBR channels remain virtual. The
offline `resident-texture` cook quantizes the lossless `.r16` interchange to
R8 and a `blue-noise` cook generates the ray-start dither tile; both load via
`loadResidentTexture` as `r8unorm` (filterable, no `float32-filterable` required).
Its feedback variant runs the
same bounded POM march, requesting pages at displaced UVs while deriving mip
footprints from the original continuous UV. Base/POM feedback pipelines are
prewarmed and switch with the visible material. Automated clients use
`window.__afterglowDungeon` for exact poses, collision-aware movement, frame
stepping, idle waits, allocation-free telemetry reads, snapshots, and named
scenarios. Soak modes are `stable`, `traverse`, and deliberately hostile
`thrash`; atlas baselines are `cold`, `half`, `full`, and `churn`. The desktop
session must remain unlocked.

On fox-laptop, the constrained 2,809-slot atlas completed 1,087 initial visible
uploads with zero failures and drained pending, ready, and scheduled work to
zero. The measured GPU render time was 13.36 ms, down from roughly 26 ms with
the device-maximum atlas. With independent `0/+1/+2` channel mips, a fresh run
settled at 1,788 slots: regular page tables reported 1,271 albedo, 395 normal,
and 113 mask pages, plus nine pinned mip-tail slots. It had zero failures/errors
and measured 10.63 ms GPU render time. A matching AMD RGP 2.7 capture identified
the dominant 4.56-million-pixel material draw at 4.824 ms base and 5.749 ms POM.
POM added 0.924 ms while increasing the fragment shader from 40 to 56 VGPR and
reducing theoretical occupancy from 12/16 to 9/16; neither variant spilled.
The earlier full baseline reached all 3,600 slots with a 6.955 ms maximum rAF
interval. A subsequent 1,014-eviction churn run averaged 6.970 ms, peaked at
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
