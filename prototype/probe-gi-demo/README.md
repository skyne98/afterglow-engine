# Probe-GI reference demo

The design in `docs/research/probe-based-gi.md` (§7.6, §10) running as a raw page in
the native shell: 3 camera-locked cascades of 8×4×8 probes at 1/3/9 m spacing (768
resident), 64 rays/probe, a binary BVH for the probe rays *and* the shading shadow ray,
per-probe 8×8 octahedral irradiance and distance maps with a 1-texel border in a
288² `rgba16float` atlas (ping-pong), grid-snapped scrolling with modulo slot
indexing, and probe validity/relocation at init. Fully dynamic and map-agnostic: no
bake, no authored volumes.

Live numbers for this state, and the defects validation found, are in §7.6 of the
research doc. Read that before changing a bias, the relocation bound, or the atlas
mapping — each of those has a measured failure attached.

## The page reads its JS at load

Editing any of these modules while an instance is running changes **nothing** — restart the
shell to see a change. (A report of "still broken" after an edit is usually a stale
process.)

## Run

```sh
# W=960 H=540 ./prototype/probe-gi-demo/run.sh <logname> [page] [seconds]
./prototype/probe-gi-demo/run.sh last-run                                  # simple scene, 3 captures
./prototype/probe-gi-demo/run.sh room prototype/probe-gi-demo/configs/complex.js
./prototype/probe-gi-demo/run.sh soak prototype/probe-gi-demo/configs/complex-orbit.js 20
```

`run.sh` preflights every module with `node --check` (a stray backtick inside a WGSL
template literal is the usual syntax error), starts the shell, stops it on the
`capture-all-done` marker or after `seconds`, and decodes each captured view to
`frame-<n>.png`. Any `WGSL ERROR` or panic fails the run instead of leaving stale
frames in place.

## Configs (`configs/`)

