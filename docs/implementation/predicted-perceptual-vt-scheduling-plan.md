# Predicted perceptual VT scheduling plan

Status: **P1–P6 accepted 2026-07-25; minimal implementation landed, generated
artifact/unit gates pending final run; 32/48/64 ms real-GPU selection and target
acceptance remain open**.

This plan replaces the current binary center priority with one small perceptual
page score evaluated from the camera pose expected 100 ms in the future. The
score balances predicted screen-center proximity, camera distance, screen
coverage, and desired-to-resident mip gap.

The score is adapted from known-good public systems audited in
[`../research/virtual-texture-perceptual-priority-score.md`](../research/virtual-texture-perceptual-priority-score.md):

- Zhang et al.'s evaluated per-fragment coverage + distance + displayed-mip
  weight;
- Cesium's camera-center foveated factor;
- RAGE's desired-versus-resident mip gap and feedback sample count.

The first version deliberately prioritizes KISS and YAGNI. It does **not** add
provider tickets, mutable downstream priorities, priority transcode queues,
priority upload queues, dual production feedback passes, game teleport hints,
or a second scheduler.

## 1. Selected product behavior

1. Sample camera movement every frame.
2. Extrapolate one camera pose 100 ms ahead.
3. Render the existing feedback pass once from that predicted camera.
4. Compute one page importance from:
   - predicted-center proximity;
   - camera closeness;
   - feedback coverage;
   - current fallback mip deficit.
5. Admit the highest importance page first.
6. Immediately demote scheduled/pending pages whose latest predicted importance
   falls.
7. Preserve bounded work already admitted to I/O/transcode/upload; do not build
   a mutable priority protocol for every stage without evidence.
8. Give low-importance exact pages a longer bulk deadline so prediction can
   reduce request count, not merely reorder pages.

This supersedes the earlier absolute “center is always number one” rule. A deep
corridor at screen center should compete with camera-close detail near the edge.
No single factor is absolute.

## 2. Selected decisions

### P1. Perceptual score — selected

Use equal 3-bit contributions:

```text
qf ∈ [0, 7]  closeness to predicted screen center
qd ∈ [0, 7]  closeness to predicted camera
qg ∈ [0, 7]  desired-to-nearest-resident mip gap
```

For each feedback pixel:

```text
pixelWeight = 1 + qf + qd
```

For each deduplicated page:

```text
Wpage = Σ pixelWeight + min(coverage, 255) × qg
```

The base `1` makes fragment count/coverage part of the score. All three added
terms have equal maximum influence. This preserves the balancing behavior the
user requested.

Alternative: make center or distance lexicographically dominant. Rejected by
default because it recreates the deep-center or close-edge failure in the other
direction.

### P2. Distance normalization — selected

Pack logarithmic camera-relative distance between the active camera's near and
far planes:

```text
d = clamp(log2(viewDistance / near) / log2(far / near), 0, 1)
qd = round(7 × (1 - d))
```

This requires no world-unit convention or game tuning slider. Logarithmic depth
gives the foreground useful precision when the far plane is much larger than
the near plane.

Technical alternative to measure: linear near/far normalization. Accept it only
if corridor/edge captures show a better priority distribution on both GPUs.

### P3. Already-admitted work — selected KISS trade-off

Priority is guaranteed at scheduler admission. The
fixed 16-page admitted pipeline remains bounded FIFO after admission. A newly
important page preempts a worse pending generation through the existing abort
path, but already-running I/O/transcode/GPU work finishes or is discarded.

Alternative: propagate mutable priority through bulk, transcode, and upload
queues. That requires new handles/indexes and movable queues at every stage. Do
not implement it unless traces show unacceptable center/foreground inversion
under the minimal version.

### P4. Feedback mode — selected

Use one predicted feedback pass in production. When motion history is invalid,
predicted pose equals current pose.

Alternative: current + predicted passes. Rejected for the first implementation
because it doubles feedback scene rendering/readback and requires merge policy.
A temporary diagnostic build may render both during a measurement, but no dual
production path remains.

### P5. Peripheral batching — 64 ms candidate implemented; measurement open

Keep:

