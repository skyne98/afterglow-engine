# Virtual texturing production plan

This checklist tracks removal of the shortcuts exposed by `vt-demo.html`.
Items are completed in dependency order; an item is complete only with tests,
API documentation, and the user-facing book updated where applicable.

## Phase 1 — Correct GPU representation

- [x] **VT-01 Packed integer page tables.** Store every virtual mip in one
  vertically packed `r32uint` texture; provide tested offset/index helpers and
  make WGSL use mip level zero of that packed texture.
- [x] **VT-02 Incremental GPU writes.** Update one atlas slot and one page-table
  texel with `GPUQueue.writeTexture`; never mark a resident full texture dirty.
- [x] **VT-03 One authoritative GPU atlas.** Ensure Three.js samples the same
  `GPUTexture` that receives page uploads; remove the detached compressed-atlas
  allocation.
- [x] **VT-04 UV/filter correctness.** Implement clamp/repeat/mirror semantics,
  edge-safe coordinates, explicit gradients, and border-aware filtering.

## Phase 2 — Correct residency

- [x] **VT-05 Globally unique page identity.** Cache keys include texture ID as
  well as mip/X/Y; eviction clears the owning texture's page table.
- [x] **VT-06 Asynchronous state machine.** Use missing/queued/loading/resident
  states, reserve slots, deduplicate requests, and reject stale completions.
- [x] **VT-07 Per-texture pinning.** Pin levels relative to each texture's own
  maximum mip, not a global maximum.
- [x] **VT-08 Packed mip tails.** Pack all sub-page mips, keep the tail resident,
  and expose shader offsets/scales.

## Phase 3 — Real demand discovery

- [x] **VT-09 Scalable feedback IDs.** Encode texture identity plus at least
  eleven page-coordinate bits per axis; validate decode bounds.
- [x] **VT-10 GPU feedback pass.** Render at reduced resolution, asynchronously
  read back, deduplicate requests, and consume results one or more frames later.
- [x] **VT-11 Scheduling and prediction.** Connect feedback coverage, camera
  prediction, hysteresis, frame-time budget, and cancellation to the residency
  state machine.

## Phase 4 — Assets, validation, and tooling

- [x] **VT-12 Descriptor validation.** Validate dimensions, mip counts, page
  coordinates, compressed block layout, byte lengths, and GPU limits.
- [x] **VT-13 Offline page pipeline.** Tile real textures with correct borders,
  transcode pages to the selected format, and write seekable `.big` chunks plus
  mip-tail metadata.
- [x] **VT-14 Engine diagnostics.** Move atlas inspection, reconstructed virtual
  view, mip coloring, slow residency, page boundaries, and statistics from the
  demo into a reusable debug API.
- [x] **VT-15 End-to-end regression suite.** Cover multiple textures, races,
  eviction ownership, compressed uploads, feedback latency, mip tails, device
  limits, and long-running cache churn.
