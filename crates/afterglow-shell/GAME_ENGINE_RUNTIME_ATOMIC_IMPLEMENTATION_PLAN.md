# Game Engine Runtime — Atomic Complete Implementation Plan

## Implementation status

The atomic cutover is active in the product tree: the clear-color/dual-device
spike and obsolete CDP/probe executables are deleted; `src/main.rs` runs real
three.js through a shared deno_webgpu/wgpu-core native surface with no full-frame
readback; lifecycle/readiness/clock modules are library-owned; the browser test
executable is a thin wrapper around `src/testing/browser_runner.rs`; queue-submit
readiness removes both former no-canvas races; native resize/input, adapter
reporting, device-loss termination, suspend/resume, and configurable windowed
game-module loading are wired. The default `native_game.ts` uses unmodified
three.js and has been verified from Xvfb surface captures. The production
LinkeDOM/Blitz HUD now emits a Vello scene that is rasterized directly into a
persistent texture by the shared GPU device and composited after the game pass;
there is no production HUD pixel raster or upload on the CPU. Native hit testing reaches both HUD controls and the game
canvas. Same-adapter Chrome/NVIDIA/Vulkan diagnostics now isolate bundled-golden
adapter differences for the core GPU set, and runtime-native goldens are pinned.
Remaining work is narrower per-attachment numeric analysis, not a second runtime
or HUD path.

## 1. Objective

Turn the current screenshot-oriented WebGPU/browser experiment into one production game-engine runtime that executes unmodified three.js WebGPU applications in a native window with:

- one authoritative JavaScript runtime and browser environment;
- one authoritative WebGPU device/queue ownership model;
- direct native presentation without full-frame GPU readback;
- correct render, compute, texture, depth, blending, MSAA, and postprocessing behavior;
- deterministic asset loading and test execution;
- continuous production timing and a separate deterministic test clock;
- native pointer, keyboard, focus, resize, and window lifecycle routing;
- usable DOM/CSS HUD and editor UI;
- explicit readiness, device-loss, error, and diagnostics state.

This is an **atomic product cutover**. Work may be implemented in dependency order on one branch, but the final tree has no old spike host, alternate renderer, compatibility fallback, hidden feature flag, client-source patch, example-specific behavior, or partial stub.

All example HTML, module code, and `three/addons/*` continue to run verbatim. Missing functionality is implemented in the host, JavaScript browser environment, vendored deno_webgpu/wgpu patches, or Blitz integration.

## 2. Runtime profile

### 2.1 Required product features

1. Reliable adapter/device/renderer startup and recovery.
2. Direct native frame presentation.
3. Complete WebGPU render and compute operation coverage needed by three.js.
4. Correct texture formats, mip levels, filtering, anisotropy, copies, and color spaces.
5. Correct MSAA, depth/stencil, blending, clipping, shadows, and render targets.
6. Correct compute ordering, barriers, atomics, reductions, and render/compute handoff.
7. Stable PBR lighting, environment maps, transmission/refraction, and volumetrics.
8. Stable postprocessing and temporal-history initialization.
9. Deterministic file/data/blob/HTTPS resources and worker decoding.
10. Pointer, mouse, wheel, keyboard, focus, pointer capture, and resize routing.
11. Basic browser-quality HUD/editor layout, hit testing, scrolling, observers, and readable text.
12. Production and deterministic frame clocks.
13. Structured GPU, JS, resource, and frame diagnostics.

### 2.2 Explicit non-goals

These are not implemented as part of this cutover:

- browser tabs, history, cross-document navigation, or full form submission;
- cookies, service workers, cache storage, permissions, or browser security UI;
- full accessibility-tree parity;
- byte-identical Chrome/Skia glyph antialiasing;
- WebCodecs and `HTMLVideoElement` playback unless video becomes a product requirement;
- exact Chrome first-frame pixels for temporal effects when the runtime is stable and visually correct;
- arbitrary multi-page browser behavior.

Non-goals are not represented by silent no-op APIs. An API is either correctly implemented for the supported profile, absent, or fails explicitly with a typed unsupported-capability error.

## 3. Non-negotiable invariants

1. Never modify examples or addons.
2. Never intercept an import to return a fake implementation.
3. Never use sleeps as renderer/device/resource readiness.
4. Never read back the full game frame to CPU in the production presentation path.
5. Never maintain separate unrelated wgpu devices for JavaScript rendering and native presentation.
6. Never classify a large visual difference as harmless without a buffer/texture/shader-level cause.
7. Small floating-point differences require a documented tolerance; state/order defects require fixes.
8. Every exposed host API has real state transitions and error behavior.
9. Production and test timing share the same scheduler implementation; only the clock source differs.
10. Device loss, surface loss, out-of-memory, shader errors, rejected promises, and resource failures are observable.
11. Diagnostics used to find a defect are removed or converted into a deliberate diagnostics facility.
12. One unconditional runtime path is used by the product host and test harness.