- urgent parent deadline: 1 ms;
- high-importance exact deadline: 16 ms.

The implementation currently uses a 64 ms low-importance exact lane and must
still compare 32, 48, and 64 ms non-resetting deadlines. Select the shortest deadline that materially reduces total requests
without making predicted pages publish after the actual camera reaches them.

The implemented bucket-12 boundary is provisional so the candidate can emit
three-lane traces. Select the accepted boundary from recorded score histograms;
do not expose it as a runtime slider.

### P6. Bandwidth and rollout — selected

- Maximum additional source bytes: +5% versus matching no-prediction run.
- Prototype is explicitly enabled only in benchmark builds.
- After RTX 3090, Radeon 680M, and soak gates pass, replace the old binary-center
  scheduler globally and delete the prototype switch.
- Do not retain permanent legacy/predicted scheduler modes.

The user authorized P1–P6 on 2026-07-25. P5 still requires its stated
measurements before 64 ms is promoted from candidate to accepted policy.

## 3. Minimal architecture

```text
small camera predictor
        ↓ predicted Three.Camera
existing one-pass GPU feedback
        ↓ page ID + 3-bit distance; CPU adds center/coverage
one integer page weight
        ↓ fixed priority bucket
existing bounded scheduler
        ↓ urgent / focus / peripheral batch class
existing bounded read → transcode → upload pipeline
```

Mechanism remains separated from policy:

- camera prediction produces a pose;
- feedback produces page observations;
- one pure score helper produces an integer;
- the scheduler consumes a bucket;
- the byte-range queue consumes only a batch class.

The asset provider never learns camera, center, distance, mip semantics, or
material channels.

### 3.1 Burst behavior: 800 pages in one feedback epoch

“Detected,” “scheduled,” and “admitted” are distinct states. A feedback epoch
may detect 800 unique pages, but it can never admit 800 pages into the async
pipeline:

1. Feedback decoding deduplicates the 800 observations.
2. Material-channel/parent expansion writes only into fixed scratch storage. If
   the expanded working set exceeds physical-atlas capacity, the existing
   capacity LOD bias is increased and the set is rebuilt at coarser mips. If it
   still cannot fit at maximum bias, deterministic scheduler-overflow telemetry
   is emitted and omitted pages continue sampling their pinned tail.
3. The persistent scheduler stores at most its bootstrap capacity and orders
   candidates by the perceptual score.
4. One `poll()` admits at most the existing eight-page operation budget and
   0.25 ms scheduling budget.
5. The complete admitted pipeline holds at most 16 pages / 2 MiB. Once full,
   scheduling stops until a generation completes or acknowledges cancellation.
6. Bulk/transcode/upload stages therefore see at most admitted work, never the
   original 800-page burst. Existing transport and upload budgets remain in
   force.
7. The next completed predicted-feedback epoch promotes/demotes retained
   candidates. Requests absent from two completed epochs become stale and are
   removed. Camera motion therefore replaces obsolete backlog instead of
   accumulating it.

There is no promise to refine all 800 pages by the next frame. The pinned tail
and resident parents provide valid coarse sampling while the highest-value
pages refine over later frames. If the camera becomes static, the scheduler
progressively drains the retained set. If it keeps moving, low-value obsolete
pages normally expire before admission.

Any path that calls the page provider for all 800 pages directly is an admission-
boundary bug and must fail a vertical test; it is not an alternate burst mode.

## 4. Code explicitly removed from the earlier plan

Do not implement:

- a public prediction policy object;
- arbitrary ring counts or weight sliders;
- dual production feedback passes;
- game-authored future-pose/teleport hints;
- `provider.reprioritize()`;
- mutable page-pipeline tickets;
- intrusive movable bulk jobs;
- priority-aware transcode workers;
- priority-aware ready-upload queues;
- priority-aware atlas partitions;
- a general priority framework for unrelated systems;
- adaptive/self-tuning weights;
- per-page prediction outcome trace events;
- a permanent fallback scheduler.

These remain future work only if measured evidence demonstrates a specific
failure that the minimal design cannot solve.

## 5. Feedback encoding

### 5.1 Existing layout

