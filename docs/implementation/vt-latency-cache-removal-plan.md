# VT latency reduction and persistent-cache removal plan

Status: **core implementation and RTX 3090 latency validation complete 2026-
07-25; request-count exception, Radeon 680M profiles, and soaks pending**.

Baseline commit: `281c643` (`fix(telemetry): validate VT profiling semantics`).
Implementation result:

- `docs/benchmarks/dungeon-vt-no-cache-rtx3090-2026-07-25.md`
- Traverse and teleport latency/frame/trace gates pass on RTX 3090.
- Bulk requests are 2.94× the pre-removal hostile baseline versus the provisional
  2× gate. A 24 ms experiment still reached 2.34× and regressed latency/frame
  maxima. Deterministic source-order/grouping/mip-priority replay reproduced
  156 requests; sorting cut adjacent source runs 30.9% but not request count.
  The 16 ms policy remains selected pending explicit acceptance of the exception.

Baseline evidence:

- `docs/benchmarks/dungeon-vt-unified-telemetry-rtx3090-2026-07-25.md`
- `docs/benchmarks/dungeon-vt-unified-telemetry-rtx3090-2026-07-25.json`
- `docs/benchmarks/dungeon-vt-unified-telemetry-rtx3090-2026-07-25.agtb`

This plan removes the persistent derived-page cache and fixes measured VT
latency by reducing batching delay and bounding admission to actual transcode
throughput. It does not add another cache, increase worker count, alter the BIG
format, or change atlas/upload policy. The decision-gated follow-up
`predicted-perceptual-vt-scheduling-plan.md` investigates one 100 ms predicted
feedback camera, a bounded perceptual center/distance/coverage/resident-gap
score, and one longer peripheral batch lane; it is not part of this accepted
implementation.

## 1. Blocking product decisions

The user approved every recommended default below on 2026-07-25. They are now
acceptance constants; changing one requires a new explicit decision.

| Decision | Recommended default | Alternatives | Consequence |
|---|---|---|---|
| Persistent cache scope | Delete the generic `PersistentBlobCache` implementation and all runtime composition because VT is its only production consumer | Remove only VT composition but retain an unused public primitive | Full deletion removes OPFS, API, tests, telemetry and maintenance burden; retaining unused code violates YAGNI |
| Urgent bulk deadline | 1 ms | 0–4 ms | Parent/tail recovery latency versus request coalescing |
| Exact/quality bulk deadline | 16 ms | 8 ms latency-first; 32 ms request-count-first | Visible detail latency versus HTTP multipart request count |
| Public-web transcode workers | Keep 4 maximum | 6 or 8 | More workers consume separate WASM memories and CPU; queueing should be fixed before scaling |
| Total admitted page loads | 16 | 12 or 20 | Lower values cancel stale work earlier; higher values increase queue latency |
| Transcode waiting slots | 12, plus 4 active workers = 16 total | 8 or 16 waiting | Must match total admitted work without rejecting valid submissions |
| Pending-byte cap | 2 MiB | 1–4 MiB | Must fit 16 uncompressed RGBA pages and remain explicitly bounded |
| Feedback cadence | 55 ms monotonic interval | 33 or 66 ms | Detection latency versus feedback render/readback frequency |
| Source sorting | Leave unchanged in this change | Wire source-sorted provider | RTX profile showed 3 ms reads; do not mix an unmeasured transport change into this latency fix |
| Success target | Traverse p99 admitted page load ≤100 ms; hostile teleport reported separately | Different quality target | Determines whether worker scaling is admitted later |

These values are acceptance constants. Do not silently tune them during
implementation.

## 2. Non-goals and invariants

- Do not preserve a compatibility overload accepting a cache.
- Do not add an in-memory, service-worker, HTTP, IndexedDB, OPFS, or native
  replacement cache.
- Do not change `.big` source encoding or offline Basis/UASTC cooking.
- Do not increase the four-worker public-web profile in the first implementation.
- Do not alter the 4 MiB response, two-response, 8 MiB in-flight, or 256-span
  transport hard limits.
