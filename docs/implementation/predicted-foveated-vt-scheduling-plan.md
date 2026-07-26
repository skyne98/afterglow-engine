# Predicted-foveated virtual-texture scheduling plan

Status: **design only; blocked on the product decisions in section 2**.

This plan changes virtual-texture demand from a mostly current-view,
coarse/center binary priority into a strict radial priority around the camera
view expected 100 ms in the future. It also propagates that priority through
all queued pipeline stages and gives peripheral work a longer batching window.
No implementation is authorized by this document alone.

## 1. Product intent

For every completed feedback epoch:

1. estimate the camera pose 100 ms in the future;
2. identify the pages visible from that expected pose;
3. assign the expected center the highest non-pinned priority;
4. assign monotonically lower priorities to concentric radial rings;
5. immediately demote queued work that moved away from the expected center;
6. let newly important center work preempt strictly worse queued work;
7. batch peripheral work longer so prediction can reduce transport request
   count instead of merely changing which page is requested first.

The selected policy is **predicted foveation**, not a new persistent cache and
not CPU traversal of scene geometry. CPU code predicts one camera pose; the
existing GPU feedback path determines visibility, materials, UVs, displacement,
occlusion, and desired mip for that pose.

### 1.1 Required invariants

- Pinned tails and bootstrap safety pages remain non-evictable and cannot be
  preempted.
- Expected-center ordering is deterministic and fixed-capacity.
- No lower radial ring may age past a higher radial ring.
- Priority changes affect every queued stage: scheduler, bulk wait, transcode
  wait, and ready-upload wait.
- Already-running browser I/O, native I/O, worker transcoding, and GPU writes are
  non-preemptible. Their stale results may be discarded, but the engine must not
  claim instantaneous cancellation.
- Existing 16-page / 2 MiB admitted bounds, two / 8 MiB bulk-response bounds,
  worker count, and atlas capacity do not increase without a separate decision.
- Prediction adds no gameplay-sealed general-purpose allocation.
- Public web and native hosts use identical scheduling/framing policy. This work
  does not legitimize the known CEF WASM texture-worker violation.
- Visual demos configure public engine APIs only; they do not own prediction,
  priority queues, page-provider internals, or worker lifecycle.

## 2. Decisions required before implementation

The user has already selected:

| Decision | Selected policy |
|---|---|
| Prediction target | Camera view expected **100 ms** in the future |
| Spatial order | Expected center first, then monotonically outward |
| Reprioritization | Pages moving away from the expected center are demoted immediately |
| Center behavior | Newly important center pages become priority number one among non-pinned work |

The following consequential policies remain unresolved.

### P1. Current-view safety versus strict future-center priority — blocking

**Recommended default:** render current and predicted feedback during the first
prototype, but make predicted radial rank primary. A current-only page remains
eligible at the lowest radial rank so prediction failure cannot permanently
hide an actually visible miss. It does not outrank predicted-center work.

Alternatives:

- **Strict predicted-only production pass:** replace the current feedback camera
  with the predicted camera. Lowest GPU cost and most literal interpretation of
  the selected policy, but abrupt reversal can deprioritize what is visible now.
- **Co-equal current center:** current-center and predicted-center pages both
  occupy ring zero. Safest visually, but weakens the requirement that the
  expected future center always wins and may increase request count.

A diagnostic current pass is still needed while evaluating predicted-only mode
so prediction hit/miss rates can be measured rather than inferred.

### P2. Ordering inside one radial ring — blocking

**Recommended default:**

1. missing coarse-restoration parent;
2. exact page;
3. albedo before normal/emissive before scalar masks;
4. larger desired-to-resident mip deficit;
5. larger coverage;
6. stable FIFO order.

Radial ring is compared before every item above. Therefore a ring-zero exact
page outranks a ring-one coarse parent. This is the key departure from the
current global “all urgent parents before all exact pages” policy.

Alternative: channel-first ordering can place exact albedo before coarse normal
and mask parents in the same ring. That may improve perceived center color at
the cost of temporarily mismatched material channels.

### P3. Peripheral batching deadline and bandwidth allowance — blocking

