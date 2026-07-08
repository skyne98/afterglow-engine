# How stable and usable is the Three.js WebGPU renderer?

**Date:** 2026-07-08
**Three.js version at time of writing:** r185 (released 2026-07-01)
**Verdict:** Usable now for *instanced / compute-shader-heavy* workloads with a
fallback story; **not yet a safe drop-in for many-draw-call / non-instanced
scenes**, and the API still churns every release. Treat as "early production,"
not "battle-tested."

---

## 1. The WebGPU platform itself is done

WebGPU hit **baseline in every major browser** by late 2025 / Jan 2026:

| Browser | Status |
|---|---|
| Chrome / Edge | Stable, on by default (Chrome 113+) |
| Firefox | Stable (141+) on Windows & macOS |
| Safari 26 (iOS + macOS Tahoe) | Stable, on by default |

The 15-year WebGL era is effectively over at the platform level. **Browser
support is no longer the blocker.**

Sources:
- https://vr.org/articles/webgpu-baseline-2026-three-js-webxr-default
- https://www.intelligentgraphicandcode.com/development/threejs-interfaces/webgpu

---

## 2. Three.js WebGPURenderer — the marketing line vs. reality

### The "production-ready" claim
Since **r171 (September 2025)**, Three.js ships `WebGPURenderer` with
**zero-config import** and **automatic WebGL2 fallback**:

```js
import * as THREE from 'three/webgpu';
const renderer = new THREE.WebGPURenderer(); // WebGPU if available, else WebGL2
```

Numerous consultancy/blog posts (utsubo, altersquare, ravespace) call this
"production-ready." That is **overstated** for the general case.

### The cautious view (more accurate)
**Threlte** (the major Svelte + Three.js framework) explicitly states in their
docs (accessed 2026-07):

> "The WebGPU specification is still in active development. WebGPU support in
> Three.js is in an early stage and is subject to frequent breaking changes.
> **As of now, we do not recommend using WebGPU in production.** We highly
> recommend targeting version r171 onwards because of potential duplication
> and configuration issues."

Source: https://threlte.xyz/docs/learn/advanced/webgpu/

### API churn is real and ongoing
Every recent release removes deprecated code, deprecates methods, and refactors
the node-material system. Sample churn from r181 → r185:

- **r181** — deprecated `renderAsync()`, `computeAsync()` and related async
  methods (you now `await renderer.init()`).
- **r183** — removed deprecated code; deprecated module; introduced TSL spec.
- **r184** — **removed deprecated instancing render paths** (breaking for
  anyone on old instancing APIs).
- **r185** — deprecated `.scale()`, `.rotate()`, `.translate()`; refactored to
  "native node material hooks"; removed `modInt` from TSL exports.