Feedback is `RG32Uint`. Word one remains the full texture ID. Word zero currently
uses:

```text
bit 31       valid
bits 17–27   page y
bits 6–16    page x
bits 0–5     desired mip
```

Bits 28–30 are free.

### 5.2 New layout

Store `qd` in bits 28–30:

```text
bit 31       valid
bits 28–30   camera-closeness qd
bits 17–27   page y
bits 6–16    page x
bits 0–5     desired mip
```

This changes neither target format, target size, readback bytes, texture ID,
page coordinates, nor maximum supported VT dimensions.

### 5.3 Shader input

Extend `vtFeedback()` with:

```text
viewDistance
cameraNear
cameraFar
```

Material nodes pass:

- `positionView.length()`;
- Three TSL `cameraNear`;
- Three TSL `cameraFar`.

The predicted camera supplied to `renderer.render()` automatically controls
those values. Both ordinary VT and POM feedback material factories use the same
WGSL function. No per-frame material uniform update or game callback is added.

Validate finite positive near/far in WGSL defensively:

```text
safeNear = max(cameraNear, epsilon)
safeFar = max(cameraFar, safeNear + epsilon)
```

Clamp `qd` to 0–7 before packing.

## 6. Predicted-center contribution

The feedback decoder already knows the feedback pixel coordinate and calculates
`squared radius × 128` as `screenPriority` from 0 to 255.

Use eight equal-area bands:

```text
qf = 7 - min(7, screenPriority >> 5)
```

Because `screenPriority` is proportional to squared radius, equal score ranges
are approximately equal-area screen regions. No square root, trigonometry,
per-page screen projection, or extra GPU field is required.

The center is measured in the predicted view, so this is automatically the
expected center 100 ms ahead.

## 7. Per-page feedback accumulation

Extend pooled `VirtualPageRequest` records with one numeric
`perceptualWeight`. Do not create a score object.

When decoding the first pixel for a page:

```text
coverage = 1
perceptualWeight = 1 + qf + qd
```

For each duplicate pixel:

```text
if coverage < 255:
    perceptualWeight += 1 + qf + qd
coverage = min(65535, coverage + 1)
```

Coverage remains capped at 65,535 for diagnostics, while score contribution is
capped at 255 samples. A page covering more than 255 feedback pixels is already
unambiguously important; further accumulation cannot improve ordering enough to
justify a larger score range.

This accumulation uses the actual foveation and distance of each pixel. It does
not combine an unrelated closest pixel with an unrelated center pixel.

## 8. Resident-quality contribution

During material-channel expansion, resolve the nearest currently resident
fallback for each requested channel using the same bounded mip walk as the
shader. Compute:

```text
qg = min(7, residentMip - desiredMip)
Wpage = perceptualWeight + min(coverage, 255) × qg
```

A pinned tail is treated as the coarsest fallback. Each linked PBR channel gets
its own `qg` because channel residency is independent. Existing albedo/normal/
mask mip biases remain unchanged.

This replaces the current priority based on absolute `qualityDepth`. A page
already close to desired quality should not outrank a page showing a much
coarser fallback merely because its texture has more total mip levels.

The fallback walk is bounded by the existing maximum mip count. It does not add
a map or scan atlas occupancy.

## 9. Fixed importance buckets

The exact known-good system sorts weights. Afterglow must remain allocation-free
and avoid frame-time sorting.

With score coverage capped at 255:

```text
maximum Wpage = 255 × (1 + 7 + 7 + 7) = 5,610
```

Quantize weight into two fixed buckets per power-of-two interval:

```text
exponent = floor(log2(max(Wpage, 1)))
upperHalf = exponent == 0 ? 0 : second-highest bit of Wpage
importanceLevel = exponent × 2 + upperHalf   // 0..24
importanceBucket = 24 - importanceLevel      // 0 is highest
```

Use `Math.clz32` and integer shifts; do not call floating-point `log2` on the hot
path. Unit tests prove monotonicity for every integer from 1 through 5,610.

Compose scheduler lanes:

```text
importanceBucket (25) → parent/exact (2) → channel (3)
```

Total: 150 fixed lanes, close to the existing 132. Pinned bootstrap work remains
outside normal competition. Inside one lane, preserve FIFO.