**Recommended default for the measured prototype:** test 32, 48, and 64 ms
non-resetting peripheral deadlines; select the shortest value that materially
reduces total requests while keeping predicted-center publication ahead of the
actual view. Keep the existing 1 ms parent and 16 ms focus/exact deadlines.
Permit at most **5% additional source bytes** versus the matching no-prediction
run.

Alternatives:

- 16 ms for every ring: lowest latency but the previous replay proves that
  priority changes alone do not reduce request count.
- 100 ms peripheral batching: strongest coalescing, but the full
  read/transcode/publication pipeline is unlikely to fit inside a 100 ms
  prediction horizon on the measured p99 path.
- Drop far rings instead of reading them: lowest bandwidth, visible peripheral
  quality degradation during stops and reversals.

The deadline is a product latency/bandwidth choice; 32/48/64 selection is a
technical benchmark question.

### P4. Prediction misses and current-only work — blocking

**Recommended default:** a page absent from the latest predicted view is moved
to the farthest ring immediately, not canceled immediately. Existing two-epoch
staleness then cancels it unless actual/current feedback still reports it.
Higher-priority center work may preempt it before that horizon.

Alternative: cancel on the first prediction miss. This follows camera turns
more aggressively but can repeatedly discard useful work under controller
jitter or oscillation.

### P5. Scripted teleports and game hints — non-blocking for phase 1

**Recommended default:** do not add game-authored future-pose hints in phase 1.
Velocity prediction cannot improve an unannounced teleport, so the existing
hostile-teleport request exception remains a non-regression gate rather than a
claimed success case.

Alternative: later expose one bounded future-view hint for doors, rails,
cutscenes, and teleports. That is a public gameplay contract and must be decided
separately; it must not become mandatory for ordinary camera movement.

### P6. Production feedback mode — technical gate

Prototype both:

- one predicted feedback pass replacing the current-view pass; and
- current plus predicted passes merged with explicit origin metadata.

**Recommended production default if measurements pass:** one predicted pass,
because it does not double feedback scene rendering/readback. Keep dual-view
mode as an armed diagnostic, not a permanent gameplay cost. Promote dual-view
to production only if reversal tests show visible current-view regressions.

### P7. Radial resolution — technical gate

**Recommended default:** 16 rings derived from the existing 0–255 squared radial
screen score (`ring = screenPriority >>> 4`). Equal intervals in squared radius
are approximately equal-area screen bands and require no square root.

Compare against eight rings only if 16-ring queue/telemetry cost is measurable.
Do not expose arbitrary runtime ring counts; choose one fixed implementation
constant after the prototype.

### P8. Rollout and compatibility — blocking before production promotion

**Recommended default:** prediction is explicitly enabled only in the profiling
prototype. After both target GPUs and soaks pass, replace the binary-center
scheduler globally for VT sessions and remove the prototype switch. Do not keep
predicted and legacy production schedulers as permanent compatibility paths.

Alternative: permanent per-game opt-in. This preserves historical ordering but
creates two engine policies, doubles long-term test surface, and conflicts with
the project's legacy-removal rule.

## 3. Current implementation and why priority alone is insufficient

Current code:

- renders feedback at 1/8 display scale every 55 ms;
- calculates `screenPriority` from 0 at screen center to 255 at corners;
- compresses that score into one `centralOrLarge` bit;
- globally prioritizes all coarse-restoration parents before all exact pages;
- has 132 fixed scheduler lanes;
- allows pending priority only to improve (`min`), not worsen;
- keeps absent requests for two feedback epochs;
- admits at most eight pages per poll and sixteen total pages / 2 MiB;
- batches admitted reads through non-resetting 1/16 ms lanes;
- dispatches transcode and ready-upload work FIFO;
- cannot change a bulk/transcode queue position after admission.

The committed RTX replay reproduced 156 hostile bulk requests. Reversing mip
priority and adding channel affinity still produced 156. Source sorting reduced
adjacent source runs but not request count. Therefore this plan must change both
**which work advances** and **how peripheral work is batched**. Replacing the
binary center bit with radial rings without changing downstream queues and
batch timing cannot satisfy the request-count objective.

## 4. Ownership and API boundaries

### 4.1 Feedback coordinator owns pose prediction

`VirtualTextureFeedbackCoordinator` owns:

- fixed pose-history records per registered feedback renderable;
- bootstrap-created predicted camera clones;
- velocity estimation, reset detection, and 100 ms extrapolation;
- selection of current/predicted/diagnostic feedback cameras;
- complete-snapshot publication and prediction telemetry.