There is a formal Migration Guide per release
(https://github.com/mrdoob/three.js/wiki/Migration-Guide). Plan to read it on
**every** upgrade. Pin your version.

### Integration friction (Threlte's notes)
- `three` and `three/webgpu` **don't mix well** — importing both inflates
  bundle size and you must `extend()` node-material classes (`MeshPhysicalNodeMaterial`,
  lights, etc.) separately. For an engine, **import only from `three/webgpu`**.
- WebGPU uses top-level async; Vite needs `build.target: 'esnext'` +
  `optimizeDeps.esbuildOptions.target: 'esnext'`, or `vite-plugin-top-level-await`.

---

## 3. The big blocker: UBO performance with many draw calls

### Issue #30560 (OPEN, **High priority**, Feb 2025 → last updated Mar 2026)
"WebGPURenderer: Current UBO system has severe performance issues with many
render items."
https://github.com/mrdoob/three.js/issues/30560

**Root cause:** `WebGPURenderer` allocates a **separate UBO per object** for
object-scope uniforms (e.g. world matrix). With thousands of non-instanced
meshes, the per-object UBO upload/bind overhead dominates. `WebGLRenderer`
historically updated uniforms via `uniformMatrix4fv()` with no UBO, so it's
faster in this specific regime.

**Measured regression** (M1 Pro, Chrome, r173 / reconfirmed r181):

| Scene | WebGLRenderer | WebGPURenderer | forceWebGL fallback |
|---|---|---|---|
| 5k cubes | 350 fps | 140 fps | 50 fps |
| 10k cubes | 130 fps | 60 fps | 14 fps |
| 50k cubes | 40 fps | 3–6 fps | 1–2 fps |

So WebGPU is **~2.5–4× slower** than WebGL for many non-instanced meshes, and
the `forceWebGL: true` fallback is **even worse** (5–10× slower than native
`WebGLRenderer`) — it is *not* a perf-equivalent fallback.

### Maintainer status (Mugen87, Nov 2025)
- Acknowledged as a real issue: *"It is definitely an issue when developers
  migrate to WebGPURenderer and experience noticeable performance degradation…
  the renderer must handle this in a performant fashion."*
- Planned fix: refactor to batch object-scope uniforms into shared storage
  buffers, using #27388 as a starting point. Requires an extra render-list
  iteration to build nodes before processing render items. **Not yet landed in
  r185.**

### Workarounds (available now)
- **Use instancing** (`InstancedMesh`) wherever possible — sidesteps the
  per-object UBO path entirely and WebGPU shines.
- **Manually use storage buffers (SBOs)** for bulk uniform data and update
  selectively from the CPU (see Spiri0's prototypes; `device.queue.writeBuffer`).
- **GPU frustum culling / render bundles** (toji.dev best practices) for very
  large object counts.

### The flip side — where WebGPU wins
For **compute-shader-heavy** or **instanced** workloads, WebGPU is a clear win:
- Segments.ai reported a **100× speedup** moving their LiDAR tool to WebGPU
  (compute-bound).
- WebGPU enables TSL compute, storage buffers, storage textures, indirect
  draws — things WebGL simply can't do.

---

## 4. Other open issues to watch (60 open / 618 closed mentioning WebGPU)

- **#33821** — `WebGPURenderer: Material initialization is extremely slow
  compared to WebGLRenderer` (startup / first-frame cost).
- **#33795** — `StorageTexture` compute writes not visible to `RenderPipeline`
  `texture()` bindings in the same frame (sync hazard).
- **#32969** — TSL: missing features, memory leaks, unexpected behavior.
- **#30725** — WebGPU Compatibility Mode not supported.
- **#33559** — CI: tests are not consistently running with WebGPU (tooling gap).
- **#26673** — umbrella "WebGPURenderer: Increase performance" tracker.

Issue tracker search:
`https://github.com/mrdoob/three.js/issues?q=is:issue+webgpu`

---

## 5. TSL (Three Shading Language)

TSL is the node-based shader authoring layer that replaces hand-written GLSL
for the WebGPU path. Status:
- A **TSL spec** (`TSL.md`) was introduced in r183.
- Still has rough edges: `toVar` inside `IF` triggers false warnings (#33838);
  separable blurs / map-driven blurs still being built (#32401).
- **For an engine:** TSL is the forward path — raw GLSL does not work on the
  WebGPU backend. Committing to WebGPU means committing to TSL (or WGSL via
  raw WebGPU, bypassing Three.js materials).

---

## 6. Recommendations for afterglow-engine

1. **Adopt `three/webgpu` as the renderer**, but pin a specific version and
   treat upgrades as breaking until the UBO refactor lands.
2. **Architect around instancing from day one.** Non-instanced "one mesh per
   entity" scenes will hit the #30560 wall. Use `InstancedMesh` / GPU-driven
   drawIndirect for anything with many instances.
3. **Plan for the batched-uniforms workaround** (storage buffers) if the
   engine needs many distinct materials/objects — don't assume the generic
   renderer path is fast yet.
4. **Do not rely on `forceWebGL: true` as a perf fallback** — it's slower than
   native `WebGLRenderer`. If a WebGPU-free path is needed, ship a true
   `WebGLRenderer` build separately, or accept WebGL2-only via the auto-fallback
   for *correctness* only.
5. **Use TSL** for all shading; don't write GLSL that would block the WebGPU
   path.
6. **Track #30560** as the gating issue for "WebGPU is universally faster than
   WebGL." Re-evaluate each release.
7. **Budget for API churn** — read the Migration Guide every release, keep a
   regression test suite for rendering.

---

## Sources

- Three.js official `WebGPURenderer` docs: https://threejs.org/docs/pages/WebGPURenderer.html
- Three.js releases: https://github.com/mrdoob/three.js/releases (r181–r185 notes)
- Migration Guide: https://github.com/mrdoob/three.js/wiki/Migration-Guide
- Issue #30560 (UBO perf): https://github.com/mrdoob/three.js/issues/30560
- Issue #33821 (slow material init): https://github.com/mrdoob/three.js/issues/33821
- Issue #33795 (storage texture sync): https://github.com/mrdoob/three.js/issues/33795
- Forum: WebGPU performance issue (real-world benchmark): https://discourse.threejs.org/t/webgpu-performance-issue/87939
- Threlte "WebGPU and TSL" guidance: https://threlte.xyz/docs/learn/advanced/webgpu/
- WebGPU baseline 2026: https://vr.org/articles/webgpu-baseline-2026-three-js-webxr-default
- IGC migration/architecture writeup: https://www.intelligentgraphicandcode.com/development/threejs-interfaces/webgpu
- WebGPU optimization fundamentals (greggman): https://webgpufundamentals.org/webgpu/lessons/webgpu-optimization.html
- Render bundles / GPU frustum culling (toji): https://toji.dev/webgpu-best-practices/render-bundles
