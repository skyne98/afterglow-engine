# Servo + Three.js status deep-dive

**Date:** 2026-07-08
**Question:** Can Servo run Three.js — specifically the **WebGPU renderer** — well
enough to use as the engine's native shell? (Servo would be the ideal shell:
Rust-native, lightweight, has WebGPU, embeddable. This checks whether it's
actually usable.)

**Verdict: No.** Three.js does **not** run reliably on Servo today — not the
WebGL path (years-old unfixed rendering/perf bugs) and **especially not the
WebGPU path** (Servo's WebGPU is incomplete: missing methods, crashes,
undefined behavior, broken conformance-test infra; plus missing CSS/layout
like `aspect-ratio` and `ResizeObserver` gaps). A real WebGPU game (SpookyBall)
does not run on Servo as of June 2026. Nobody has even reported the Three.js
*WebGPU* renderer on Servo (0 issues). Servo is the right aspiration but is
**not a viable Three.js-WebGPU shell today**; revisit in ~2 years.

---

## 1. Servo itself is healthy and serious

- **37k★**, pushed today (2026-07-08), ~3,150 open issues, MPL-2.0.
- Governed by **Linux Foundation Europe**; sponsored; monthly blog releases.
- **v0.1.0 on crates.io** (2026-04-13) — first release of the `servo` *library*
  crate (embeddable). Explicitly **not 1.0**: *"we still haven't finished
  discussing what 1.0 means for Servo… breaking changes in the regular monthly
  releases are expected."* An LTS channel exists for embedders.
- Cross-platform: Windows, macOS, Linux, Android, OpenHarmony, FreeBSD.
- Embedding API ("WebView API") exists and is maturing, but pre-1.0.

So Servo-the-project is vibrant. The question is purely web-platform + WebGPU
completeness for Three.js.

---

## 2. Three.js on Servo: tested, but broken (WebGL), untested (WebGPU)

Servo's team **does** track Three.js — **168 issues** mention it. But:

- **All are about the WebGL renderer** examples (canvas, css3d, shadowmap,
  postprocessing, raymarching, draco loader, points, etc.).
- **Zero issues combine "three.js" + "webgpu"** — nobody has reported the
  Three.js **WebGPU renderer** running on Servo. It is untested/unreported.
- The WebGL issues are **years old (2018–2019) and mostly still open**,
  some updated as recently as 2026-06. Long-standing, unresolved:
  - `#20999` three.js css3d orthographic "only shows wireframe" (open, 2018, updated 2026-06-09)
  - `#21012` three.js canvas sandbox "renders incorrectly" (open, 2018)
  - `#20995` three.js shadowmap pcss "performs poorly" (open, 2018, updated 2025-09)
  - `#21011` three.js canvas performance "performs poorly" (open)
  - `#21003` three.js canvas ascii effect "segfaults when dragging" (open)
  - `#20980` three.js points dynamic "has JS error" (open)
  - `#20946` three.js draco loader "has JS error" (open)
  - `#24824` three.js camera movement "can get stuck" (open, 2019)
  - `#25993` meta: "Evaluate three.js performance benchmarks" (open, 2020, stale)

**Takeaway:** even the *mature WebGL* Three.js path has years-old unfixed
rendering, performance, and JS-error bugs on Servo. The WebGPU path is
untested — and, given §3, almost certainly non-functional.

---

## 3. WebGPU in Servo: under heavy active development, but incomplete

Servo's WebGPU is implemented via **wgpu** (the same Rust WebGPU crate Firefox/
others use). It is being actively worked — **188 issues** with "webgpu" in the
title, many from 2026. But large swathes are still being implemented:

**Still-implementing (open, 2026):**
- `#46178` webgpu: Update to wgpu v30
- `#46286` Support `GPUTextureUsage::TRANSIENT_ATTACHMENT` + `GPUCanvasContext::getConfiguration`
- `#45129` webgpu: Implement immediates
- `#24706` umbrella "Implement WebGPU in Servo" (open since 2019; original checklist left resource creation, pipeline creation, command interfaces, fences unchecked)
- `#45303` Split WebGPU into its own crate (ongoing refactor)

