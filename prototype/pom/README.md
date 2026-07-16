# POM prototype — super-cheap parallax occlusion mapping

Evaluation prototype for `docs/research/surface-detail-low-end-fallbacks.md`.
Renders a close, screen-filling brick wall through **Three.js WebGPU (TSL
shaders) inside `afterglow-cef`** and benchmarks three surface-detail tiers
plus an optional contact-shadow variant:

| Key | Tier | Cost |
|-----|------|------|
| `1` | Normal map only (universal minimum) | 0 height fetches |
| `2` | Offset-limited one-tap parallax (GPU Gems 2) | 1 height fetch |
| `3` | Low-core POM (8–32 layers, no silhouette/refine) | N height fetches |
| `4` | POM + light-direction contact shadow | N + 2 height fetches |

The wall is mostly front-facing with a slow rotation, so the workload is
**fragment-bound** — exactly the regime the research identifies as POM's
bottleneck on the Radeon 680M.

## Why this is WGSL-valid

A POM relief march with a data-dependent `Break` is **non-uniform control
flow**. WGSL forbids `textureSample` (TSL `texture()`) there — so the march
uses `textureLevel()` which lowers to `textureSampleLevel` (explicit LOD),
allowed in non-uniform flow. The adaptive POM layer count is a bounded runtime
WGSL loop; the optional shadow uses a fixed two-sample loop.
`parallaxDirection` (the tangent-space view dir = `positionViewDirection.mul
TBNViewMatrix`) is the expensive TBN transform, taken from TSL directly.

## Build the scene

```sh
cd ~/dev/afterglow-engine
bun build prototype/pom/pom.ts --outfile prototype/pom/pom.js --target browser
```

(`engine-bundle.js` is loaded as a global at runtime from
`crates/afterglow-web/www/`; `pom.ts` imports the engine's `webgpu-only.ts`
and `bench.ts` via `.ts` specifiers, per AGENTS.md.)

### Key implementation choices

- **No MSAA** — the single biggest lever for a fragment-bound POM (4× fewer
  shaded samples).
- **Raw WGSL march via `wgslFn`** — loop-carried `var` state is natural in WGSL;
  the TSL `If`-gated assignment forms a cyclic node graph (`curUV → h =
  sample(curUV) → curUV`) that TSL rejects as "Recursion detected".
- **`textureSampleLevel`** (explicit LOD 0) in the march — `textureSample` /
  TSL `texture()` is forbidden in non-uniform control flow; the data-dependent
  intersection test is non-uniform.
- **View-angle-adaptive layer count** — `numLayers = mix(maxLayers, minLayers,
  |v.z|)` computed in-shader, so head-on stays cheap (16 layers) and grazing
  rises to 64 to kill discrete banding. The loop bound is runtime (not
  unrollable); head-on is still vsync-bound, and the grazing wall covers fewer
  pixels at edge-on, so both stay within budget.
- **`viewDir = parallaxDirection.negate()`** — Three's exported `parallaxDirection`
  (`positionViewDirection.mul(TBNViewMatrix)`) is camera→fragment in tangent
  space; the canonical march needs fragment→camera, so negate it. Verified at
  runtime by reading the debug-material pixel from the live CEF page (the agent
  cannot view images, so the direction was encoded as RGB and read as numeric
  pixel values via CDP).

### Occlusion and shadows

- **Self-occlusion (view direction)** — this *is* what POM does: the relief
  march finds the first surface point along the view ray that occludes what's
  behind it (a closer brick ridge hides the mortar behind).
- **Self-shadowing (light direction, mode 4)** — the POM hit point casts a
  two-sample ray *toward the directional light* in tangent space. Brick=1 is
  physical raised height and mortar≈0.16 is recessed height; a sampled terrain
  height above the upward ray blocks it. A 0.035 bias prevents a bilinear sample
  from shadowing itself. The result is binary visibility applied to Three's
  **direct diffuse and direct specular** terms at default strength 0.72;
  ambient and indirect fill remain unshadowed. This is deliberately a cheap
  **contact** shadow, not a long soft
  shadow; press `S` to toggle it or use `window.pomSetShadowStrength(0..1)`.