Bindings continue to provide a scene, current camera, and material swap hooks.
They do not implement prediction.

### 4.2 VirtualTextureStore owns page policy

`VirtualTextureStore` owns:

- radial rank construction after material-channel expansion;
- deterministic ring-first scheduler order;
- promotion, demotion, preemption, staleness, and publication validity;
- predicted/current origin bookkeeping;
- fixed stage priority passed to the provider;
- hit/miss/waste accounting.

It does not know BIG offsets, HTTP, CEF/native transport, or worker APIs.

### 4.3 Page provider owns stage ordering and batching

The generic bounded page provider owns:

- non-resetting transport deadlines;
- O(1) movement of queued work between priority classes;
- strict queued transcode ordering;
- cancellation at stage boundaries;
- two-response / 8 MiB transport bounds;
- stage telemetry.

Texture semantics remain in the VT consumer. The underlying bulk-byte queue
stays a generic fixed-capacity byte-range mechanism.

### 4.4 Proposed bootstrap configuration

The exact names are provisional, but the ownership should be:

```ts
interface VirtualTexturePredictionConfig {
  horizonMs: 100;
  currentViewPolicy: 'lowest-ring-fallback' | 'predicted-only' | 'coequal-center';
  peripheralBatchDeadlineMs: number;
  maxSourceByteOverheadPercent: number;
}

new VirtualTextureFeedbackCoordinator(renderer, store, {
  renderables: 1,
  passes: 1,
  cadenceMs: 55,
  scale: 0.125,
  prediction: predictionConfig,
});
```

Configuration is validated and copied during bootstrap. Gameplay does not
replace policy objects. If the final production mode uses dual feedback, pass
capacity must account for the doubled targets explicitly.

`BigAssetSessionOptions` gains the selected peripheral deadline only after the
prototype proves a third lane is necessary. It must not acquire camera or VT
policy.

## 5. Camera prediction mechanism

### 5.1 Pose sampling

For each registered renderable, reserve during construction/registration:

- previous/current world position;
- previous/current world quaternion;
- linear and angular velocity scratch;
- last sample timestamp and confidence/reset state;
- one cloned camera of the same Three.js camera subtype;
- matrix/quaternion/vector scratch needed by extrapolation.

Sample the current feedback camera every render frame, before the feedback
cadence check. This gives prediction enough temporal resolution even though GPU
feedback submits every 55 ms.

### 5.2 Translation

Compute world-space displacement over a positive bounded `dt`, convert it to
velocity, and apply a fixed exponential filter. Predict:

```text
predicted_position = current_position + filtered_linear_velocity * 0.100 s
```

The prototype must measure filtered versus unfiltered prediction. Filtering
must not introduce a dynamic sample array.

### 5.3 Rotation

Calculate the shortest quaternion delta from previous to current pose, derive a
bounded angular displacement, then apply that delta for the 100 ms horizon.
Do not extrapolate Euler angles: wraparound, rotation order, and gimbal behavior
would make the result camera-dependent.

### 5.4 Projection and hierarchy

- Copy projection, inverse projection, layers, near/far, zoom/FOV, and camera
  flags from the current camera at submission time.
- Predict world pose only; do not predict FOV/zoom in phase 1.
- The predicted clone has no scene parent. Its world pose is written directly
  and its world/inverse matrices are updated before rendering.
- Perspective and orthographic cameras must either both pass contract tests or
  unsupported camera types must fail at bootstrap. Silent current-view fallback
  by camera subtype is not allowed.

### 5.5 Discontinuity and confidence handling

Reset filtered velocity and use the current pose for one prediction epoch when:

- time delta is zero, negative, non-finite, or above a suspension threshold;
- translation or angular change exceeds the configured discontinuity bound;
- projection/camera subtype changes;
- the browser resumes after background suspension;
- a diagnostic/programmatic camera teleport is detected.

Exact discontinuity bounds are technical benchmark inputs. They must be based
on maximum plausible movement per frame, not hard-coded from the Dungeon demo.
Expose stable reset counters.

## 6. Feedback production

### 6.1 Single predicted-view candidate