- Do not alter the atlas size, four-uploads-per-poll ceiling, page-table format,
  or shader fallback walk.
- Do not renumber telemetry descriptors. Existing AGTB evidence must remain
  decodable.
- Do not delete or rewrite historical benchmark evidence. Label it pre-removal.
- All new capacities are allocated during bootstrap. No sealed hot-path growth.
- Queue overflow defers work in the existing fixed scheduler; it never turns a
  valid page into a failed load.

## 3. Target pipeline after the change

```text
feedback snapshot (~55 ms cadence)
  -> fixed priority scheduler
  -> at most 16 admitted page generations / 2 MiB
  -> urgent 1 ms or quality 16 ms non-resettable bulk lane
  -> at most two / 8 MiB bulk responses in flight
  -> four active transcoders + at most twelve waiting jobs
  -> fixed ready-upload ring
  -> at most four uploads per poll
  -> page-table publication
```

The scheduler, not the transcoder queue, owns excess demand. Stale scheduler
entries are cheap to discard because they have not performed I/O or transcode.

## 4. Phase 0 — preserve and automate the baseline

### 4.1 Add a project-owned profiler

Create `scripts/profile-dungeon-vt.ts` so future agents do not use ad-hoc CDP
expressions. It must:

1. Accept `--cdp`, `--scenario`, `--duration-ms`, and `--output-prefix`.
2. Attach only to a page whose URL ends in `/dungeon.html`.
3. Fail unless `crossOriginIsolated`, `navigator.gpu`, and
   `window.__afterglowDungeon.ready()` are true.
4. Record adapter vendor/architecture and reject software/fallback adapters.
5. Support two fixed scenarios:
   - `traverse`: continuous collision-valid movement representative of play;
   - `teleport`: a new, explicitly defined nine-pose hostile burst with one
     pose change every 450 ms. Do not claim this is the existing `thrash`
     scenario: `scripts/soak-dungeon.sh` currently cycles eight poses every
     frame.
6. Wait for pending pages, scheduled requests, ready uploads, active
   transcodes, queued transcodes, and bulk in-flight work to reach zero.
7. Arm unified telemetry, run the scenario, freeze it, retrieve `traceBatch()`,
   and write `<prefix>.agtb` without altering bytes.
8. Decode a cold-path aggregate into `<prefix>.json` with stage counts,
   p50/p95/p99/max, frame intervals, statuses, drops, and unmatched spans.
9. Validate the AGTB header, record count, byte length, epoch, and 1 GHz tick
   rate before accepting output.
10. Exit nonzero on page errors, GPU errors, failed loads, dropped records,
    unmatched spans, timeout, or non-empty final queues.

Add a Bun test using a synthetic AGTB buffer for header validation and span
pairing. The script is diagnostic cold-path code; it is not imported by engine
or demo production modules.

### 4.2 Keep the accepted baseline immutable

Do not rerun or overwrite the `2026-07-25` files. New results use a new date and
`-no-cache` suffix. The old JSON's cache fields are historical evidence.

## 5. Phase 1 — delete persistent caching completely

Perform this as one isolated commit before latency policy changes.

### 5.1 Delete implementation and tests

Delete:

- `crates/afterglow-web/web/src/engine/assets/persistent-blob-cache.ts`
- `crates/afterglow-web/web/src/engine/assets/persistent-blob-cache.test.ts`

Remove the exports from
`crates/afterglow-web/web/src/engine/assets/index.ts`:

- `PersistentBlobCache`
- `persistentCacheNamespace`

Remove the stale `assets/persistent-blob-cache.ts: "budgeted"` entry from
`crates/afterglow-web/web/contracts/engine-allocation-effects.json`.

A final repository search must find no production/test references to either
symbol.

### 5.2 Remove cache ownership from BIG sessions

In `big-asset-session.ts`:

1. Remove the `PersistentBlobCache` import.
2. Remove `cache?: PersistentBlobCache` from `BigAssetSessionOptions`.
3. Remove the cache argument passed to `createPageDataProvider`.
4. Update startup tests to prove unknown/legacy cache wiring is absent rather
   than silently ignored.