| Entry | Purpose |
|---|---|
| `complex.js` | the enclosed room, three fixed poses (interior, doorway, overview) |
| `complex-orbit.js` | the room with the camera orbiting inside it: continuous reseed + relocation |
| `sealed.js` | **leak test**: the room with its doorway sealed, so nothing can carry light in. Any interior light is a leak |
| `nocapture.js` | soak without captures (isolates capture cost from render cost) |
| `live.js` | the room, captures off, free-fly: the entry to hand to a human |
| `noupdates.js` | `probeUpdates: false` (isolates render cost from probe updates) |
| `repro.js` | the camera pose from the reported "harsh uniform squares" artifact (read off the reporter's HUD) |
| `repro-indirect.js` / `repro-weights.js` | the same pose with the indirect term and the cage weight sum isolated |
| `closeup.js` | three near views over the cube's shadowed side, where GI-only lighting exposes cage/atlas defects |
| `freshband.js` | forces a reseeded scroll band every frame (`CFG.forceFreshBand`): the only way to see the reseed/hysteresis step with a static camera |
| `repro-staticlight.js` | the reported pose with the light frozen (`CFG.staticLight`), so the field fully converges |
| `black-*.js` | the floor case, term by term (`debug` 2 indirect, 3 weights, 4 per cascade, 5 direct/indirect split, 9 albedo) |
| `sweep-room-height.js` | three camera heights looking at the floor, for camera-locked coverage steps |
| `lookdown-room.js` / `lookdown-simple.js` | straight down at the floor |
| `scroll-only.js` | static camera + static light with only the **scroll origin** drifting (`CFG.scrollDrift`): the only way a one-frame scroll event shows up in a still image |
| `fly-in.js` / `fly-in-noupdate.js` | the camera flying inside the lit room at 6 m/s with consecutive captures, and the same path with the probe update disabled (the difference between the two frame sequences is the field's) |
| `fly-trace.js` | the same flight with `CFG.fieldTrace`, which reads the radiance atlas back every frame and logs the per-frame delta and the per-cascade atlas mean |
| `cascade-only-0/1/2.js` | indirect from one cascade only at a fixed pose: compares the three levels |
| `cascade-blend.js` | the same pose with the real blended result, as the reference for those three |
| `debug-*.js` | diagnostic views, see below |

`CFG.cameras` replaces the whole preset list (so one run sweeps several views) and
`CFG.camera` replaces only the first preset. `CFG.autoFly` (m/s) drives the camera along +X
at a fixed height so a scroll lands on a known frame, `CFG.captureFrames` overrides the three
capture marks (consecutive frames are what a temporal A/B needs), `CFG.staticLight` freezes
the animated light, `CFG.scrollDrift` moves the scroll origin without moving the camera, and
`CFG.probeUpdates: false` leaves the atlas untouched so the probe passes' cost is separable
from the render's.

## Diagnostics

`CFG.debug` selects what the fragment writes (see the `DBG` uniform in
`render.wgsl.js`), `CFG.debugCascade` narrows it to one cascade:

| `debug` | View |
|---|---|
| 2 | indirect irradiance only (`E/(E+1)` for display) |
| 3 | cage weight sum |
| 4 | indirect from cascade `debugCascade` only |
| 6 | combined, but the cage **drops every probe outside the room AABB** |
| 7 | combined, cage drops probes **above the roof** |
| 8 | combined, cage drops probes **outside in x/z** |

Modes 6–8 are the leak attribution used in §7.6: with `sealed.js` the frame mean goes
45.0 → 10.6 (7), 12.8 (8) and 0.0 (6), which is what identifies cage interpolation of
outside probes as the entire leak.

## Checks

- `node prototype/probe-gi-demo/selftest.mjs` — CPU-only: BVH traversal vs brute
  force, cascade/slot/atlas indexing, scroll-band reseeding, and the **atlas mapping
  round trip** (the blend's texel directions against the render's fetch coordinate;
  this caught a half-texel mapping error).
- `prototype/gi-ref/` — the CPU-first harness that diffs each GPU stage against a
  hand-checked reference. Run `node prototype/gi-ref/test-cpu.mjs` first, then
  `prototype/gi-ref/run-gpu.sh`.
- `shadowcheck.mjs` — CPU reference for the shadow rays the fragment casts.
- `node prototype/probe-gi-demo/probecheck.mjs [scene] [spacings] [mode]` — the CPU
  reference for the *field*: it mirrors `probeTrace` and `sampleCascade` (including the
  Chebyshev gate, the backface rule and the cascade blend) so a hypothesis about a cage
  costs seconds instead of a shell launch. Reports the per-cascade probe sweep split by
  which side of the room shell each probe is on, the **swing** of a fixed world point over
  several camera positions, and the **floor profile** a moving query produces. Modes:
  `all` (default), `swing`, `detail` (per-probe terms for one cage). It is how the
  cascade-handover weight was shown to be the cause of the moving brightness step:
  `probeDir compl 2.5` gives a flat, honest near field (0.09-0.10) where `fade ramp 2.5`
  gives 0.56-0.64 against an honest in-room level of ~0.07.

## Probe seeding (easy to break)

A probe that gets reseeded by a scroll must not start from no history: the band would be a
one-frame value next to neighbours averaging ~14 frames, and with the light moving that
boundary is a visible step. `seedFrom` (a per-probe index the host fills in `updateScroll`)
names the inward neighbour one cell back along the scrolled axis; `probeHysteresis` then
returns the normal hysteresis and `previousTile` reads the neighbour's tile **while the
store still targets the probe's own tile**. Two traps: a seed pointing at a deactivated
probe reads a never-written tile (undefined texels → NaN → black frames), and no seed at
bootstrap is correct (there is nothing to inherit).

## Cascade blend and hysteresis (also easy to break)

Inside a cascade's box the **finer cascade must win**. Each cascade's own boundary fade is not
a share: a coarse cascade sits at fade ≈ 1 for a point deep inside the *fine* cascade's box,
so using `share = fade` gave the 9 m cascade 74 % of a floor point's weight one metre from the
camera — measured at E = 1.27 against the honest in-room 0.08 — which is what a lit region
that moves with the camera looks like on screen. The blend is
`fade × Π(1 − fade_finer)` now, so the shares sum to 1 and the loop stops as soon as the
remaining share is negligible.

`probeHysteresis` must return the normal hysteresis for **every** probe and 0 only for one
that was reseeded this frame with nothing to inherit. An earlier version tested the seed
source alone, which is `0xffffffff` for every ordinary probe, so the whole field ran with no
temporal accumulation at all (`git log`-free prototype: read the function's comment before
editing it). `blendDistance` is dispatched before `blendRadiance` because the latter clears
the fresh bit; keep that order.

`probeInit` deactivates a probe when 5 of its 8 validity rays hit **backfaces**, with no
distance limit: any backface means the ray left a solid. A limit of `spacing × 0.5` left 64 of
the near cascade's 256 probes sitting half a metre inside the ground slab, valid, with an
all-backface sweep — zero-radiance votes in every cage near the floor.

## Reading the frame

All ambient light arrives through the probes: there is no `skyRadiance()` ambient
stand-in in the fragment, on purpose. Forcing the probe term to zero changes ~55 % of
pixels, which is the check that GI is wired end to end.

**Frame-rate caveat:** with a second display attached the shell's present path pins to
60 Hz, and every ablation measured (constant fragment, 320×200 window, all compute
zeroed) reports the same 60.0 fps with `gpu` ≈ 42 ms — that field is present latency,
not per-frame cost. Compare fps only within one display configuration.