This one lane number replaces separate radial rings, distance queues,
coverage classes, mip-depth classes, and center bits.

## 10. Camera predictor

### 10.1 Ownership

`VirtualTextureFeedbackCoordinator` owns one fixed predictor record and one
bootstrap-cloned camera per registered feedback camera. Bindings continue to
provide only their current scene/camera/material hooks.

Keep the predictor implementation in one small internal module so pose math is
unit-testable. Do not expose it as a general game camera API.

### 10.2 State

Reserve at registration/warm-up:

- previous/current world position;
- previous/current world quaternion;
- previous/current monotonic timestamp;
- predicted position/quaternion scratch;
- one cloned camera.

No sample arrays, Maps, closures, or gameplay allocations.

### 10.3 Extrapolation

Sample the current feedback camera every frame before the cadence check.

Translation:

```text
velocity = (currentPosition - previousPosition) / dt
predictedPosition = currentPosition + velocity × 0.100 s
```

Rotation uses the shortest quaternion delta over `dt`, extrapolated to 100 ms.
Do not extrapolate Euler angles.

Copy projection, layers, near/far, zoom/FOV, and relevant camera flags to the
clone at feedback submission. Predict world pose only.

### 10.4 Reset behavior

Use current pose without extrapolation when:

- no prior sample exists;
- `dt` is non-positive, non-finite, or indicates browser suspension;
- translation jumps by a substantial fraction of camera far distance in one
  frame;
- angular change is teleport-like;
- camera type or projection becomes invalid.

Exact reset thresholds are unit-tested technical constants, not game sliders.
Expose only a predictor reset counter.

### 10.5 One feedback pass

Submit the existing feedback scene once with the predicted clone. Static camera
behavior is equivalent to current-view feedback. No current/predicted merge,
second render target, or second readback exists in production.

## 11. Reprioritization and staleness

### 11.1 Scheduled pages

For pages observed in the latest predicted epoch, update the 150-lane position
in both directions. Existing requests can improve or worsen.

For pages absent from the latest epoch, avoid a full scheduler scan. Store their
last-seen epoch. When an old request reaches a lane head, lazily move it to the
lowest normal bucket before considering admission. Existing two-epoch staleness
then removes it.

A lower-importance page cannot age across its importance bucket. Remove the old
cross-quality aging rule.

### 11.2 Pending pages

Piggyback on the existing fixed scan of at most 16 pending records:

- update seen records to the latest priority;
- demote unseen records to the lowest normal priority immediately;
- let a newly important page abort the strictly worst pending generation when
  capacity is full.

Do not immediately reuse an aborted pending slot. Existing async ownership
releases it when the downstream stage acknowledges cancellation.

### 11.3 Downstream work

Bulk wait, transcode wait, and ready upload remain FIFO within their existing
bounded capacities. Signals still cancel at existing stage boundaries. A stale
completion is generation-checked before publication.

Telemetry must report any center/foreground wait caused by already-admitted
work. If that bounded inversion fails acceptance, return to the user before
adding mutable downstream priority.

## 12. Minimal peripheral batching

Extend only the bulk queue's fixed lane arrays from two to three:

```text
urgent parent
focus exact
peripheral exact
```

No lane movement after admission. No page-key index. No new provider method.

Properties remain:

- first arrival opens a non-resetting timer;
- ready urgent wins, then ready focus, then ready peripheral;
- at most 256 spans;
- at most 4 MiB estimated response;
- at most two / 8 MiB responses in flight;
- canceled slots are skipped at pump;
- source container and result order remain exact.

All parent restoration retains the existing 1 ms lane in the minimal version.
Exact pages above the measured importance threshold use 16 ms; the rest use the
selected 32/48/64 ms peripheral deadline.

This is the only request-count mechanism added. Priority by itself is not
credited with reducing bulk requests.

## 13. Public/internal surface

Minimal bootstrap changes:

