# Native canvas region publication

## Result

The RTX 3090 release shell completed the changed-region raster check.
The source canvas used 512², 2048², and 4096² pixels, with a 512² CSS display.
Each frame changed one 64² RGBA8 region (16 KiB).
Each size used 20 warm-up frames and 120 measured frames.

| Source size | CPU commit mean | CPU commit p99 | rAF mean | rAF p99 | rAF maximum |
|---|---:|---:|---:|---:|---:|
| 512² | 0.0172 ms | 0.028 ms | 16.701 ms | 16.754 ms | 16.755 ms |
| 2048² | 0.0172 ms | 0.039 ms | 16.757 ms | 19.634 ms | 22.764 ms |
| 4096² | 0.0125 ms | 0.026 ms | 16.696 ms | 16.890 ms | 17.237 ms |

At 4096², the previous full-raster commit took 14.826 ms mean and 17.336 ms p99.
Its rAF interval was 21.963 ms mean and 28.388 ms p99.
See `../raster-baseline/` for that separate run.

The region path copies changed rows into a retained CPU raster.
It uploads the pending region into a persistent source GPU texture.
Vello still copies the full changed source texture into its GPU atlas.
Initial publication and resize still copy and upload the full raster.
The source RGBA buffer stays available to JavaScript without a new typed-array view per commit.

## Pixel and lifetime checks

The hardware test `browser::canvas::tests::gpu_regions_keep_pixels_across_capture_resize_and_removal` passed.
It checks exact source-texture bytes and compares region publication with a full GPU upload through the same image identity.
It includes partial updates between frames, combined pending regions, alpha, scaling, CPU capture, resize, and texture retirement.
No pixel tolerance is permitted.
The test completed without GPU validation errors.

An initial version compared CPU-rasterized output with GPU-rasterized output.
It failed on one-byte channel differences during 64-to-32 filtering.
The corrected test keeps the GPU rasterizer and image identity the same for the upload comparison.
It separately checks that GPU publication does not change CPU capture output.

The same verification run passed 27 browser tests, the hardware test, and 91 editor tests (3,464 assertions).
It also passed the release build, TypeScript, Vite, and generated-web checks.
The later physical recovery check stopped safely because the pointer was outside its owned window.
That failure is in `../recovery-regions/restore.log` and the task log for `b95b24311`.
The runner did not click outside its window.
The separate run in `../recovery-regions-ready/` subsequently passed Restore/Discard with the pointer-window check still mandatory.

## Evidence and limits

- `result.json`: measured values and process memory.
- `raster.log`: native adapter information and probe output.
- Whole-process VmHWM and final VmRSS: 456,136 KiB.
- CPU commit timing excludes subsequent GPU work.
- rAF measures frame production, not presentation or input latency.
- The test is short. It does not establish a sustained memory bound or a long-run timing limit.
- A 4K display raster is applicable to a 16K paint document at display scale 4.
- This standalone canvas check does not exercise 16K brush work, paging, or native/web parity.
- No artist configuration or fixed canvas-count limit was added.
- Aggregate memory admission, Vello atlas memory, and disk journal bounds remain open.

## Run

```sh
nix-shell shell.nix --run 'bun scripts/test-native-paint-recovery.ts docs/benchmarks/native-paint-shell/raster-regions --raster'
```

The runner uses private temporary storage and removes it after the check.