- **Edge discard** (LearnOpenGL's border fix) is the documented next step for
  border-oversampling artifacts if they appear on non-centered geometry.

## Build + launch (CEF)

```sh
cd ~/dev/afterglow-engine
nix-shell shell.nix --run "cargo build --example pom_bench -p afterglow-cef"
XA=$(ls /run/user/1000/.mutter-Xwaylandauth.* | head -1)
setsid env DISPLAY=:0 XAUTHORITY="$XA" nix-shell shell.nix --run \
  "./target/debug/examples/pom_bench --ozone-platform=x11" \
  </dev/null >/tmp/cef-pom.log 2>&1 &
```

DevTools on port **9222**. In the window press **B** (300-frame bench) or load
`?bench=300` to auto-run. Results appear in the top-right HUD and the JS console
(`[bench]` prefix).

## Prove hardware WebGPU (never accept WebGL fallback)

```sh
./target/debug/latency-tool eval \
  '(async()=>{const a=await navigator.gpu.requestAdapter();return JSON.stringify(a&&a.info)})()' \
  127.0.0.1:9222
! grep -E 'GPU process exited|WebGPU is not available|WebGL2 backend' /tmp/cef-pom.log
```

A null / non-AMD adapter, `GPU process exited unexpectedly`, or "running under
WebGL2 backend" is a **failed run** — fix the Vulkan stack before trusting any
FPS number (see AGENTS.md → "fox-laptop CEF/WebGPU validation").

## Headless / OLED-safe 300-frame benchmark via CDP

```sh
./target/debug/latency-tool eval \
  '(async()=>{const f=[];let p=-1;await new Promise(r=>{function l(t){if(p>=0)f.push(t-p);p=t;if(f.length<300)requestAnimationFrame(l);else r()}requestAnimationFrame(l)});const s=[...f].sort((a,b)=>a-b);return JSON.stringify({n:f.length,fps:(1000/(s.reduce((q,v)=>q+v,0)/s.length)).toFixed(1),p99:s[s.length*0.99|0].toFixed(2),max:s[s.length-1].toFixed(2),below55:f.filter(x=>x>1000/55).length})})()' \
  127.0.0.1:9222
```

## Measured results (2026-07-16, fox-laptop)

Validated stack: WebGPU-only (no WebGL fallback), adapter `vendor=amd`
`architecture=rdna-2` (Radeon 680M), `--ozone-platform=x11`, Nix Vulkan
loader + Mesa RADV (per AGENTS.md). Window 1440×900 logical at DPR 2 →
2880×1800 physical, vsync on, **no MSAA** (`antialias:false` — MSAA multiplies
a fragment-bound POM shader 4×; with MSAA on, POM-16 dropped 16–19/300).

The POM march is the **canonical LearnOpenGL / GPU Gems** relief march
(first-intersection + occlusion interpolation) with a **view-angle-adaptive layer
count**: few layers head-on (`|v.z|~1`, cheap), many at grazing (`|v.z|~0`, up
to 64) — the industry-standard fix for the discrete banding a fixed layer
count produces at steep angles. Press **V** (or the `∠ hard angle` button) to
snap to a near-edge-on grazing view. Press **F** to freeze the rotation for
inspection.

Verified against the canonical references (LearnOpenGL `Parallax Mapping`
tutorial + bentoBAUX `Parallax Mapping with Self Shadowing`): the POM march,
adaptive layer count, occlusion-interpolation formula, and explicit-LOD
sampling in non-uniform loops match the standard implementations. The optional
shadow ray uses the corresponding physical-height convention: it advances UV
**toward light**, rises in height, and blocks when a taller terrain sample
intersects it. Direct `window.pomSetMode()` hooks—not synthetic keyboard
events—make the CEF visual A/B capture deterministic.

300-frame rAF-timing benchmark via CDP (`latency-tool eval`), clean process
(no second `requestDevice()` — a transient second device contends with the
renderer and regresses the measurement). The session must be **unlocked**
(GNOME screensaver active ⇒ rAF throttles to ~1 Hz; `gdbus … ScreenSaver
.SetActive false` unblanks when `LockedHint=no`).

| View / tier | avg FPS | p99 | drops (<60fps) |
|------|--------:|------:|----------------:|
| Orbit, normal map (tier 0) | 60.0 | 16.68 ms | 0/300 |
| Orbit, one-tap parallax (tier 1) | 60.0 | 16.68 ms | 0/300 |
| **Orbit, POM 16 (adaptive, tier 2)** | **60.0** | **16.68 ms** | **0/300** |
| Hard-angle (edge-on), POM 16 (adaptive) | 60.0 | 16.68 ms | 0/300 |
| Orbit, POM 16 + 2-sample contact shadow (strength 0.72) | 59.18 | 16.68 ms | 5/600 |

**Target met for the low-core POM tiers:** every tier holds ~60 FPS, p99 within
the 16.68 ms vsync budget, **0 / 300** dropped frames — including the
hard-angle grazing view, where the adaptive layer count rises to 64 to remove
banding but the wall covers fewer screen pixels at edge-on, so cost stays
within budget. The optional contact-shadow capture held p99 but had 5 isolated
frames below 55 FPS in 600 (max 50.03 ms); it is visually verified and bounded,
but is not claimed as a strict zero-drop tier. This reproduces/proves the
research's low-core-POM verdict (`docs/research/surface-detail-low-end-fallbacks.md`)
in the real CEF/WebGPU stack. Tune live with `←/→` (head-on POM layers; the
grazing max auto-scales to 4×, capped at 96) and `↑/↓` (height scale).