**Recently landed (closed):** `GPUExternalTexture` (#45873), `QuerySet` (#45644),
`copyExternalImageToTexture` (#45646), `GPUDebugCommandsMixin` (#45489). So
core pieces were still missing until very recently.

**Correctness/stability bugs (open):**
- `#45237` webgpu: **UB** (undefined behavior) in `DataBlock::view`
- `#44969` webgpu: enforced memory limits **leads to crashes**
- `#45504` webgpu: Investigate proper cleanup in `GPUBuffer`

**Conformance test suite (CTS) infrastructure broken:**
- `#34708` WebGPU CTS **killed when run with python 3.12** (in CI) — open
- `#29829` Standalone webgpu cts fails with **"Too many open files"** — open
- `#30999` split variants in WebGPU's `cts.html` for **less timeouts** — open

So Servo can't even reliably *run* the WebGPU conformance tests end-to-end.

---

## 4. Smoking gun: a real WebGPU game doesn't run (June 2026)

**`#45474` "webgpu: Can't play SpookyBall in servo"** (open, created 2026-06-08,
updated 2026-06-09). SpookyBall (`toji/spookyball`, a WebGPU game by Toji — a
well-known WebGPU dev) *"works fine in Firefox, but doesn't work in servo."*
Open sub-tasks:

- [ ] Implement `pushDebugGroup` (and friends) — **missing WebGPU method**
- [x] Crash on macOS (Metal) — fixed
- [ ] The game renders much too small — **missing CSS `aspect-ratio`** (a stylo/layout bug!)
- [x] Paddle flickers wildly at low FPS — fixed in the game's JS
- [ ] `resize-observer` `device-pixel-content-box` should return device pixels — **missing ResizeObserver feature**

This is the most direct evidence available: a real, shipping WebGPU game fails
on Servo due to **both** missing WebGPU API surface **and** missing CSS/layout/
observer features. Three.js WebGPU uses a *much* broader slice of WebGPU than
SpookyBall does, so it is extremely unlikely to fare better.

---

## 5. Why Three.js WebGPU specifically would fail

Three.js `WebGPURenderer` (r171+) exercises essentially the *entire* WebGPU
surface: storage buffers/textures, samplers, bind groups, pipeline layouts,
shader modules (WGSL via TSL), render + compute pipelines, command encoders,
queues, `QuerySet`, `copyExternalImageToTexture`, `GPUExternalTexture`,
debug groups, canvas context configuration, and more. Servo is **still
implementing** several of these (TRANSIENT_ATTACHMENT, getConfiguration,
immediates; only just landed QuerySet/ExternalTexture/copyExternalImageToTexture
in 2026), has open UB/crash bugs, and can't run its CTS cleanly. Combined with
the missing CSS `aspect-ratio` and `ResizeObserver` gaps that break even
SpookyBall, Three.js WebGPU will not run.

---

## 6. Conclusion & recommendation for afterglow-engine

- **Servo is not a viable shell for a Three.js-WebGPU engine today.** Both the
  rendering API (WebGPU incomplete, crashes, UB, broken CTS) and the surrounding
  web platform (CSS `aspect-ratio`, `ResizeObserver` gaps) are insufficient.
- It is the right **long-term aspiration** (Rust, lightweight, WebGPU via wgpu,
  embeddable) and the project is healthy and well-funded. But realistic
  timelines: WebGPU conformance passing + Three.js WebGL examples green + a
  Three.js WebGPU report that works = **~2+ years out**.
- **Don't bet the engine on Servo now.** If you want a Rust-native, lightweight,
  WebGPU shell, the only honest path is to revisit Servo annually with a Three.js
  WebGPU spike. Meanwhile, for shipping, use Electron (proven) or Tauri (accept
  WebGL2 on Linux) per the prior research notes.
- The embedding API is also pre-1.0 with expected monthly breaking changes —
  another reason not to build a product on it yet.

**Re-evaluate trigger:** when Servo's WebGPU CTS runs cleanly in CI and
SpookyBall (or any non-trivial WebGPU game) runs without WebGPU/CSS workarounds
on Linux. That's the signal WebGPU is "real" on Servo.

---

## Sources

- Servo homepage: https://servo.org/
- Servo v0.1.0 crates.io release (pre-1.0, breaking changes expected): https://servo.org/blog/2026/04/13/servo-0.1.0-release/
- WebGPU umbrella issue #24706 (open since 2019): https://github.com/servo/servo/issues/24706
- WebGPU CTS infra broken: #34708, #29829, #30999
- WebGPU UB/crashes: #45237 (UB), #44969 (crashes), #45504 (buffer cleanup)
- Still-implementing: #46178 (wgpu v30), #46286 (TRANSIENT_ATTACHMENT), #45129 (immediates)
- **SpookyBall WebGPU game doesn't run** (smoking gun): https://github.com/servo/servo/issues/45474
- Three.js WebGL issues (years-old, open): #20999, #21012, #20995, #21011, #21003, #20980, #20946, #24824, #25993
- Three.js + WebGPU combined search in Servo issues: **0 results**
