# Virtual texturing

The web engine's virtual-texture implementation lives in
`crates/afterglow-web/www/engine/virtual-texture.ts`. It provides a shared
physical atlas, packed page-table entries, LRU residency, mip fallback, page
loading budgets, camera prediction, and adaptive LOD selection.

## Public surface

- `VirtualTextureStore(loader, pageDataProvider?, format?, device?)`
- `loadTexture(path, options?) -> AssetHandle<THREE.Texture>`
- `processFeedback(feedback)` accepts globally identified requests
  `{ path, mip, x, y }` and schedules deduplicated asynchronous page loads.
- `attachRenderer(renderer)` binds the actual Three.js backend textures so
  atlas-slot and packed-page-table changes use `GPUQueue.writeTexture`.
- `unloadTexture(path)` cancels pending generations and releases owned slots.
- `recordCamera(position, zoom)` and `recordFrameTime(milliseconds)` feed the
  prediction and adaptive-quality strategy.
- `poll()` advances asynchronous page work.
- `VirtualTextureFeedbackPass` renders a supplied feedback-material scene into
  an `RG32Uint` target at reduced resolution and performs non-blocking readback.
- `getDebugSnapshot()`, `setDebugPaused()`, `setDebugPageBudget()`, and
  `VirtualTextureDebugController` support reusable atlas and slow-residency UIs.
- `VirtualTextureRes` is the ECS resource definition.
- `detectBestTextureFormat(adapter?)` selects BC7, ASTC, or RGBA fallback.
- `VT_SAMPLE_WGSL` and `VT_FEEDBACK_WGSL` contain the sampling and feedback
  shader functions.

`AssetStore.setVirtualTextureStore(store)` enables universal VT;
`AssetStore.loadTexture()` then delegates to that store.

## Layout and encoding

Pages contain a 128x128 payload and a four-texel border on every side. A page
therefore occupies a 136x136 atlas slot. Every virtual mip table is packed
vertically into mip zero of an `r32uint` GPU texture. A resident entry is:

```text
1 | (physical_x << 1) | (physical_y << 9)
```

Bit zero is the resident flag. Sampling starts at the derivative-selected mip
and walks toward coarser mips until it finds a resident page. Mips smaller than
128x128 (64 through 1 texel) are packed with individual four-texel borders into
one additional pinned 136x136 mip-tail slot; its entry occupies the otherwise
unused x=1 texel of the terminal page-table row. Atlas sampling
uses explicit translated gradients and supports clamp, repeat, and mirrored
repeat addressing.

Feedback uses `RG32Uint`: word zero stores valid, six mip bits, and eleven bits
for each page coordinate; word one stores the full virtual-texture ID. This
supports the 2048x2048 page grid of a 256K texture without aliasing identities.

## Asset containers

`.big` headers identify VT chunks with
`ChunkMeta::VirtualTexturePage { mip, page_x, page_y }`.
`parseBigHeader()` and `createPageDataProvider()` in `big-parser.ts` locate and
load those chunks. `afterglow-pipeline process` now decodes PNG/JPEG sources,
requires square power-of-two dimensions of at least 128, generates filtered
mips, writes independently seekable 136x136 bordered pages, and emits a
`VirtualTextureMipTail` chunk. The offline-only `afterglow-basis-encoder` crate
then encodes every complete 136x136 slot independently as UASTC Basis. Runtime
`afterglow-texture` transcodes those chunks to BC7, ASTC, or RGBA. Raw RGBA
chunks remain explicitly tagged and are never sent to the Basis transcoder.

## Demonstration status

`afterglow-cef --example vt-demo` is an executable WebGPU shader demonstration.
It exposes a procedurally generated 262,144x262,144 noise texture (256 GiB if
materialized as RGBA8) through an `r32uint` page table packed vertically, a
fixed 2,176x2,176 RGBA atlas, WGSL page lookup, borders, derivative-based LOD,
fallback, and incremental page generation. Its current request source is a CPU
camera-coverage simulation. `VT_FEEDBACK_WGSL` exists in the engine, but
GPU feedback rendering/readback is not yet connected to this demo. Its live
atlas inspector mirrors changed slots into a corner canvas. Slow-debug mode
loads one page every 20 frames and applies a per-mip diagnostic tint in WGSL.

Run:

```sh
nix-shell shell.nix --run "cargo build --example vt-demo -p afterglow-cef"
nix-shell shell.nix --run "./target/debug/examples/vt-demo --ozone-platform=x11"
```

## Real-GPU regression

`scripts/test-vt-gpu.sh` launches the CEF/WebGPU demo and executes its CDP
self-test. It performs three independent CEF launches. Within every launch it
renders and precisely reads back 1,024 `RG32Uint` feedback pixels from eastward,
westward, and rotated raster directions; follows eastbound, westbound, and
diagonal trajectories across overview, paged, and one-texel LODs; byte-verifies
three RGBA writes at top-left, bottom-right, and interior locations; and submits
three supported compressed writes at corresponding atlas locations. Uncaptured
WebGPU validation errors and device loss fail the test.

```sh
DISPLAY=:0 ./scripts/test-vt-gpu.sh
```

On fox-laptop's Radeon 680M/RADV adapter, BC7 was selected. All three independent
launches passed; each launch validated all three directional feedback scenarios,
all three residency trajectories, three exact RGBA readbacks, and three BC7
uploads.

600-frame rAF measurements at 1440×900 held 59.97 FPS with zero drops during
stable rendering, bidirectional panning, and continuous streaming. Pathological
12-way per-frame teleports and full-cache 20-way thrashing each dropped one of
600 frames (p99 16.68 ms, max approximately 33.35 ms). Canonical methodology and
full results are in `docs/research/performance-benchmarks.md`.