```ts
new VirtualTextureFeedbackCoordinator(renderer, store, {
  renderables: 1,
  passes: 1,
  cadenceMs: 55,
  scale: 0.125,
  predictionHorizonMs: 100,
});

await EngineAssets.open({
  // existing options...
  urgentBatchDeadlineMs: 1,
  focusBatchDeadlineMs: 16,
  peripheralBatchDeadlineMs: 64, // provisional candidate
});
```

The horizon is explicit because it is product policy. Score weights, score cap,
bucket count, and distance quantization are fixed implementation constants, not
public tuning knobs.

Rename internal `quality` batch terminology to `focus` when the third lane
lands. Update all callers in one clean break; do not retain aliases.

## 14. Minimal telemetry

Reuse existing correlated events:

- `vt.feedback_detected` records final lane priority;
- `vt.scheduler_wait` records the same lane and outcome;
- `vt.page_load` retains page correlation and status;
- `vt.bulk_wait` extends tier argument from 0/1 to 0/1/2;
- downstream trace descriptors remain unchanged.

Add only stable aggregate counters/stats needed for acceptance:

- predictor resets;
- pages admitted by urgent/focus/peripheral class;
- peripheral batches/pages/bytes/cancellations;
- maximum pending priority inversion wait;
- final score-bucket histogram for armed diagnostics.

Do not add one prediction event per pixel/page beyond existing detection spans.
Update AGTB/profile decoding for tier 2 and score-lane interpretation.

## 15. Implementation phases

### Phase 0 — lock decisions and baseline

1. Accept or override P1–P6.
2. Keep committed RTX traces immutable.
3. Capture matching Radeon 680M static corridor, moving traverse, reversal, and
   teleport baselines.
4. Define two deterministic static views:
   - deep corridor centered with camera-close edge surfaces;
   - close center with distant edges.

Exit: baseline images, traces, request counts, bytes, and latency are recorded.

### Phase 1 — perceptual score on current camera

1. Add 3-bit feedback distance encoding.
2. Add per-pixel `qf`/`qd` accumulation.
3. Add per-channel resident-gap `qg`.
4. Add the pure weight-to-bucket helper.
5. Replace 132 lanes with 150 importance/kind/channel lanes.
6. Keep the current camera and existing two bulk deadlines.

This isolates score quality from prediction and batching.

Exit:

- close edge can outrank deep center in the intended static case;
- close center still outranks distant edges in the inverse case;
- no frame, byte, failure, allocation, or publication regression.

### Phase 2 — one 100 ms predicted camera

1. Add fixed pose history and camera clone.
2. Submit the existing pass using the predicted camera.
3. Add reset counter and tests.
4. Update priority in both directions each completed epoch.
5. Apply lazy absent-page demotion and fixed pending demotion.

Exit:

- steady movement loads pages ahead of the actual view;
- static view matches Phase 1;
- reversal/stop behavior remains bounded;
- unannounced teleport is no worse than baseline.

### Phase 3 — one peripheral batch lane

1. Extend bulk queue arrays/timers/stats from two lanes to three.
2. Add fixed exact-page importance threshold.
3. Run 32, 48, and 64 ms candidates.
4. Select one candidate or reject the third lane.

Exit: total requests fall materially without center/foreground publication or
source-byte regression.

### Phase 4 — target gates and cleanup

1. Run RTX 3090 and Radeon 680M acceptance scenarios.
2. Run 30-minute traverse and 60-minute thrash/teleport soaks.
3. Delete binary-center priority, old quality-depth lanes, prototype switches,
   and superseded tests.
4. Update API docs, book, capacity tables, telemetry docs, and evidence.

## 16. File-level implementation map

Expected authored changes:

- `virtual-texture-feedback.ts`
  - shared bit masks and distance encode/decode test helpers;
- `virtual-texture.ts`
  - request weight, resident-gap score, 150 lanes, demotion semantics, WGSL
    packing;
- `virtual-texture-material.ts`
  - pass view distance and camera near/far for ordinary and POM feedback;
- `virtual-texture-feedback-pass.ts`
  - decode `qd`, calculate `qf`, accumulate bounded weight;
- `virtual-texture-feedback-coordinator.ts`
  - fixed pose history, camera clone, 100 ms predicted submission;
- one small internal predictor module and colocated test;
- `deadline-range-batcher.ts` and `vt-page-provider.ts`
  - third timer/lane and stats only;
