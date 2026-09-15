# Native paint: final validation pass

Task `becb57128` completed `scripts/test-paint-final.sh` with exit code 0.
The run used the release Linux shell on the RTX 3090 and a separate headless Chromium process for the web target.
All live checks used private storage. The runners removed that storage and their processes after the checks.

## Checks

- 108 Rust tests passed: six memory, eight byte-store, 25 native paint, 40 shared paint, 28 browser, and one explicit hardware GPU test.
- 94 TypeScript tests passed: 91 editor tests and three comparison tests, with 3,470 assertions in total.
- TypeScript, WASM, Vite, native release, generated-web build/drift, mdBook, and diff checks passed.
- The GPU check retained exact pixels through region updates, CPU capture, resize, and texture retirement.
- Physical Restore/Discard checks passed after process termination, with the saved composition and history intact after Restore.
- Both 16K drawing runs completed ten minutes with the maximum UI brush radius of 60.

## 16K drawing results

| Measurement | Native | Public web |
|---|---:|---:|
| Duration | 600,037 ms | 600,010 ms |
| Completed strokes | 5,593 | 6,173 |
| Resident tiles | 5,120 | 5,120 |
| Final admitted tile limit | 6,144 | 6,144 |
| Initial 256 strokes: mean completion | 97.379 ms | 28.191 ms |
| Initial 256 strokes: p99 completion | 77.070 ms | 39.840 ms |
| Initial 256 strokes: maximum completion | 7,274.183 ms | 169.575 ms |
| rAF interval: mean | 6.968 ms | 16.666 ms |
| rAF interval: maximum | 54.988 ms | 16.670 ms |
| rAF intervals above 1000/55 ms | 1 / 86,110 | 0 / 36,002 |

The same initial stroke sequence crossed the initial 4,096-tile block on both targets.
Undo and redo retained the selected row exactly.
The comparison matched all 131,072 bytes across eight display rows, with zero differences.
It does not compare every document pixel.

**The native stroke-completion result is slower than the web result in this run.**
The native maximum was 7.274 seconds during the initial traversal.
The test records only aggregate stroke timings, so it does not identify that sample or its cause.
This latency needs a separate investigation. The successful correctness gate does not establish performance parity.
Stroke completion includes the command sequence, commit, publication, and probe acknowledgement.
rAF intervals measure frame production, not GPU presentation or input-to-pixel latency.
The native and headless-web rAF cadences differ, so their frame counts are not a throughput comparison.

## Memory and disk

The final three-minute native RSS floor increased from 794,886,144 to 795,004,928 bytes (118,784 bytes).
The browser process-tree PSS floor decreased from 1,136,358,400 to 1,033,118,720 bytes.
Both passed the specified floor-growth threshold: the larger of 32 MiB or five percent of the earlier floor.
Each run collected 60 memory samples.
These different process-memory measures are not a direct native/web memory comparison.
Ten minutes and a three-minute floor comparison do not prove an unlimited-session bound.

Native shared admission ended at 1,895,862,272 bytes, with a peak of 2,499,842,048 bytes under a 32,413,581,312-byte limit.
These are capacity reservations, not physical allocations or RSS.
The measured SQLite database/journal extent peak was 201,658,944 bytes, below the 64 GiB limit.
The quota regression separately forced journal exhaustion, verified rollback and continued writes, and rejected bypass operations.
Foreign allocators and GPU drivers remain outside the reservation counter.
The file limit excludes filesystem metadata and block-allocation overhead.

## Raster and recovery

The final 4K raster check changed 16 KiB per frame.
CPU commit time was 0.00596 ms mean and 0.026 ms p99 across 120 frames.
The earlier full-copy path measured 14.826 ms mean and 17.336 ms p99.
These separate runs do not establish identical frame conditions or end-to-end speedup.

Seed and Restore retained the 512 × 384 dimensions, hash 2063070052, 249 colored center-row pixels, and Group 1 / Layer 2 / Layer 1 order.
Discard returned a blank 2048 × 2048 document without history.
These recovery hashes are not byte-for-byte cross-process proof.
Lower-level store tests cover process exit before and after atomic commit.

## Evidence and reproduction

- `stress-native/result.json` and `stress-native/stress-memory.json`: native drawing and memory.
- `stress-web/result.json` and `stress-web/memory.json`: public-web drawing and process-tree memory.
- `stress-comparison.json`: exact-row, admission, disk, and memory-floor results.
- `final-raster/result.json`: final raster measurements.
- `final-recovery/result.json`: physical recovery results.

```sh
nix-shell shell.nix --run 'bash scripts/test-paint-final.sh'
```

The final pass emitted the existing mdbook-mermaid version warning. It did not prevent the book build.