The minimal production candidate uses the existing feedback scene/materials and
target count, but submits each pass with the predicted camera clone. Existing
pixel decoding already computes distance from that predicted view's center, so
no CPU page-space projection is needed.

Benefits:

- no extra scene traversal, target, or readback;
- predicted POM, visibility, UV addressing, mip derivatives, and occlusion stay
  shader-correct;
- static/low-confidence cameras naturally reduce to current-view behavior.

Risk: pages visible now but absent from the predicted frustum are represented
only by retained scheduler/residency state.

### 6.2 Dual-view diagnostic/candidate

Reserve separate current and predicted targets. Submit both atomically for every
active local material pass. Do not merge a partial pair. Each decoded request
carries a numeric origin (`current` or `predicted`) without creating per-request
objects.

The store uses current feedback for:

- actual-visibility/prediction-hit measurement;
- current-only fallback eligibility according to P1;
- residency touches and stale-work correctness.

It uses predicted feedback for primary radial rank.

### 6.3 Capacity fitting

Predicted demand must not force actual visible demand to a globally coarser mip
merely because two views were unioned.

For dual mode:

1. build and capacity-fit current demand first;
2. insert predicted pages with predicted radial rank;
3. deduplicate pages shared by both views;
4. when scratch capacity is exhausted, reject the farthest predicted rings
   before increasing current-view LOD bias;
5. retain the existing per-channel `0/+1/+2` bias policy.

Selection must use fixed radial buckets or bounded replacement. It may not sort
Maps or scan an occupancy-growing list each frame.

## 7. Priority representation

### 7.1 Ring-first key

Use a fixed packed integer or hierarchical bucket index with this logical order:

```text
pinned/bootstrap
radial_ring                  0 .. 15
restoration_class            parent before exact
channel_class                albedo, normal/emissive, scalar
mip_deficit                  larger deficit first
coverage_class               larger coverage first
stable FIFO sequence
```

The representation must provide enough values without unsafe arithmetic and
must be documented as telemetry ABI. `screenPriority` remains the raw diagnostic
score; `radialRing` is internal numeric state.

### 7.2 No cross-ring aging

Aging may reorder requests only inside the same radial ring and channel/detail
floor. It cannot move ring 1 into ring 0. This enforces “center is always number
one” even under sustained peripheral waiting.

### 7.3 Fixed queues

Replace linear “find first nonempty lane” scans with fixed ring/lane occupancy
bitsets or a fixed minimum-nonempty index. Intrusive arrays retain head, tail,
previous, and next slot indices. Required operations:

- insert tail;
- remove;
- move to another lane;
- inspect highest priority;
- pop highest priority;
- mark empty/nonempty.

All are allocation-free and bounded. Add focused tests before integrating the
queue into VT. Keep it local unless another engine subsystem has an immediate
use; do not publish speculative generic infrastructure.

## 8. Reprioritization semantics

### 8.1 Scheduled requests

On every completed prediction epoch:

- pages still predicted receive their new exact ring;
- pages that move inward are promoted immediately;
- pages that move outward are demoted immediately;
- pages absent from prediction move to ring 15 under the recommended P4 policy;
- current-only pages use P1's selected fallback ring;
- stale epoch remains independent from radial rank.

Avoid a full active-scheduler scan. Update seen pages directly. When an unseen
request reaches a queue head, compare its epoch stamp and lazily move/remove it
before admission. A fixed previous-epoch key list may be used only if its
capacity and operation budget are explicit.

### 8.2 Pending/admitted requests

Current pending records only permit priority improvement. Change them to store
the priority selected by the latest complete epoch in both directions.
`preemptWorstPending` compares the complete ring-first key.

Demotion does not imply cancellation. Cancellation occurs when:

- a higher-priority request needs capacity;
- the selected stale horizon expires;
- the provider/session closes;
- a generation becomes invalid.

### 8.3 Non-preemptible stages

The following finish once started:

- an already-dispatched HTTP/native bulk read;
- one active worker transcode;
- one submitted GPU write.

At completion, generation and latest-priority/visibility state are checked
before publication. Obsolete peripheral results are discarded deterministically
rather than touching the atlas and page table.

Track “priority inversion due to non-preemptible stage” duration. Center-first
means no queued lower-priority work starts first; it does not mean running work
is forcibly interrupted.

## 9. End-to-end pipeline priority

