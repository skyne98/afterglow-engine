# Virtual Texturing

> **Current status:** public web uses Fetch plus WASM texture workers. The
> native shell uses source-backed OS texture workers: encoded Basis pages stay
> in native memory from confined `pread` through transcode. A 2026-07-26 Dungeon
> smoke run completed 400 native page transcodes with zero page failures;
> long-soak release validation remains open. The checked reference is
> `docs/api/virtual-texturing.md`.

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
longer compensates for pass scale. Dungeon uses a 55 ms monotonic feedback
cadence and one camera pose extrapolated 100 ms ahead, independent of refresh
rate. Invalid/suspended/teleport-like history resets to current pose.
The feedback word uses its three formerly spare bits for logarithmic camera
closeness. Each pixel contributes coverage plus equal predicted-center and
camera-distance weight; the store adds each channel's desired-to-resident mip
gap before capacity fitting, priority admission, bounded scheduling/upload
budgets, and fixed second-chance clock residency.
Resident lookup/touch is O(1); it does not scan or rebuild an LRU as the atlas
fills. Feedback expansion and capacity fitting reuse preallocated numeric
scratch records rather than constructing per-frame maps and channel objects. A
fixed persistent scheduler keeps requests that exceed one frame's budget.
One hundred fifty fixed-array lanes encode 25 perceptual importance buckets,
parent/exact kind, and albedo/normal/scalar channel order. The score follows
publicly documented Zhang/Cesium/RAGE mechanisms and is quantized without a sort
or runtime allocation. Each miss emits at most a mip+2 parent with a
non-resettable 1 ms batch window and one exact page. High-importance exact work
uses 16 ms; lower-importance exact work currently uses a provisional 64 ms lane.
The bucket-12 focus/peripheral split and the deadline remain measurement
candidates pending score histograms and the 32/48/64 ms GPU gate. Existing coarser pages and the pinned tail
remain visible immediately; intermediate ancestor requests are not created.
Pages absent from two newer feedback snapshots expire, approximately 110 ms
plus frame/readback quantization at any refresh rate. Newly important work can mark a strictly worse pending
load canceled, but its slot remains occupied until the asynchronous stage
acknowledges cancellation. In-flight public-web Fetch currently has neither transport abort propagation
nor a response deadline. Native source-backed jobs are one-in-flight per worker
and use ring backpressure rather than dropping completions. Atlas/page-table commits
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
evict useful pages. At most 16 jobs and 2 MiB of expected output may be pending;
stage budget exhaustion is reported rather than allowing an unbounded burst.
Pinned startup overflow remains in the highest-priority fixed scheduler until
resident and is not canceled by feedback staleness.
The public-web serving client owns a fixed 256-slot bulk queue. The live
provider preserves scheduler/admission order; a separate page-range helper can
source-sort and restore caller order but is not yet wired into that provider.
HTTP multipart responses are capped at 4 MiB with at most two / 8 MiB in flight.
Native Basis pages bypass this queue and send only fixed source-range
descriptors to the OS texture workers. Because color and data channels share one linear atlas, material nodes explicitly decode albedo with
Three.js `sRGBTransferEOTF`; normal and packed masks remain linear. The cook
currently box-filters every role in byte space and always clamps global-edge
border texels. Linear-light albedo mips, renormalized normal mips, and seam-
correct repeat/mirrored-repeat borders remain open quality work. Roughness
and AO are packed offline into mask R/G, reducing each material from four
streamed pages to three and sharing one shader sample. Each PBR channel resolves its own requested and fallback mip from its own page
table. Every channel retains a pinned tail, so partially streamed materials stay
valid while color detail arrives first. Eviction removes only the selected
physical page; mixed channel mips are intentional.

The compact VT directory introduced by `.big` v5 stores each mip as a
contiguous row-major block: one block offset, its page-grid dimensions, and
compact per-page sizes. Current writer version 6 retains that VT layout while
adding resident-texture format metadata; the bundled Dungeon remains v5. Runtime
expands those sizes once into typed offset/size arrays, allowing direct page
indexing and individual range reads without a serialized object per page. The
nine-channel dungeon header is 123,768 bytes, down from 764,192 bytes in v4.
The offline pipeline accepts arbitrary positive PNG/JPEG dimensions—including
rectangular and non-power-of-two sources—filters their mips, and emits 128x128
payloads with four-texel neighbor borders. Partial edge pages clamp only at the
real image boundary. Levels from
64x64 through 1x1 are packed into one permanently resident mip-tail slot rather
than wasting one physical slot per tiny level. Every complete slot is encoded independently as UASTC Basis offline, then
transcoded on demand to BC7, ASTC, or RGBA. Public web uses optimized texture
WASM Web Workers over the shared-memory ring. The native shell discovers named
services from application bootstrap and uses generated clients over real OS
workers. Encoded native source bytes never enter V8.

## Demos

The canonical model VT demo runs on `EngineRuntime`, `RendererHost`,
`EngineAssets`, stable-index material bindings, and atomic feedback. Its cook
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
bun scripts/build-web.ts
nix-shell shell.nix --run "cargo build --release -p afterglow-shell"
nix-shell shell.nix --run \
  "./target/release/afterglow-shell crates/afterglow-web/www/dungeon.html"
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
`www/dungeon.big` deployment asset. At runtime bounded serving-layer multi-range fetches and a fixed two-to-four
`TextureWorker` pool stream pages without the former page-side AssetLoader
latency. Urgent mip+2 restoration batches for at most 1 ms; perceptually
important exact work batches for 16 ms and lower-importance exact work currently
uses a provisional 64 ms peripheral lane. There is no persistent derived-page cache: every
nonresident page uses source read and transcode. The selected profile admits at most 16 pages/2 MiB and reserves sixteen
waiting jobs in addition to the active worker slots, so every admitted page has
a bounded pipeline slot. It
submits one feedback view predicted 100 ms ahead on a 55 ms monotonic cadence.
Aligned PBR
channels share one albedo feedback stream while loading and evicting
independently. The scheduler gives diffuse pages strict priority over
normal/emissive and scalar-mask pages; `MeshStandardNodeMaterial` samples each
channel's best resident fallback independently while retaining Three.js's PBR
shader. The dungeon profile caps the shared atlas at 53×53 slots
(`53 * SLOT_SIZE`, 2,809 pages) instead of expanding to the adapter's maximum
2D texture dimension. This bounds the active display working set and avoids
shared-memory bandwidth saturation on integrated GPUs:

```sh
nix-shell shell.nix --run "cargo run -p xtask -- serve"
# Launch hardware WebGPU Chromium on dungeon.html with CDP port 9333, then:
bun scripts/profile-dungeon-vt.ts --cdp 127.0.0.1:9333 \
  --scenario traverse --output-prefix docs/benchmarks/vt-traverse
./scripts/soak-dungeon.sh 600 traverse vt-traverse-10m.log
./scripts/baseline-vt-atlas.sh full vt-atlas-full.log
```

Dungeon is a canonical `EngineRuntime` consumer using `RendererHost`,
`EngineAssets`, the feedback coordinator, and bounded input/diagnostics. No
visual demo retains a global bridge or architecture-baseline exception. Its BIG
session feeds unified feedback, scheduler, page-load, bulk-wait/read, RPC,
transcode, upload, and page-table publication spans into `runtime.telemetry`. Diagnostic clients
can call `traceArm()`, run a bounded scenario, call `traceStop()`, and retrieve
the `AGTB` batch with `traceBatch()` through `window.__afterglowDungeon`. The
65,536-record Dungeon capture buffer is 2.5 MiB and remains prefix-preserving.

The pre-removal cold-cache RTX 3090 nine-pose baseline loaded 973 pages with no
failures while 582 scenario frames held 6.955 ms p99 and 13.900 ms maximum.
Complete page-load
latency was 149.0 ms mean / 231.4 ms p99: the 56.9 ms mean bulk batching wait
and 81.8 ms mean texture queue wait dominated the 3.0 ms bulk read, 11.4 ms
transcode, and 0.028 ms atlas publication stages. The accepted AGTB contained
24,850 records with zero drops and zero unmatched spans; raw evidence is under
`docs/benchmarks/dungeon-vt-unified-telemetry-rtx3090-2026-07-25.*`.

After cache removal and admission/deadline correction, hostile RTX 3090 page
latency fell to 42.02 ms mean / 58.40 ms p99; bulk wait was 9.11/16.45 ms and
transcode queueing 18.89/34.55 ms. All 582 frames stayed within 13.895 ms, with
zero failures, trace drops, or unmatched spans. Source bytes fell 13%; requests
rose 2.94× versus the former 100 ms batch policy. A tested 24 ms deadline still
missed the request gate and regressed latency. Deterministic trace replay then
reproduced all 156 requests: source sorting reduced modeled adjacent read runs
31% but not request count, while mip-deficit/channel grouping also stayed at
156. Those values remain the historical no-prediction baseline. The current
predicted-perceptual 16/64 ms candidate has not yet passed its RTX/680M gate and
must not inherit these performance claims. Evidence:
`docs/benchmarks/dungeon-vt-no-cache-rtx3090-2026-07-25.md` and
`docs/benchmarks/dungeon-vt-trace-replay-rtx3090-2026-07-25.md`.

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
and initially reported 10.63 ms GPU time. A later audit invalidated that number:
the external engine loop left Three's timestamp frame ID at zero, so unresolved
passes were grouped according to readback cadence, and the field called
`gpuMainMs` was actually the ~1.07 ms output color-transform pass rather than the
HDR scene. `RendererHost` now resets Three's external-loop counters and assigns
the engine frame ID before rendering. The timing API exposes the resolved frame plus
separate scene, output, feedback, and total durations; the misleading
`gpuMainMs` field was deleted. Corrected settled scene-plus-output means across
forward/reverse/corner were 4.19/4.28/5.84 ms without POM and
6.56/5.49/8.29 ms with POM; corner POM p99 was 10.49 ms. The permanent API then
resolved 80/80 monotonic forward/POM frames with exact scope sums and
5.211/1.083/0.006/6.301 ms scene/output/feedback/total means. An AMD
RGP 2.7 trace still identified a 4.56-million-pixel material draw as the dominant
event and showed POM increasing the fragment shader from 40 to 56 VGPR, reducing
theoretical occupancy from 12/16 to 9/16 without spills. Its traced 4.824/5.749
ms event durations are not production timings because SQTT was active and the
safe capture preceded settled fine-page residency. The earlier full baseline
reached all 3,600 slots with a 6.955 ms maximum rAF interval. A subsequent 1,014-eviction churn run averaged 6.970 ms, peaked at
20.850 ms, and missed one 17 ms threshold; failed loads, queue overflow, long
tasks, and GPU errors remained zero. The historical timestamp record's 0.149 ms
“main” value was actually Three's output transform; its independently measured
0.018 ms feedback pass remains valid.

The corrected close-wall streaming measurement bypassed the former page-side
AssetLoader latency and used four WASM texture workers on fox-laptop. It remains
public-web renderer/worker evidence. Native composition is now implemented and
has separate 400-page zero-failure smoke evidence, but no equivalent long native
soak yet. In the historical web run, forty-eight new physical PBR pages settled in
283.29 ms; a larger unseen view showed its first
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