## 4. Acceptance criteria

The cutover is complete only when all of the following hold:

- `src/main.rs` runs a real unmodified three.js WebGPU game in a resizable winit window.
- The game canvas renders through the same deno_webgpu device used by presentation.
- No production frame performs canvas-to-CPU-to-surface round trips.
- Resize, minimize/restore, scale-factor change, focus, pointer, wheel, and keyboard events work.
- Renderer startup cannot reach the first frame before adapter, device, canvas configuration, pipelines, and required resources are ready.
- Device loss transitions through a defined state and either recovers or terminates with a structured error.
- Core GPU conformance tests pass on Lavapipe and NVIDIA.
- All non-video examples complete without runtime errors in a serial 182-example run.
- Core renderer examples meet the strict differential target or have an approved runtime-native deterministic golden where Chrome identity is not meaningful.
- Two deterministic runs produce identical runtime-native output hashes.
- There are no example-name checks, client rewrites, stub modules, fallback hosts, or debug-only behavior changes.

## 5. Final source architecture

Replace the spike and example-local host duplication with library-owned subsystems:

```text
src/
  main.rs                     production winit entry point only
  runtime/
    mod.rs                    EngineRuntime orchestration
    lifecycle.rs              Created/Loading/Ready/Running/Suspended/Lost/Stopped
    javascript.rs             JsRuntime creation, snapshots, module evaluation, rejection drain
    scheduler.rs              rAF, timers, tasks, production/test clock sources
    readiness.rs              typed async readiness registry
    errors.rs                 structured runtime errors and fatal policy
  gpu/
    mod.rs                    shared deno_webgpu instance/device ownership
    adapter.rs                feature/limit negotiation and capability report
    canvas.rs                 game/offscreen canvas registry
    presenter.rs              native surface acquisition and final GPU composition
    compositor.rs             game texture + DOM/UI texture + color conversion
    diagnostics.rs            labels, error scopes, timestamp queries, optional captures
    recovery.rs               device/surface loss state transitions
  assets/
    mod.rs                    URL resolution and request lifecycle
    provider.rs               file/data/blob/HTTPS provider
    images.rs                 image decode and ImageBitmap
    compressed.rs             KTX2/Draco worker decode
    fonts.rs                  deterministic UI fonts
    accounting.rs             readiness and failure accounting
  browser/
    protocol.rs               LinkeDOM ↔ Blitz records and live state
    document.rs               long-lived reconciliation
    style.rs                  Stylo and CSSOM
    geometry.rs               rects/metrics/hits
    interaction.rs            native event translation and defaults
    observers.rs              resize/intersection/media delivery
    paint.rs                  HUD/editor raster production
  testing/
    deterministic.rs          fixed clock/RNG/frame policy
    capture.rs                GPU capture and image output
    differential.rs           manifests, tolerances, classifications
    probes.rs                 buffer/texture/shader conformance probes
```

JavaScript is split into deliberate environment modules:

```text
browser_bridge/
  bootstrap.js
  dom.js
  events.js
  canvas.js
  assets.js
  workers.js
  scheduler.js
  diagnostics.js
```

`examples/browser_test.rs` becomes a thin consumer of the library runtime. It no longer owns an independent browser, scheduler, canvas, or readiness implementation.

## 6. Atomic implementation sequence

The steps below are implementation order, not separately shippable product phases. The old path remains untouched only while the replacement is incomplete; the final commit switches all consumers and deletes the old path in the same change.

### 6.1 Freeze the baseline and define test classes

1. Save the current 182-example manifest, output images, hashes, adapter report, and logs.
2. Re-run every current failure at least three times to distinguish deterministic defects from lifecycle races.
3. Define three test classes:
   - **Core GPU differential:** strict Chrome comparison for non-temporal GPU correctness.
   - **Runtime visual regression:** deterministic native golden for temporal effects and text/UI.
   - **Optional browser feature:** explicit exclusion for video/navigation/full browser behavior.
4. Add machine-readable classification and tolerance files. Tolerances are per test class, never arbitrary per example.
5. Add microtests for every major GPU feature so screenshot differences can be localized.

### 6.2 Replace the spike with one production runtime