Changing only `VirtualTextureStore` is insufficient. Priority must follow the
page key/correlation through each stage.

### 9.1 Provider contract

Extend the internal provider contract with a numeric pipeline priority and a
bounded reprioritization operation keyed by stable packed page identity. Exact
shape is selected during implementation, but it must support:

```ts
provider(path, page, signal) -> Promise<bytes>
provider.reprioritize(pageKey, pipelinePriority) -> status
provider.getStats()
provider.close()
```

Statuses distinguish not found, queued-and-moved, already active, and complete.
No silent success.

An alternative mutable priority cell attached to the stable pending record may
be prototyped, but queue timers still need an explicit wakeup when work moves
into a faster lane.

### 9.2 Bulk queue

Generalize `BoundedBulkReadQueue` from two ring queues to three:

1. urgent parent: 1 ms;
2. focus exact: 16 ms;
3. peripheral predicted: selected 32/48/64 ms candidate.

Replace circular lane storage with fixed intrusive slot links so a queued slot
can move lanes in O(1). Maintain a fixed page-key-to-slot index. Promotion:

- removes the slot from peripheral;
- inserts it into the faster lane;
- uses the earlier of its already-earned deadline and the new lane deadline;
- wakes the pump if immediately ready.

Demotion starts the peripheral batching interval at demotion time so it can
actually coalesce with later peripheral arrivals. It never resets another
request's open deadline.

Dispatch remains strict among **ready** lanes and retains:

- at most 256 spans;
- at most 4 MiB estimated response;
- at most two / 8 MiB responses in flight;
- no mixing of source containers;
- exact caller result association;
- cancellation checks before dispatch and after response.

Source sorting/adjacent merge is an independent mechanism and must not be
credited as request-count reduction unless wired and measured.

### 9.3 Transcode queue

Replace FIFO waiting jobs with fixed priority buckets carrying the latest
pipeline priority. Reprioritization moves only waiting jobs. Active workers stay
non-preemptible. Aborted jobs are removed before dispatch, and center jobs
always win the next free worker.

Preserve one in-flight call per SPSC worker and the existing fixed waiting
capacity. Do not add workers or queue slots in this change.

### 9.4 Ready-upload queue

Replace FIFO ready-upload selection with the same ring-first queued priority.
Before selecting an upload:

- validate generation;
- validate latest predicted/current eligibility;
- discard stale peripheral completion;
- choose highest-priority ready page;
- respect existing adaptive count/time upload budget.

A late peripheral transcode must not publish before an already-ready center
page merely because it entered the FIFO first.

### 9.5 Atlas behavior

Keep the O(1) lookup and second-chance clock in phase 1. Current/predicted
feedback may touch resident pages according to P1, but radial priority does not
create separate atlas partitions or pin center pages. If measurements show
peripheral prefetch evicting center pages, treat priority-aware residency as a
new policy decision rather than silently modifying the clock.

## 10. Batching policy and request-count mechanism

Priority determines order; deadlines determine bulk request count.

Recommended initial mapping:

| Work | Scheduler order | Bulk lane |
|---|---:|---:|
| Pinned/required tail | absolute | urgent 1 ms |
| Ring 0 missing parent | highest non-pinned | urgent 1 ms |
| Ring 0 exact | next | focus 16 ms |
| Near predicted rings | radial order | peripheral candidate |
| Far predicted/current-only | lowest radial order | peripheral candidate |

The prototype must also test whether the first two radial rings should use the
focus lane. This is a measured quality/request trade-off, not a guessed default.

The 100 ms lead budget is approximately:

```text
prediction lead
  - wait until peripheral deadline
  - bulk I/O
  - transcode queue
  - transcode execution
  - upload/publication
= margin before actual visibility
```

Use trace correlations to calculate this margin per page. A page that publishes
after it becomes currently visible is a late prediction even if it eventually
loads successfully.

## 11. Telemetry and diagnostics

Append descriptors; never renumber the existing AGTB catalog.

Required events/metrics:

- prediction snapshot: frame, horizon, confidence/reset reason;
- raw/filtered linear and angular movement bins;
- predicted candidate page and radial ring;
- current-view confirmation of a predicted page;
- prediction lead from first predicted detection to current detection;
- hit-before-visible, late-hit, and never-visible outcomes;
- promotion and demotion counts by source/destination ring;
- provider reprioritization status;
- peripheral bytes, pages, batches, and cancellations;
- queued priority inversion and non-preemptible inversion duration;
- stale result discarded before upload;
- per-ring scheduled, pending, and ready high-water marks;
- current/predicted feedback GPU duration when dual diagnostics are armed.

`VtFeedbackDetected`, `VtSchedulerWait`, `VtPageLoad`, `VtBulkWait`,
`TextureTranscodeQueue`, `TextureTranscode`, and `VtPagePublished` retain one
numeric page correlation. Pack ring/origin into documented arguments where it
fits; add descriptors where overloading would destroy existing semantics.

Update `scripts/profile-dungeon-vt.ts` and the offline replay so they understand
the appended descriptors and can report:

- total versus focus/peripheral requests;
- pages and bytes per request;
- lead-time percentiles;
- center publication latency;
- request/byte cost of never-used predictions.

A trace that cannot distinguish actual confirmation from predicted demand is
not sufficient acceptance evidence.

## 12. Implementation phases

### Phase 0 — decisions and baseline freeze

1. Resolve P1–P4 with the user.
2. Record selected policies in this plan.
3. Keep the committed no-cache RTX traces immutable.
4. Capture matching Radeon 680M baseline traces before changing behavior.
5. Add a deterministic camera-motion scenario definition: linear movement,
   yaw, stop, reverse, oscillation, and teleport.

Exit: decisions are explicit and both target GPUs have comparable baselines.

### Phase 1 — predictor and diagnostic evidence, no scheduling change

1. Implement fixed camera-pose history and 100 ms prediction.
2. Add current/predicted diagnostic feedback pairing.
3. Add prediction telemetry and profile decoding.
4. Do not feed predicted maps into scheduling yet.
5. Measure 50/100/150 ms diagnostically while keeping 100 ms as the selected
   product target; this only establishes error curves.
6. Measure static, translation, yaw, reversal, collision-stop, and teleport.

Exit:

- predictor allocates nothing after sealing;
- predicted pose tests pass;
- diagnostic snapshots are atomic;
- hit/late/miss and GPU cost are measured;
- no runtime scheduling behavior changed.

### Phase 2 — radial scheduler using existing current feedback

1. Introduce fixed 16-ring priority representation.
2. Replace binary center priority.
3. Make radial ring primary over restoration/channel/mip according to P2.
4. Clamp aging within a ring.
5. Support promotion and demotion of scheduled records.
6. Keep existing current camera and 1/16 ms provider lanes.

This isolates scheduler correctness from prediction and batching.

Exit:

- exact deterministic ordering tests pass;
- no queued lower ring outranks a higher ring;
- allocation lint and bounded-operation tests pass;
- current-view GPU regression has no visual/pipeline failure.

### Phase 3 — predicted feedback drives radial rank

1. Select single or dual production mode from Phase 1 evidence.
2. Feed predicted radial rank into material-channel expansion.
3. Apply P1 current-only behavior.
4. Apply immediate outward demotion and P4 miss handling.
5. Preserve atlas-capacity fitting and per-channel biases.
6. Add prediction outcome accounting.

Exit:

- center follows predicted pose in deterministic camera trajectories;
- turn/reverse/teleport degrade predictably;
- static camera behavior matches current implementation;
- no source/provider behavior changed yet.

### Phase 4 — priority propagation through all queued stages

1. Extend provider priority contract.
2. Replace bulk lane rings with movable fixed intrusive queues.
3. Replace transcode FIFO waiting queue with priority buckets.
4. Replace ready-upload FIFO selection with priority buckets.
5. Propagate promotion/demotion and cancellation statuses.
6. Preserve active-stage non-preemption and generation checks.

Exit:

- a newly predicted center page wins every next queued dispatch opportunity;
- an active lower-priority operation is reported, not falsely preempted;
- queue capacities and memory are unchanged;
- close/cancel/error tests leave every fixed slot reusable.

### Phase 5 — peripheral batching experiment

1. Add the third peripheral bulk lane.
2. Run 32, 48, and 64 ms candidates.
3. Test ring 0 only versus rings 0–1 on the 16 ms focus lane.
4. Measure total request count, source bytes, prediction lead, and center
   publication latency.
