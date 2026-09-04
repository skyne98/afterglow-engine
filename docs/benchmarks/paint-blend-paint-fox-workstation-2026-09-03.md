# Radius-60 blend-and-paint worker profile — 2026-09-03

## Configuration

The test used these items:

- AMD Ryzen 9 9950X3D with 32 logical CPUs
- Chromium 146 with cross-origin isolation
- The character-editor Vite development server
- A 2,048 x 2,048 document
- The `blend+paint` brush with radius 60
- 280 pointer samples at a 4 ms interval
- A 695-pixel horizontal path

A CDP trace recorded all paint-worker `FixedTaskWake` function calls. The test waited eight seconds after pointer release and found an empty motion queue.

## Result

| Metric | Source profile | Corrected profile |
|---|---:|---:|
| Wake count | 491 | 201 |
| p50 | 13.378 ms | 0.245 ms |
| p90 | 26.017 ms | 0.295 ms |
| p95 | 29.227 ms | 0.319 ms |
| p99 | 35.159 ms | 1.165 ms |
| Maximum | 42.171 ms | 15.702 ms |
| Wakes above 16.7 ms | 193 | 0 |

The source profile is `/home/fox/Downloads/Trace-20260903T005119.json`. The corrected profile used the automated sample path above. Thus, the wake count and total work are not a direct throughput comparison.

The corrected CPU profile shows sparse smudge sampling on the paint worker. Full-tile dab work runs in the tile workers. The display queue was empty after the test.

## Quality gate

The correction does not remove or combine brush samples. It does not change brush radius, spectral bins, or compositor math.

The Rust test set compares sparse and eager smudge results, scalar compositor results, row-sliced jobs, and the C parity oracles. All tests passed. The TypeScript test set and type check also passed.

A Chromium wasm check compared all 4,096 pixels for the SIMD normal, eraser, alpha-lock, and one-layer compositor paths. Each path matched the scalar fix15 equations exactly.
