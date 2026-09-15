# Native canvas publication baseline

The RTX 3090 release shell completed this measurement with the current Canvas2D raster path.
Each frame changed one 64 × 64 RGBA8 region (16 KiB).
The canvas displayed at 512 × 512 CSS pixels for all three source sizes.
Each size used 20 warm-up frames and 120 measured frames.

| Source size | Published bytes per frame | Tile-copy mean | Commit mean | Commit p99 | rAF mean | rAF p99 |
|---|---:|---:|---:|---:|---:|---:|
| 512² | 1 MiB | 0.101 ms | 0.110 ms | 0.285 ms | 16.712 ms | 16.808 ms |
| 2048² | 16 MiB | 0.099 ms | 1.820 ms | 4.341 ms | 16.707 ms | 16.783 ms |
| 4096² | 64 MiB | 0.095 ms | 14.826 ms | 17.336 ms | 21.963 ms | 28.388 ms |

A 16K paint document uses a 4K display raster.
At that raster size, the native commit copied 4,096 times the changed byte count.
Tile copying stayed near 0.1 ms, but synchronous commit time increased with raster size.
The source path copies the full JavaScript buffer into a new Rust vector and a new immutable Blitz image.
Vello then receives the new image identity.
This result supports a dirty-region upload experiment. It does not select an implementation by itself.

## Evidence and limits

- `result.json` contains timing and process-memory values.
- `raster.log` contains the native process output.
- `paint-raster-probe.ts` checked the changed pixel and an unchanged corner.
- Commit time is synchronous CPU time, not a GPU timestamp.
- rAF time measures frame production, not input-to-present latency.
- The process resident high-water value was 730,704 KiB. This includes other native services.
- This short check does not prove a memory plateau or the paint RAM ceiling.
- TypeScript, 90 editor tests, Vite, and the diff check also passed.

## Run

Build the release shell and editor first. Then run from the repository root:

```sh
nix-shell shell.nix --run 'bun scripts/test-native-paint-recovery.ts docs/benchmarks/native-paint-shell/raster-baseline --raster'
```

The runner uses private temporary storage and removes it after the measurement.

## Persistent-texture experiment

`crates/afterglow-shell/examples/bench_canvas_upload.rs` compares immutable image publication with Vello's registered-texture API.
It measures a full image copy against a 16 KiB texture update and compares rendered output at two transforms.
The experiment uses the existing Vello and wgpu dependencies, with no Canvas2D runtime change.
The RTX 3090 completed 120 samples per mode and size with no GPU validation errors.
`texture-comparison.json` contains the results.

| Source size | Immutable setup/submit mean | Persistent setup/submit mean | Immutable completion mean | Persistent completion mean |
|---|---:|---:|---:|---:|
| 512² | 0.227 ms | 0.158 ms | 0.537 ms | 0.437 ms |
| 2048² | 2.582 ms | 0.156 ms | 3.260 ms | 0.476 ms |
| 4096² | 21.066 ms | 0.182 ms | 23.613 ms | 1.118 ms |

At 4096², persistent setup/submit p99 was 0.367 ms and completion p99 was 1.933 ms.
The persistent path uploaded 16 KiB from the CPU per sample, instead of 64 MiB.
Vello still copies the full texture into its GPU atlas after a change.
Blocking completion time is not a GPU timestamp or a presentation measurement.
These values do not include the native JavaScript bridge or document layout.
The modes ran in a fixed order, not in randomized order.

All six comparisons with the same image identity returned exactly the same output bytes.
They include scaling, fractional rotation, filtering, and alpha at all three source sizes.
With different image identities, one green channel differed by one byte value at 512² with rotation (212 versus 213).
The initial strict comparison rejected this difference.
The experiment now records different-identity differences separately and keeps the same-identity comparison exact, with no tolerance.
Image identity can change atlas placement, so the different-identity comparison does not isolate the upload method.
The current evidence does not establish the cause of that one-byte difference.
Native integration, texture retirement, resize, CPU capture, and long-run memory checks remain incomplete.

```sh
nix-shell shell.nix --run 'cargo run --release -p afterglow-shell --example bench_canvas_upload'
```