5. Reject any candidate that wins by silently dropping unresolved center pages
   or leaving work undrained.
6. Select one deadline only if it passes both GPUs.

Exit: one measured policy is selected, or the third lane is rejected and the
plan records that prediction improves order but not request count.

### Phase 6 — real-GPU acceptance and soaks

Run on RTX 3090 and Radeon 680M:

- static camera;
- steady forward traverse;
- steady strafe;
- constant yaw;
- combined translation/yaw;
- abrupt stop;
- 180-degree reversal;
- alternating/oscillating input;
- collision-constrained movement;
- unannounced teleport;
- POM on/off;
- full-atlas replacement pressure.

Then run the existing 30-minute traverse and 60-minute thrash/teleport soaks.

Exit: section 14 gates pass and heap/queues/timers plateau.

### Phase 7 — documentation and cleanup

Update in the same behavior-changing commits:

- `docs/api/virtual-texturing.md`;
- `docs/api/asset-system.md`;
- `docs/api/runtime-capacities.md`;
- `docs/api/telemetry.md`;
- `docs/api/testing.md`;
- `book/src/window/virtual-texturing.md`;
- allocation-effect contracts;
- benchmark reports and raw hashes;
- this plan's final status.

Delete binary-center/dead code and obsolete tests. Do not retain parallel legacy
schedulers or a hidden fallback policy.

## 13. Test plan

### 13.1 Unit tests

Camera predictor:

- static pose;
- constant translation;
- constant yaw across angle wrap;
- combined translation/rotation;
- zero/non-finite/large `dt` reset;
- suspension and teleport reset;
- projection copy;
- parented current camera to unparented predicted world pose;
- perspective/orthographic contract;
- no source-camera mutation.

Radial priority:

- center and every ring boundary;
- corner clamping;
- ring 0 exact before ring 1 parent if P2 selects radial-first;
- channel/detail/mip/coverage/FIFO ties;
- aging never crosses ring;
- promotion and demotion relink exactly once;
- absent prediction receives P4 behavior;
- linked PBR channel bias remains correct.

Fixed queues:

- insert/remove/move head/middle/tail;
- empty/nonempty bit maintenance;
- wrap/reuse every slot;
- cancellation while timer open;
- promotion to an already-ready lane;
- demotion does not reset other jobs' deadline;
- strict ready-lane order;
- close releases all slots once.

Pipeline:

- bulk promotion/demotion;
- transcode waiting priority;
- active transcode non-preemption;
- ready-upload priority;
- stale peripheral result discarded;
- center preempts worst pending generation;
- no duplicate page generation after promotion;
- all rejection paths close telemetry spans.

### 13.2 Vertical integration tests

- Camera movement → predicted feedback → radial scheduler → provider lane →
  transcode → upload → page-table publication with one page correlation.
- Current/predicted duplicate page merges once.
- Prediction reversal demotes queued work and promotes the new center.
- Peripheral work cannot consume a dispatch opportunity while ready center work
  is queued.
- Transport still obeys 256-span, 4 MiB response, two-response, and 8 MiB bounds.
- Session close during each stage drains all timers/promises/slots.
- Public web and native provider adapters return identical ordering/status
  semantics.

### 13.3 Allocation and complexity tests

- Run custom hot-path lint over predictor, feedback merge, scheduler, and queue
  operations.
- Assert no Promise, closure, array/object literal, growing Map/Set, sort,
  splice/shift, or new typed-array view in sealed authored hot paths.
- Prove fixed queue operations remain bounded at full capacity.
- Soak telemetry must show stable heap floor and fixed queue high-water marks.

### 13.4 Artifact and documentation tests

```sh
bun scripts/build-web.ts
bun scripts/build-web.ts --check
cargo run -p xtask -- conformance
cargo run -p xtask -- test
cd book && nix-shell -p mdbook mdbook-mermaid --run 'mdbook build'
```

Run focused Bun tests during each phase before the full lanes.

## 14. Acceptance gates

The exact prediction-quality thresholds require P1–P4 decisions and Phase 1
baseline evidence. The following gates are non-negotiable:

### Correctness

- Predicted center is the highest queued non-pinned radial priority.
- No queued lower-priority work dispatches while higher-priority ready work can
  use that same stage.