- `engine-assets.ts`
  - one peripheral deadline option;
- profile/replay scripts
  - tier-2 and score-bucket decoding;
- focused tests and synchronized docs/book.

No cook, BIG format, worker RPC, atlas layout, page-table format, shader sampling,
or native/public-web transport framing change is required.

## 17. Test plan

### Score and encoding

- all packed mip/x/y/texture identities survive distance bits;
- `qd` round-trips 0–7;
- malformed/edge near/far values clamp deterministically;
- center/corner `qf` boundaries;
- duplicate pixels accumulate their own combined values;
- score coverage stops at 255 while diagnostic coverage continues;
- resident-gap clamp and channel independence;
- bucket monotonicity for every weight 1–5,610;
- exact lane order: importance, parent/exact, channel, FIFO.

### Predictor

- static pose;
- constant translation;
- quaternion rotation across angle wrap;
- combined movement;
- invalid/suspended `dt` reset;
- teleport-like translation/rotation reset;
- projection/layers copied;
- source camera never mutated;
- perspective and orthographic cameras or explicit bootstrap rejection.

### Scheduler/pipeline

- inward and outward score changes move scheduled pages;
- unseen head lazily demotes before admission;
- pending scan demotes at most 16 records;
- high score preempts strictly worse pending work;
- aborted slot is not reused early;
- three bulk timers are independent and non-resetting;
- ready-lane order is urgent, focus, peripheral;
- capacities, close, errors, and cancellation release every slot;
- final queues drain to zero.

### Allocation/complexity

- no sort, heap, growing collection, per-frame object, Promise, closure, or new
  typed-array view in predictor/feedback/scheduler hot regions;
- one bounded page-table fallback walk;
- fixed queue operations and score calculation pass allocation lint;
- long soak heap, timers, queues, and pending work plateau.

## 18. Acceptance gates

### Correctness

- Zero failed pages, queue overflow, trace drops/unmatched spans, stale
  publication, GPU errors, and post-seal pipelines.
- Final scheduled/pending/bulk/transcode/ready counts are zero.
- Expected camera pose, feedback distance, page score, scheduler lane, load, and
  publication retain one traceable page identity.

### Visual priority

- In the deep-corridor test, camera-close edge pages publish before distant
  center pages when their combined known-good score is higher.
- In the close-center inverse test, center pages remain first.
- Steady movement shows no visible prediction lag relative to baseline.
- Stop/reversal does not leave a stable incorrect quality region.

### Performance

- No new >60 Hz frame miss beyond baseline variance on RTX 3090 or Radeon 680M.
- Feedback GPU/CPU cost remains within measured run variance or is explicitly
  accepted.
- Center/foreground admitted page p99 does not regress from the matching
  no-prediction baseline.
- Source bytes stay within +5%.
- Peripheral batching is accepted only if it lowers total requests against the
  same score/prediction run with a 16 ms peripheral lane.
- Unannounced teleport is non-regressed; motion prediction cannot know its
  destination.

### KISS rejection gate

If the minimal admitted FIFO causes unacceptable foreground inversion, stop and
report the exact stage/time distribution. Do not automatically implement
end-to-end mutable priority. That becomes a separate user decision backed by the
failure trace.

## 19. Documentation and commit sequence

Required synchronized documents after behavior changes:

- `docs/api/virtual-texturing.md`;
- `docs/api/asset-system.md`;
- `docs/api/runtime-capacities.md`;
- `docs/api/telemetry.md`;
- `docs/api/testing.md`;
- `book/src/window/virtual-texturing.md`;
- allocation-effect contracts and benchmark evidence.

Proposed commits:

1. `test(vt): add perceptual priority fixtures`
2. `feat(vt): score perceptual feedback demand`
3. `feat(vt): predict feedback camera pose`
4. `feat(assets): batch peripheral VT demand`
5. `test(vt): gate predicted perceptual streaming`
6. `docs(vt): publish perceptual scheduling results`

Core implementation is present; release promotion remains blocked on focused
artifact/allocation gates, the 32/48/64 ms comparison, both target GPUs, and the
required soaks.