1. Move JsRuntime construction, snapshots, extensions, module loading, browser setup, WebGPU setup, and rejection handling from `examples/browser_test.rs` into `src/runtime`.
2. Replace `src/main.rs`'s direct wgpu-24 clear-color spike with `EngineRuntime`.
3. Remove the independent direct `wgpu = 24` presentation device. Use the same wgpu-core/deno_webgpu generation and instance as JavaScript WebGPU.
4. Make both production and tests construct `EngineRuntimeConfig`; differences are windowed/headless presentation, clock source, and capture policy only.
5. Load the selected game URL/module through the normal module loader without rewriting it.

### 6.3 Implement a typed lifecycle and readiness barrier

Use a state machine:

```text
Created → EnvironmentReady → AdapterReady → DeviceReady → CanvasReady
       → ResourcesReady → RendererReady → Running
       → Suspended | DeviceLost | Stopped
```

1. Replace `__pendingGPURequests`, inferred animation-frame readiness, and timeout-based release with native readiness tokens.
2. Instrument adapter request, device request, canvas configuration, pipeline compilation, and resource decode at the host boundary.
3. A token records subsystem, operation, source URL/stack, start time, completion, and failure.
4. `RendererReady` requires a configured game canvas and at least one successfully submitted/presentable frame, not merely module completion.
5. Capture and production presentation reject calls before `RendererReady` with a structured state error.
6. Queue asynchronous errors and unhandled rejections until their original stack can be reported.
7. Implement surface suspend on zero size and deterministic reconfigure on restore.
8. Implement device-loss notification, cancellation of pending work, resource invalidation, and explicit recovery/termination policy.

This directly removes the `lights_dynamic` and `tsl_vfx_flames` no-canvas races.

### 6.4 Implement direct GPU presentation and composition

1. Extend the vendored deno_webgpu canvas layer with a game render-target mode backed by the shared native device, not `DynamicImage`.
2. three.js renders into a GPU texture owned by the canvas context.
3. Create the winit surface through the same deno_webgpu/wgpu-core instance.
4. At presentation:
   - acquire the surface texture;
   - render/copy the game texture into the destination with explicit color conversion;
   - upload only changed DOM/HUD raster regions, not the full game frame;
   - composite below-game UI, game content, and above-game UI in paint order;
   - submit once and present once.
5. Preserve a headless GPU texture path for tests. Readback occurs only when a test capture or explicit diagnostic capture is requested.
6. Handle sRGB/linear formats, HDR capability negotiation, premultiplied alpha, opaque window output, and platform surface formats explicitly.
7. Add dirty rectangles or texture-atlas updates for CPU-rendered Blitz HUD content.
8. Keep the existing exact canvas command for headless browser differential tests, but do not use CPU full-page composition in the production game-frame path.

### 6.5 Establish GPU feature and limit correctness

1. Emit a capability report containing adapter, backend, features, limits, texture format capabilities, sample counts, subgroup properties, timestamp support, and surface formats.
2. Compare advertised WebGPU features/limits with the actual wgpu-core device before exposing them to JavaScript.
3. Remove compatibility-mode misclassification and retain the current core-defaulting/subgroup patches only with regression tests.
4. Add tests for every requested feature and limit used by the 182 examples.
5. Fail device creation with the exact unsupported requirement instead of silently lowering behavior.
6. Enable labels and error scopes around pipeline, texture, bind-group, and command creation in diagnostics builds without changing semantics.

### 6.6 Fix texture, sampler, and color-space fidelity

Implement and test:

1. unorm, snorm, integer, float, depth, compressed, array, 3D, and cube textures;
2. all copy paths, row alignment, origin, extent, mip, layer, and aspect handling;
3. `copyExternalImageToTexture` orientation, premultiplication, color conversion, and subrect behavior;
4. mip-level selection and manual mip chains;
5. anisotropic filtering and sampler-limit negotiation;
6. cubemap seams and environment-map LOD behavior;
7. HDR decode, working color space, tone mapping, and output transfer function;
8. KTX2 transcoding based on actual adapter-supported targets;
9. texture update visibility across queue submissions;
10. comparison samplers and depth textures.

Create GPU-generated/readback microtests before changing example results. Target failures include anisotropy, manual mipmaps, BPCEM environment maps, HDR, refraction, and texture-sensitive lighting.

### 6.7 Fix rasterization, MSAA, depth, and blending

1. Verify sample-count negotiation and multisampled attachment creation.
2. Verify color/depth resolve ordering and formats.
3. Test alpha-to-coverage, alpha hashing, premultiplied/straight alpha boundaries, and blend factors.
4. Test viewport/scissor rounding, front-face/culling, depth bias, stencil, and clip distances.
5. Validate reversed-Z and logarithmic-depth pipelines independently from DOM labels.
6. Validate point, line, sprite, shadow-map, and render-target edge behavior.
7. Compare per-attachment readbacks with Chrome/reference values rather than diagnosing only final PNGs.

