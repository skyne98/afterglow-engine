# Probe-GI reference harness

A tiny, hand-checkable GI pipeline used to verify each stage of the real
implementation separately. The point is *order*: the CPU reference is written and
checked first, then every GPU stage is diffed against it, so a mismatch is always
localized to one stage instead of showing up as "the picture looks wrong".

## Why the scene looks like this

`scene.js` is deliberately trivial and *exact*: a ground quad and a five-sided box,
a white point light, and a **constant** sky radiance. Tracing is brute force over
12 triangles — no BVH — because a reference must not share an acceleration
structure (or a bug) with the thing it verifies. Every probe ray lands on a value
that can be worked out by hand:

| Ray | Expected |
|---|---|
| probe 0 `(0, 0.5, -1)`, -Y | hits the ground, `t = 0.5` |
| probe 1 `(1, 0.5, -1)`, -Y | hits the box top, `t = 0.1` |
| probe 3 `(1, 1.5, -1)`, -Y | hits the box top, `t = 1.1` |
| any +Y ray | miss, radiance = sky |
| ground at `(0, 0, -1)` | lit: `albedo · E / π`, `E = 8/(1+0.09d²) · NdotL` |
| ground under the box | shadowed: exactly zero |

The ground rectangle and the box are both offset off the probe grid lines on
purpose: a ray that passes exactly through a shared triangle edge is a tie, and an
f64 CPU and an f32 GPU are entitled to break a tie differently. `test-cpu.mjs`
asserts the tie margin directly, so the geometry cannot drift into a comparison
failure that is really an arithmetic tie.

## Stages

| Stage | CPU | GPU | Status |
|---|---|---|---|
| 1. probe grid positions | `probePositions()` | `probePos` buffer readback | ✅ exact |
| 2. probe ray trace (hit, t, backface rule, direct radiance, sky) | `traceProbeRays()` | `probeTrace` compute | ✅ 8e-8 |
| 3. atlas blend (octahedral texel dirs, cosine weights, gamma-5) | `blendProbe()` | `blendAtlas` compute | ✅ 1.1e-7 |
| 4. atlas texture path (packed tiles, borders, bilinear, hysteresis) | `sampleTileBilinear()` | texture + sampler | not yet |
| 5. cage sample (trilinear × backface × Chebyshev, biased position) | `sampleCascade()` | fragment helper | not yet |
| 6. fragment shade (direct/π + indirect·AO, ACES, sRGB) | `shadeFragment()` | full render | not yet |
| 7. probe init / validity / relocation | not yet | not yet | not yet |

## Run

```sh
node prototype/gi-ref/test-cpu.mjs            # hand-checkable CPU checks (must pass first)
prototype/gi-ref/run-gpu.sh <logname>         # runs gpu.js in the shell, then diffs
```

`run-gpu.sh` stops the shell as soon as the page logs `GIREF done` (the shell's own
startup timeout is ~70 s) and pipes the log through `compare.mjs`, which prints a
per-stage maximum absolute error and fails over tolerance:

```
ok   probe positions              max |cpu - gpu| = 0.000e+0 (tol 1e-6)  8 probes
ok   probe rays (radiance + t)    max |cpu - gpu| = 8.003e-8 (tol 1e-5)  48 rays, 8 hits
ok   atlas blend (gamma-5 encoded) max |cpu - gpu| = 1.055e-7 (tol 1e-4)  288 texels
```

Stage data crosses the boundary as one text line per stage (`GIREF <stage> <json>`)
because the shell has no artifact-write op for a page; if the tables grow, a bounded
`op_write_artifact` in `afterglow-shell` would be the right addition rather than
larger log lines.