5. Do not add a deprecated alias or compatibility overload.

This is an intentional breaking TypeScript API removal.

### 5.3 Remove cache work from the page provider

In `big-parser.ts`:

1. Remove the `PersistentBlobCache` import.
2. Remove the cache parameter from `createPageDataProvider`.
3. Delete cache-key string construction.
4. Delete cache `get` before bulk reads.
5. Delete fire-and-forget cache `put` for raw and transcoded pages.
6. Delete persistent-cache fields from `PageProviderStats` and its stable stats
   object.
7. Delete persistent-cache forwarding from `provider.getStats()`.
8. Retain numeric `req.cacheKey`; it is the bounded resident-page identity and
   telemetry correlation, not a persistent cache key.
9. Retain the GPU-resident `PageCache`; that is atlas residency, not persistent
   storage.

Replace the old cache-hit provider regression test with a regression proving two
identical non-resident provider calls both use source/transcode. Residency-level
deduplication remains tested in `virtual-texture-store.test.ts`.

### 5.4 Remove Dungeon OPFS composition

In `demos/dungeon/main.ts`:

1. Remove cache imports.
2. Remove source identity lookup used only for cache namespacing. Keep identity
   only if another validated consumer exists; otherwise delete the call.
3. Delete `PersistentBlobCache.open`, the 1 GiB/65,536-entry/64-write settings,
   and warning fallback.
4. Remove `cache` from `BigAssetSession.open`.
5. Remove persistent cache values from the dev-harness output/HUD if present.

The resulting demo must not touch OPFS during startup or traversal.

### 5.5 Disambiguate residency telemetry

There is no exported `VirtualTextureStats` interface today. The inferred stable
object returned by `VirtualTextureStore.getStats()` first writes resident-atlas
`cacheHits`, `cacheMisses`, and `cacheEvictions`, then the provider forwarding
block currently clobbers those same fields with persistent-cache values. Remove
that collision and rename the live resident counters in the stable store
object:

- `cacheHits` -> `residentHits`
- `cacheMisses` -> `residentMisses`
- `cacheEvictions` -> `residentEvictions`

Remove these persistent-only fields from `PageDataProviderTelemetry`,
`PageProviderStats`, and the inferred stable store stats object:

- `cacheEnabled`, `cacheBackend`, `cacheEntries`, `cacheBytes`,
  `cacheLiveBytes`, `cacheQueuedWrites`, `cacheCompactions`,
  `cacheReclaimedBytes`, `cacheMaintenance`, `cacheWrites`, `cacheRejected`,
  `cacheErrors`, `averageCacheReadMs`, `maxCacheReadMs`,
  `averageCacheWriteMs`, `maxCacheWriteMs`.

Update every live consumer. Required call sites include:

- the provider-to-store forwarding block in `virtual-texture.ts` that currently
  copies persistent fields over resident values;
- Dungeon's `churn` completion checks, which currently read
  `store.getStats().cacheEvictions` twice;
- the inline CDP expressions in `scripts/soak-dungeon.sh` and
  `scripts/baseline-vt-atlas.sh`;
- VT store tests and current API/book examples.

Do not edit old benchmark JSON/log files.

### 5.6 Preserve telemetry ABI

In `engine/telemetry/catalog.ts`:

- Keep IDs 19 and 20 occupied by the historical `cache.read` and `cache.write`
  descriptors.
- Add comments marking them reserved/no-producer after cache removal.
- Do not reuse them for new events.
- Keep `MeshOptimize = 21` and every prior descriptor ID unchanged.

No runtime code may emit descriptors 19 or 20 after this phase.

### 5.7 Cache-removal gates

Run and require all of the following:

```sh
! rg -n 'PersistentBlobCache|persistentCacheNamespace' \
  crates/afterglow-web/web/src
! rg -n 'navigator\.storage|getDirectory\(|indexedDB' \
  crates/afterglow-web/web/src/engine crates/afterglow-web/web/src/demos/dungeon
bun test crates/afterglow-web/web/src/engine
bun scripts/build-web.ts
cargo run -p xtask -- conformance
```

