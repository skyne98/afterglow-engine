# Dungeon VT batching/priority trace replay — RTX 3090 — 2026-07-25

## Question

Can the existing no-cache RTX traces reach the provisional hostile-teleport
request gate (at most 2× the old 53-request baseline, or **106 requests**) by:

1. sorting admitted page spans into source order;
2. grouping overlapping material-channel demand;
3. prioritizing large desired-to-resident mip deficits?

This is an offline sensitivity experiment. It does not change runtime or cook
policy.

## Method

`scripts/replay-dungeon-vt.ts` reads the committed AGTB trace and the cooked
`dungeon.big` v5 directory. It reconstructs:

- every scheduler admission from `vt.scheduler_wait`;
- every successful/canceled pre-dispatch page from `vt.bulk_wait`;
- the two independent, non-resetting 1/16 ms batch deadlines;
- each packed texture/mip/x/y identity and exact source offset;
- recorded `asset.bulk_dispatch` count.

The deadline replay exactly reproduced both recorded request counts: **156**
teleport and **223** traverse.

Source sorting keeps each batch and deadline unchanged, sorts its spans by BIG
offset, then counts adjacent source runs after merging exact neighbors. This is
the maximum benefit available to the existing source-sorted native adapter; an
HTTP multipart request remains one request regardless of span order.

The priority experiment preserves the successfully dispatched page set and the
observed admission opportunities. Its “mip deficit” is only a proxy obtained by
reversing the current coarse-first mip rung. Channel affinity may select an
already-detected overlapping page from another channel within one 22-lane
priority band.

**Priority results are sensitivity data, not causal replay.** AGTB does not
record every feedback refresh or the current resident fallback mip. Canceled
requests can open/anchor a deadline, so the successful-only priority control has
222 modeled traverse requests while the complete deadline replay and trace have
223.

## Results

| Measure | Traverse | Hostile teleport |
|---|---:|---:|
| Successful page reads | 928 | 1,074 |
| Canceled before dispatch | 5 | 2 |
| Recorded bulk requests | 223 | 156 |
| Complete deadline replay | 223 | 156 |
| Mean successful spans/request | 4.16 | 6.88 |
| Caller-ordered adjacent source runs | 637 | 740 |
| Source-sorted adjacent source runs | 579 | 511 |
| Source-run reduction | **9.1%** | **30.9%** |
| Requests after source sorting | **223** | **156** |

Priority sensitivity:

| Variant | Traverse requests / admission p99 / max | Teleport requests / admission p99 / max |
|---|---:|---:|
| Recomputed current priority | 222 / 4,231.91 / 4,252.64 ms | 156 / 494.24 / 1,716.20 ms |
| Mip-deficit proxy first | 222 / 3,927.11 / 4,294.50 ms | 156 / 494.14 / 1,758.00 ms |
| Mip deficit + channel affinity | 222 / 3,871.12 / 4,294.50 ms | 156 / 494.14 / 1,758.00 ms |

The apparent traverse p99 movement is not an acceptance result: the modeled max
regresses and the trace lacks the resident-mip/refresh state needed to establish
which page would really remain relevant.

## Verdict

**No tested transformation reduces hostile teleport below 156 requests; the
106-request gate is not reached.**

The reason is structural:

- source order changes work *inside* an already-created bulk request;
- channel grouping and priority choose *which* page consumes each observed
  admission opportunity;
- request count is set mainly by when bounded pending/transcode slots free and
  when the 1/16 ms non-resetting deadlines expire.

Source sorting is still technically useful for the native path: this trace
predicts 31% fewer adjacent source runs under hostile teleport. It is not a
request-count optimization and must be measured independently when the existing
source-sorted CEF provider is wired into production.

Reaching 106 at the selected deadlines would require a different mechanism,
not a scheduler tie-break:

- admit/buffer substantially more pages per burst;
- prefetch scheduled pages before normal admission;
- or cook several logical pages into larger independently readable superpages.

Each alternative changes bounded memory, source format, wasted-byte, latency,
and cancellation policy. None is adopted by this experiment. The evidence
supports retaining the measured 16 ms policy and treating 2.94× as an explicit
request-count exception unless that product trade-off is reopened.

## Reproduction

```sh
bun test scripts/replay-dungeon-vt.test.ts

bun scripts/replay-dungeon-vt.ts \
  --trace docs/benchmarks/dungeon-vt-no-cache-teleport-rtx3090-2026-07-25.agtb \
  --output /tmp/teleport-replay.json

bun scripts/replay-dungeon-vt.ts \
  --trace docs/benchmarks/dungeon-vt-no-cache-traverse-rtx3090-2026-07-25.agtb \
  --output /tmp/traverse-replay.json
```

Committed aggregate evidence:

- `docs/benchmarks/dungeon-vt-replay-teleport-rtx3090-2026-07-25.json`
- `docs/benchmarks/dungeon-vt-replay-traverse-rtx3090-2026-07-25.json`