Target failures include camera edges, multisampled renderbuffers, alpha hash, reversed/logarithmic depth, line raycasting, and rect-area/point lighting edges.

### 6.8 Fix compute semantics and synchronization

For every failing compute test:

1. capture input buffers/textures before dispatch;
2. capture output after each dispatch, not only after rendering;
3. compare shader translation, workgroup dimensions, buffer offsets, dynamic offsets, and bind-group layouts;
4. verify command ordering and compute-to-compute/compute-to-render visibility;
5. verify atomics and reduction initialization;
6. verify ping-pong resource identity and frame swap order;
7. distinguish permitted floating-point error from state divergence;
8. add the minimized defect as a native WebGPU regression test before patching deno_webgpu, naga, or wgpu.

Fix order:

1. `compute_reduce`;
2. `compute_texture_pingpong`;
3. `shadertoy`;
4. compute rasterizer;
5. particles/attractors/birds.

No tolerance is accepted for wrong resource state, missing writes, or ordering. Numerical tolerance is accepted only after state equivalence is proven.

### 6.9 Stabilize lighting, materials, and postprocessing

1. First prove texture/depth/MSAA correctness; do not compensate in material shaders.
2. Compare intermediate render targets for lighting and material failures.
3. Verify environment-map generation, BRDF LUTs, transmission targets, and volume passes.
4. Give every temporal pass an explicit history lifecycle:
   - uninitialized;
   - initialized from current frame or defined clear;
   - valid previous frame;
   - invalidated on resize/camera reset/device recovery.
5. Drive jitter and temporal sequences from the runtime frame index.
6. Define deterministic warm-up frame counts for runtime-native temporal goldens.
7. Validate postprocessing pass order, load/store operations, viewport sizes, and sampling.

Chrome first-frame identity is not required for temporal effects. Stable production output and repeatable runtime-native captures are required.

### 6.10 Complete deterministic asset and worker behavior

1. Consolidate module loading, CSS/image/font loading, fetch, Blob URLs, workers, and decoder activity under one readiness registry.
2. Preserve file/data/blob/HTTPS behavior with explicit unsupported schemes.
3. Add HTTP status, redirect, MIME, abort, and size-limit handling needed by game assets.
4. Give workers isolated execution state, deterministic message order, transfer ownership, termination, and propagated errors.
5. Cache immutable decoded assets by canonical URL and content hash without changing readiness semantics.
6. Validate glTF, Draco, KTX2, HDR/EXR, images, and fonts.
7. Never substitute an unavailable asset with an empty placeholder.

### 6.11 Route native window and input events

Connect winit directly to the existing LinkeDOM/Blitz interaction endpoint:

1. cursor enter/leave and movement → pointerover/out/enter/leave/move;
2. mouse buttons → pointerdown/up plus compatibility mouse events and click;
3. touch/pen IDs, pressure, primary pointer, cancellation, and pointer capture;
4. wheel pixel/line deltas → cancelable wheel event and scroll default;
5. keyboard physical code, logical key, modifiers, repeat, composition, keydown/up;
6. window focus → focus/blur lifecycle;
7. resize and scale-factor change → viewport/device-pixel-ratio update, media reevaluation, layout, observers, and canvas reconfiguration;
8. cursor coordinates transformed through scale factor and viewport;
9. `preventDefault()` evaluated before cancelable defaults;
10. focus, hover, active, and scroll state reflected into Stylo before repaint.

Add event-order tests, capture tests, transformed-hit tests, pointer-capture tests, and resize tests. Do not synthesize events by editing client code.

### 6.12 Finish HUD/editor UI to game-engine quality

Required, without pursuing full browser conformance:

1. LinkeDOM identity/mutations and long-lived Blitz reconciliation;
2. block/flex/grid/absolute/fixed layout;
3. transforms, clipping, stacking, opacity, overflow, and scrolling;
4. computed style and geometry;
5. front-to-back hit stacks;
6. focus, hover, active, checked, and selected state;
7. ResizeObserver, IntersectionObserver, and Stylo media queries;
8. readable deterministic fonts and stable metrics;
9. canvas/game viewport as a replaced element;
10. efficient HUD repaint and GPU upload.

Exact Skia glyph pixels are not required. Structural layout, hit regions, clipping, and stable readable text are required. Multi-canvas support remains in headless/editor mode but is not allowed to slow the primary production surface.