The first command may still match historical docs outside `web/src`; that is
handled in the documentation phase.

## 6. Phase 2 — make pipeline capacities explicit

### 6.1 Add one capacity object

In `virtual-texture.ts`, add:

```ts
export interface VirtualTextureRuntimeCapacities {
  maxPendingPages: number;
  maxPendingBytes: number;
}
```

Do not put these fields into `VirtualTextureTuningConfig`; that type controls
upload timing, not storage ownership.

Change `VirtualTextureStore` construction so a capacity object is mandatory.
Validate during construction:

- `maxPendingPages` is an integer >= 1;
- `maxPendingBytes` is an integer >= one atlas slot payload;
- all fixed arrays/maps use `maxPendingPages`;
- no fallback to 64 or 8 MiB remains.

Delete hard-coded fields:

```ts
private readonly maxPendingPages = 64;
private readonly maxPendingBytes = 8 * 1024 * 1024;
```

Store validated constructor values instead.

### 6.2 Thread capacities through `BigAssetSession`

Add required fields to `BigAssetSessionOptions`:

```ts
maxPendingPages: number;
maxPendingBytes: number;
```

Validate them in `open`, retain them in the session, and pass them to
`VirtualTextureStore` from `createVirtualTextureStore`.

Dungeon values after approval:

```ts
maxPendingPages: 16,
maxPendingBytes: 2 * 1024 * 1024,
```

Update every direct `VirtualTextureStore` construction in tests/demos with an
explicit capacity object. This explicitly includes:

- `BigAssetSession.createVirtualTextureStore`;
- `createProceduralVirtualTextureStore` in
  `engine/virtual-texturing/index.ts` and its public signature;
- `demos/vt/main.ts`;
- `demos/rigged-vt/main.ts` through its required `BigAssetSessionOptions`;
- every construction in `virtual-texture-store.test.ts` and related binding
  tests.

Update the existing `toBe(64)` pending-capacity assertions in
`virtual-texture-store.test.ts` to assert each test's explicit capacity. Do not
add optional defaults to reduce edit count.

### 6.3 Set transcode waiting capacity correctly

`BoundedTranscoderPool`'s capacity counts waiting jobs, not active workers.
Use:

```ts
workerCount: 4,
transcodeQueueCapacity: 12,
```

Four active + twelve waiting independently bounds the transcode stage to the
same maximum size as the sixteen-page whole-pipeline admission ceiling. These
are separate capacities: admitted pages may still be waiting on bulk I/O or
ready upload, so do not assert that every admitted page occupies a transcode
slot. Validate in `BigAssetSession.open` that
`transcodeQueueCapacity >= 1`; do not derive it silently from worker count
because ownership must remain explicit.

### 6.4 Admission behavior

Keep existing scheduler semantics:

- `queuePageLoad` returns false when capacity is unavailable;
- the request remains in the fixed priority scheduler;
- `failedLoads` is not incremented;
- worse pending work may be marked canceled, but its slot is not reused until
  async acknowledgment;
- source/transcode queue overflow is a bug after capacities are aligned.

Add tests proving:

1. no more than 16 providers are simultaneously admitted;
2. the seventeenth request remains scheduled and is admitted after completion;
3. a higher-priority request cancels a worse admitted request without exceeding
   16;
4. cancellation acknowledgment releases exactly one slot and byte charge;
5. queue saturation never increments `failedLoads`;
6. pending bytes never exceed 2 MiB.

Do not add a second credit manager in the initial implementation. The one
admission ceiling is the backpressure mechanism. Add deeper bulk/transcode
credit coupling only if the measured queue p99 gate fails.

## 7. Phase 3 — reduce bulk batching delay

### 7.1 Replace positional provider policy with a config object

In `big-parser.ts`, define:

```ts
export interface PagePipelineConfig {
  transcodeQueueCapacity: number;
  urgentBatchDeadlineMs: number;
  qualityBatchDeadlineMs: number;
}
```

Change `createPageDataProvider` to accept this object instead of cache and
positional queue-capacity arguments. Suggested final order:

```ts
createPageDataProvider(loader, header, workers, format, config, telemetry?)
```

Validation:

- queue capacity is an integer >= 1;
- deadlines are finite integer milliseconds >= 0;
- urgent deadline <= quality deadline;
- reject invalid values before creating timers/workers.

Add required deadline fields to `BigAssetSessionOptions`, build the config once
at bootstrap, and pass it to the provider.

Dungeon values after approval:

```ts
transcodeQueueCapacity: 12,
urgentBatchDeadlineMs: 1,
qualityBatchDeadlineMs: 16,
```

Update every `BigAssetSession.open` call, especially Dungeon,
`demos/rigged-vt/main.ts`, and all five constructions in
`big-asset-session.test.ts`. Update all direct provider constructions in
`big-parser.test.ts` to use the config object instead of the old positional
cache/queue arguments. Required options mean omission must be a TypeScript
error.

### 7.2 Parameterize `BoundedBulkReadQueue`

Change its constructor to retain the two validated deadlines. Replace:

```ts
private deadlineMs(tier: number): number { return tier === 0 ? 1 : 100; }
```

with reads from fixed constructor fields. Preserve:

- first-arrival opens a timer;
- later arrivals never reset it;
- urgent ready work wins;
- full/byte-limited responses dispatch immediately;
- aborted slots are removed before dispatch;
- at most two responses and 8 MiB are in flight.

Do not introduce a third lane in this change. The existing quality lane is the
exact-page promotion lane; reducing it to 16 ms is the selected policy.

### 7.3 Batching tests

Use a fake clock/timer where practical; never sleep 100 ms in unit tests. Prove:

1. urgent dispatches at 1 ms;
2. quality dispatches at 16 ms;
3. a second arrival does not move either deadline;
4. urgent preempts ready quality work;
5. quality eventually dispatches under bounded urgent traffic;
6. close clears both timers and rejects all retained slots;
7. byte/span limits remain unchanged;
8. dispatch preserves caller-result order.

## 8. Phase 4 — make feedback cadence time-based

### 8.1 Change coordinator API

In `virtual-texture-feedback-coordinator.ts`:

- replace constructor field `cadence: number` with
  `cadenceMs: number`;
- validate finite `cadenceMs > 0`;
- add a fixed `nextFeedbackSeconds` scalar initialized during bootstrap;
- in `render(frame)`, compare `frame.elapsedSeconds` against the deadline;
- after a successful scheduling opportunity, set the next deadline from the
  current elapsed time; do not submit catch-up passes after a stall;
- if a prior readback is still awaiting, retain the existing deferred counter
  and advance the next attempt without allocating or spinning.

Dungeon value after approval:

```ts
cadenceMs: 55,
```

Delete every frame-count cadence constant, including `FEEDBACK_INTERVAL = 8`
in Dungeon and rigged VT. Update all coordinator construction call sites:
Dungeon, `demos/rigged-vt/main.ts`, `demos/vt/main.ts`, and coordinator tests.
Atlas diagnostic stepping must convert the selected cadence to elapsed-time
waits rather than assuming eight frames.

### 8.2 Cadence tests

Update coordinator and surface-detail tests to prove:

- no submission before 55 ms;
- one submission on the first frame at/after 55 ms;
- approximately eight-frame behavior at 144 Hz;
- approximately four-frame behavior at 60 Hz;
- no burst of catch-up submissions after a 500 ms stall;
- pending readback still prevents overlap;
- the source-contract assertion in `surface-detail.test.ts` no longer requires
  `FEEDBACK_INTERVAL=8` and instead verifies the approved millisecond policy;
- zero allocations in the render hot region.

## 9. Phase 5 — complete latency telemetry

Append descriptors after ID 21; never insert or reorder:

| ID | Name | Kind | Correlation |
|---:|---|---|---|
| 22 | `vt.feedback_detected` | instant | packed resident-page key |
| 23 | `vt.scheduler_wait` | async span | same key |
| 24 | `vt.page_published` | instant | same key |

Semantics:

1. `feedback_detected` fires when a nonresident effective request first enters
   the fixed scheduler, not on every duplicate feedback sample.
2. `scheduler_wait` begins at that insertion and ends when admitted or canceled.
3. Existing `vt.page_load` covers admission through ready data.
4. Existing `vt.upload` covers atlas/page-table publication CPU work.
5. `page_published` fires only after atlas bytes and page-table entry are both
   committed.

Record cancellation status at the scheduler and page-load endpoint. Keep the
65,536-record Dungeon capacity and fail profiling if it overflows.

First-frame sampling proof is a separate renderer integration: publication in
the worker stage makes the page eligible for the same frame's later render.
Record the current runtime frame ID on `page_published`; do not claim actual
fragment sampling without a GPU-visible proof.

Add tests for one correlation across feedback -> scheduler -> load -> upload ->
publish and for cancellation before each expensive stage.

## 10. Phase 6 — documentation cleanup

### 10.1 Delete current public cache documentation

Delete:

- `docs/api/persistent-blob-cache.md`
- `book/src/window/persistent-cache.md`

Remove the book entry from `book/src/SUMMARY.md`.

### 10.2 Update current API documentation

Update in the same commit:

- `docs/api/asset-system.md`
- `docs/api/virtual-texturing.md`
- `docs/api/runtime-capacities.md`
- `docs/api/telemetry.md`
- `book/src/window/asset-system.md`
- `book/src/window/virtual-texturing.md`
- `book/src/reference/telemetry.md`
- relevant testing/building chapters if profiler commands are added
- `AGENTS.md`, removing `docs/api/persistent-blob-cache.md` from the canonical
  API list while retaining the superseded research entry

Document the exact 16-page/2 MiB/4+12/1 ms/16 ms/55 ms profile and deterministic
overflow behavior. In `docs/api/runtime-capacities.md`, reconcile both existing
rows: replace the current 64-page/8 MiB pending limit and clarify that the
separate "8 admissions" value is a per-poll operation budget, not total pending
capacity (or remove it if source no longer implements that meaning).

### 10.3 Preserve historical records honestly

Do not delete:

- `docs/research/device-transcoded-texture-cache.md`
- old audits, implementation plans, benchmark logs, JSON, or AGTB captures.

Add a dated status banner to the research note: the cache was evaluated and
then removed because the selected runtime favors a bounded no-cache pipeline.
Annotate old implementation plans as historical where they prescribe the cache.
The 2026-07-25 benchmark report must state that cache fields describe the
pre-removal build.

### 10.4 Versioning

Treat removal of `PersistentBlobCache`, namespace helpers, cache session options,
and stats fields as a breaking authored TypeScript API change. Record it in API
docs/release notes. The web package is private and unversioned; do not invent a
Cargo version bump unless a workspace release includes this API.

## 11. Phase 7 — full validation matrix

### 11.1 Static and unit gates

```sh
bun test crates/afterglow-web/web/src/engine
bun scripts/build-web.ts
bun scripts/build-web.ts --check
cargo run -p xtask -- conformance
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd book && nix-shell -p mdbook mdbook-mermaid --run 'mdbook build'
git diff --check
```

All allocation-effect, import-boundary, generated-artifact, demo-architecture,
and API tests must pass.

### 11.2 Runtime scenarios

Run each scenario from a fresh browser profile; cache state no longer exists.
Run at 144 Hz and 60 Hz where supported:

1. profiler `traverse` — realistic continuous path;
2. profiler `teleport` — nine hostile poses every 450 ms;
3. soak `stable` — existing fixed normal gameplay pose;
4. soak `traverse` — existing continuous path;
5. soak `thrash` — existing eight-pose-per-frame hostile mode;
6. atlas `churn` — existing full-atlas replacement mode;
7. 30-minute traverse soak;
8. 60-minute thrash soak.

Do not pass `teleport` to `soak-dungeon.sh` or `churn` to a soak script; they
belong to different harnesses.

Run on:

- RTX 3090 workstation Chromium/WebGPU;
- Radeon 680M laptop Chromium/WebGPU;
- native shell separately once Dungeon native launch is available; never infer
  native results from public-web workers.