- No duplicate page generation, stale page-table publication, or invalid atlas
  ownership.
- Zero failed pages, queue overflow, trace drops, unmatched spans, GPU errors,
  and post-seal pipelines.
- All final scheduler/pending/bulk/transcode/ready counts drain to zero.

### Performance

- No new frame slower than 60 Hz in the accepted RTX and 680M scenarios beyond
  baseline run variance.
- Existing upload count/time budgets remain unchanged.
- Total source bytes remain within the user-selected overhead cap (recommended
  +5%).
- Center publication latency must not regress against no-prediction baseline.
- Peripheral deadline is accepted only if total bulk requests fall materially
  relative to the matching predicted-priority run with a 16 ms peripheral lane.
- Diagnostic dual feedback is production-eligible only if its GPU/frame cost
  passes both GPUs.

### Prediction quality

Report, do not hide:

- hit-before-visible ratio;
- late-hit ratio and lead p50/p95/p99;
- never-visible pages and bytes;
- center-ring request/publication latency;
- reversal/oscillation waste;
- non-preemptible priority inversion p95/p99/max.

No numerical hit-rate target is made engine policy until Phase 1 measures the
actual scenes. The recommended initial product guard is at most 5% additional
source bytes and no center-latency regression.

### Teleport interpretation

An unannounced teleport is a non-regression case. Camera-motion prediction
cannot know its destination. It may not be cited as proof that prediction
failed to optimize normal traversal, and normal traversal may not be cited as
proof that the old 106-request hostile-teleport gate was met. A future-pose game
hint would require a separate measured gate.

## 15. Failure policy

- Invalid prediction state: reset velocity and submit current pose; increment a
  stable reset counter.
- Predicted feedback readback failure: discard the complete prediction snapshot;
  retain prior scheduler/residency state; do not publish a partial epoch.
- Prediction capacity overflow: drop farthest predicted rings first; never grow
  storage or coarsen actual current demand silently.
- Priority queue overflow: deterministic rejection counter and existing coarse/
  tail fallback; never allocate overflow storage.
- Center request blocked by active peripheral work: report bounded
  non-preemptible inversion; it receives the next available queued slot.
- Source/transcode failure: preserve existing fatal page failure diagnostics;
  prediction does not add an alternate codec or transport.
- Repeated poor prediction: telemetry can justify disabling the feature in a
  later policy decision, but runtime self-disabling is not part of phase 1.

## 16. Rollback boundaries

Each phase must be independently revertible:

1. predictor/diagnostic telemetry;
2. radial scheduler;
3. predicted scheduling;
4. downstream priority propagation;
5. peripheral batching;
6. selected production policy.

Do not keep two production schedulers behind a permanent flag. Diagnostic
current/predicted comparison may remain only as bounded profiling machinery.
If peripheral batching fails, retain predicted radial ordering only if it
independently improves center latency/quality without byte or frame regression.

## 17. Proposed semantic commit sequence

1. `test(vt): add camera prediction evidence harness`
2. `feat(vt): add fixed predicted-camera feedback`
3. `refactor(vt): schedule fixed radial priority rings`
4. `refactor(assets): propagate page priority through bounded queues`
5. `feat(vt): batch predicted peripheral pages`
6. `test(vt): gate predicted foveation on real GPUs`
7. `docs(vt): publish predicted-foveation policy and results`

No phase proceeds past its exit gate, and implementation does not begin until
P1–P4 are explicitly resolved.

## 18. Decision response required

Implementation remains paused until the user accepts or overrides:

1. **P1:** predicted rank primary; current-only pages remain eligible in the
   farthest ring during the dual-view prototype;
2. **P2:** ring → parent/exact → channel → mip deficit → coverage → FIFO;
3. **P3:** benchmark 32/48/64 ms peripheral deadlines with a +5% source-byte
   ceiling while retaining 1/16 ms center deadlines;
4. **P4:** immediate demotion on prediction miss, cancellation/preemption only
   under capacity pressure or the existing two-epoch stale rule;
5. **P8:** prototype opt-in, then one global scheduler after all gates pass.

P5–P7 use the recommended technical prototypes unless the user changes their
scope. A concise approval such as “accept P1–P4 and P8 defaults” is sufficient
to authorize Phase 0/1 implementation; it does not waive later measured gates.