### 6.13 Unify production and deterministic scheduling

1. Implement one scheduler for tasks, microtasks, timers, rAF, observer delivery, and resource completions.
2. Production clock uses monotonic real time and surface/vsync pacing.
3. Test clock uses explicit deterministic advancement and seeded randomness.
4. rAF timestamps, timer deadlines, and frame indices come from the selected clock source.
5. Define fixed-step simulation support without changing browser rAF semantics.
6. Eliminate polling sleeps as logical synchronization; blocking waits may only yield the host thread while waiting on explicit state.
7. Record frame start, JS duration, GPU submit, GPU completion, and present timing.

### 6.14 Add production diagnostics

Provide a deliberate diagnostics API rather than temporary logging:

- adapter/features/limits report;
- lifecycle and readiness state dump;
- pending resource/token dump;
- shader compilation and pipeline errors;
- uncaptured WebGPU errors and device-loss reason;
- unhandled JavaScript rejection with original stack;
- labeled command/resource inventory;
- optional GPU timestamp report;
- optional frame, texture, and buffer capture;
- deterministic test manifest with hashes and classifications.

Diagnostics are opt-in and must not alter timing, RNG, features, or rendered output.

## 7. Validation matrix

### 7.1 Unit and conformance tests

Add native tests for:

- lifecycle transitions and illegal calls;
- readiness token completion/failure/cancellation;
- surface suspend/reconfigure/loss;
- feature/limit exposure;
- every texture copy and mip case;
- anisotropy and sampler behavior;
- MSAA/depth/stencil/blending;
- compute barriers, reductions, ping-pong, and atomics;
- color-space and alpha conversion;
- temporal-history initialization/invalidation;
- asset accounting, aborts, workers, and decoder errors;
- event ordering, cancellation, pointer capture, focus, resize, and DPR;
- LinkeDOM/Blitz state and observer delivery;
- direct presentation with no game-frame readback.

### 7.2 Backend matrix

Required:

- Lavapipe serial full suite for deterministic reference;
- NVIDIA serial full suite for production backend behavior;
- repeated runs for hashes and lifecycle races;
- at least one windowed production smoke test with resize/minimize/focus/input;
- headless capture and production surface paths exercising the same device layer.

### 7.3 Example gates

1. Zero non-video runtime errors across all 182 examples.
2. Both former readiness races pass ten consecutive runs.
3. Core GPU failures are fixed in this order:
   - compute reduce/ping-pong;
   - anisotropy/manual mip/HDR;
   - MSAA/depth;
   - lighting/material intermediates;
   - remaining compute.
4. Temporal examples have stable runtime-native goldens after defined warm-up.
5. DOM/text-only Chrome diffs do not block if structural geometry tests and runtime-native goldens pass.
6. Video examples remain explicitly excluded until the optional video subsystem is approved.

## 8. Atomic cutover and mandatory deletion

In the same final change that enables `EngineRuntime`:

- delete the clear-color spike from `src/main.rs`;
- delete duplicated runtime construction from examples;
- delete the direct wgpu-24 device/surface path;
- delete pending-counter and animation-frame readiness heuristics;
- delete random-skip/trace behavior and temporary probes;
- delete full-frame production CPU readback/presentation;
- delete alternate canvas/presenter paths and fallback switches;
- delete example-specific result handling;
- remove dependencies used only by the old host;
- retain only documented, regression-tested vendor patches.

Run repository searches for forbidden source rewriting, addon interception, fake modules, no-op implementations, example names in runtime paths, and diagnostic environment variables.

## 9. Final execution checklist

1. Format all changed Rust and JavaScript.
2. Run root unit tests and all vendored regression tests.
3. Run GPU conformance probes on Lavapipe and NVIDIA.
4. Run the 182 examples serially on Lavapipe.
5. Repeat lifecycle-race examples ten times.
6. Run the core GPU subset on NVIDIA.
7. Run windowed resize/input/device-loss smoke tests.
8. Repeat deterministic captures and compare hashes.
9. Generate the final pass/fail/classification manifest.
10. Verify no production frame uses full-frame readback.
11. Verify every remaining exclusion is an explicit non-goal, not an accidental missing API.
12. Update README architecture, supported profile, diagnostics, and known optional features.

## 10. Definition of done

The runtime is done when it behaves as a native game engine rather than a screenshot harness: unmodified three.js starts reliably, renders and computes correctly, presents directly to a native surface, loads assets deterministically, responds to native input and resize, supports practical HUD/editor UI, recovers or fails clearly on GPU loss, and can diagnose every remaining difference without client modifications or hidden fallbacks.