### 11.3 Acceptance gates

The implementation is accepted only if all are true:

| Gate | Traverse target | Teleport target |
|---|---:|---:|
| Batch-wait p99 | <=20 ms | <=20 ms |
| Transcode-queue mean | <=20 ms | <=30 ms |
| Transcode-queue p99 | <=40 ms | <=60 ms |
| Admitted page-load mean | <=75 ms | report; no hard gate until measured |
| Admitted page-load p99 | <=100 ms | <=150 ms |
| Atlas upload p99 | <=0.25 ms | <=0.25 ms |
| Failed loads | 0 | 0 |
| Trace drops/unmatched spans | 0/0 | 0/0 |
| Final pending/scheduled/ready/active/queued | all zero | all zero |
| Source bytes versus baseline | <=+5% | <=+5% |
| Bulk request count versus baseline | <=2x | <=2x |
| Frame p99/max regression | none beyond run variance | no new >60 Hz misses on RTX |

The hostile teleport workload is not allowed to redefine normal gameplay
quality policy. It proves deterministic degradation and cancellation.

### 11.4 Soak gates

After sealed gameplay:

- JS heap floor plateaus;
- no OPFS handles, cache tasks, or cache timers exist;
- timer count plateaus;
- pending/scheduled/transcode/bulk depths repeatedly drain;
- no post-seal pipelines or general allocations appear;
- no queue overflow, failed page, GPU error, or long-task regression;
- AGTB record counts remain bounded and exact.

## 12. Failure handling and rollback rules

- If removing cache breaks a consumer, identify it with repository evidence;
  do not restore the cache speculatively. Return to the user for an ownership
  decision.
- If 16 admitted pages lowers throughput enough to miss visible pages, test 20
  before adding workers. Report memory and stale-work deltas.
- If queue p99 remains above 40 ms with 16 total admitted jobs, inspect stage
  correlations for capacity-accounting bugs before increasing workers.
- If 16 ms batching exceeds the 2x request-count gate, test 24 then 32 ms; do
  not return directly to 100 ms. The RTX follow-up rejected 24 ms and proved
  source sorting plus priority/grouping cannot change request count at the
  existing admission/deadline opportunities. Reopen buffering, prefetch, or
  cooked-superpage policy rather than repeating those tie-break experiments.
- If four workers remain throughput-limited after admission/backpressure is
  correct, benchmark six workers as a separate product/memory decision.
- If 55 ms feedback regresses GPU/frame timing, test 66 ms; do not restore a
  frame-count cadence because it is refresh-rate dependent.
- Never overwrite accepted baseline evidence. Revert code commits, not data.

## 13. Required commit sequence

Use small semantic commits in this order:

1. `test(vt): add reproducible unified profile harness`
2. `refactor(assets): remove persistent blob cache`
3. `refactor(vt): make pipeline capacities explicit`
4. `fix(vt): bound admission and reduce batch latency`
5. `fix(vt): use time-based feedback cadence`
6. `feat(telemetry): trace VT scheduler to publication`
7. `docs(vt): record no-cache latency architecture`
8. `test(vt): record no-cache GPU and soak evidence`

Every commit must build and pass its targeted tests. Do not combine cache
removal with timer/capacity tuning; bisectability is required.

## 14. Definition of done

- No persistent cache implementation, API, runtime reference, storage access,
  live statistic, or user-facing chapter remains.
- Historical cache research/evidence remains clearly marked as superseded.
- The entire page pipeline has one explicit 16-page/2 MiB admission boundary.
- Four workers have at most twelve queued transcodes.
- Urgent/quality deadlines are explicit 1/16 ms bootstrap policy.
- Feedback uses a 55 ms monotonic cadence.
- Excess demand stays in the bounded priority scheduler and stale work is
  canceled before expensive stages.
- One trace correlation follows a page from feedback through publication.
- Traverse latency meets every acceptance gate on RTX 3090 and Radeon 680M.
- Generated web deployment, API docs, mdBook, tests, conformance, and benchmark
  evidence are current.
