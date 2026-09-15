# Native paint shell validation

## Configuration

- Linux with NVIDIA GeForce RTX 3090 and Vulkan/WebGPU.
- `afterglow-shell --document` with the built `paint-native-probe.html`.
- One native `PaintWorker` OS thread through the generated RPC client.
- A 2048 × 2048 paint document with memory-only history.
- No native service WASM, Web Worker, or graphics fallback.

## WebSocket capture

The CLI owns the WebSocket server. Applications connect outward without credentials.
`profiling-1788654486479/` passed the native capture check on the RTX 3090.
Its DGTL file contains 2,419 host records and 44 paint records, with zero losses.
The host records include 140 matched RPC pairs and 2,139 presentation records.
The paint records include 40 stroke samples and four raster publications.

The before/capture/after mean times were 6.931/7.061/7.159 ms.
The corresponding p99 values were 9.851/17.573/18.348 ms.
This is functional capture evidence, not an isolated overhead measurement or soak.
The CLI/book checks could overlap the start of this functional test.

The initial browser run (`profiling-web-1788654507856/`) retained 53 valid records
with no losses, but its extra screenshot operation reached a deadline.
Its README describes the failed check and an overwritten timing field in the initial driver.
The corrected driver uses the existing pixel probe and keeps timing separate from file metadata.
The repeated browser check (`profiling-web-1788654677891/`) passed with 53 records,
zero losses, 40 stroke samples, 13 worker messages, and 954 colored pixels.
Its before/capture/after mean times were 6.945/6.946/6.951 ms, with p99 values of 6.980/6.980/6.985 ms.
These short samples are not a sustained overhead or memory check.

Run after the release shell/collector and Vite builds:

```sh
nix-shell shell.nix --run 'bun scripts/test-native-paint-capture.ts'
nix-shell shell.nix --run 'bun scripts/test-web-paint-capture.ts'
```

For a fresh native control process without a collector, add an output directory and `--control` to the native command.
The control and capture pages use the same paint workload, history depth, and phase delays.
The middle phase starts two seconds after readiness, and the last starts after twenty seconds.
A late connection can extend these delays. Each result records actual phase start and end times.

The [six-process comparison](profiling-comparison-1788655121419/README.md) passed all functional checks.
The median mean was 7.118 ms for control and 7.206 ms for capture (1.24% higher).
The median p99 was 18.614 ms for control and 19.208 ms for capture.
All capture maxima exceeded all control maxima. The overhead check remains open.
One capture had an unexplained one-second cadence interruption, which remains in the evidence.

Both runners close only their own application and collector processes.
The browser runner uses a separate browser session and a dedicated local server port.

## Historical direct TCP capture

Three RTX 3090 launches passed the native capture check. Each viewer connected
without credentials to the application-selected localhost port. The saved DGTL
files contain separate `afterglow-shell` and `afterglow-engine.paint` sources.
Each file has 140 matched RPC pairs, 40 stroke samples, four raster publications,
and zero dropped or overwritten records. Capture order and source identities passed.

The probe repeats four diagnostic strokes over 240 rAF intervals per phase.
These calls use the real native paint worker and RGBA publication path, but not OS
mouse input. The later two launches repeat the workload after viewer disconnect.
No builds ran during these timing samples.

| Run | Before mean/p99 | Capture mean/p99 | After mean/p99 |
|---|---:|---:|---:|
| `profiling-1788649138651` | 6.901 / 10.038 ms | 7.061 / 18.505 ms | Not sampled |
| `profiling-1788649250949` | 6.930 / 10.411 ms | 6.975 / 17.717 ms | 7.235 / 23.169 ms |
| `profiling-1788649272224` | 6.928 / 10.028 ms | 7.322 / 19.785 ms | 7.322 / 20.517 ms |

Capture increased p99 relative to the initial samples. The later samples without
capture had still higher p99. These short sequential samples do not isolate capture
cost from paint history, warm-up, garbage collection, or application age.
They do not prove zero application delay, physical input latency, or soak stability.
Each directory contains `result.json`, `host.log`, and the original DGTL file.

Run after the release shell/collector and Vite builds:

```sh
nix-shell shell.nix --run 'bun scripts/test-native-paint-capture.ts'
```

The runner closes only its own shell and viewer. It never uses screenshot auto-exit
for a normal user window. The first failed capture sent repeated empty batches.
Those batches reused sequence numbers and the collector correctly rejected them.
The pump now skips empty transfers. Unit tests cover idle sources, another active
source, disconnection, reconnection, stale work, and capture-only failure.

## Initial result

The probe completed with exit code 0 and its explicit `PASS` marker.
The log contains no native input-dispatch failure.
The screenshot shows the pointer stroke in the paint editor.

| Check | Result |
|---|---:|
| Initial colored scanline pixels | 0 |
| Diagnostic stroke | 954 |
| Undo | 0 |
| Redo | 954 |
| Pointer stroke after undo | 194 |
| Pointer undo | 0 |
| PNG output | 87,192 bytes, correct PNG signature |

The probe sends pointer events through the editor handlers.
It does not simulate an OS mouse or a pressure-sensitive tablet.
This check does not establish frame budgets, sealed allocation behavior, or long-soak stability.
Native file-dialog interaction and full browser-control visual parity need separate manual checks.
The Vulkan loader reports a missing Monado layer, but selects the real RTX 3090 adapter.

## Normal-page startup regression

The initial normal-page launch ran only the first module script, a Vite helper.
The editor module did not execute, so the window stayed black despite its readiness message.
The probe used a different module entry and did not detect this error.

The corrected loader includes every module script and preserves separate inline module scopes.
Both loader tests passed.
A new normal-page run selected the RTX 3090 and reached readiness after 11,620 active milliseconds.
Its screenshot shows the canvas, toolbar, Color panel, brush grid, and Layers panel.

## Bounded batching and idle presentation

The current adapter reads at most eight tiles concurrently and groups at most 32 adjacent stroke samples in one ordered command.
The worker retains the dirty region from every sample and continuation step.
The host schedules pending HUD output and screenshots even when RPC and animation queues are empty.
Before that scheduler correction, the probe passed but its screenshot timed out after 600 seconds.

The corrected run completed with `PASS`, exit code 0, and a new screenshot.
Diagnostic stroke, undo, and redo counts were 954, 0, and 954.
The pointer path used native hit-testing and event dispatch, not direct canvas dispatch.
Its stroke and undo counts were 189 and 0.
The PNG had 87,395 bytes and the correct signature.

The diagnostic stroke check took 243.776 ms.
The three-event pointer check took 347.081 ms, including two intentional waits and the final raster probe.
These values are not OS input-to-pixel latency and do not establish an interactive latency target.
The full probe reached readiness after 11,258 active milliseconds.
The initial direct-handler probe took 54,210 ms, but its pointer path differed.

## Native control regression

The control probe passed on the RTX 3090 with exit code 0.
Six alternating brush selections retained the painted scanline.
The probe also checked labels below images, a slider change to 47, and a 200-pixel wheel scroll.
Native hit-testing selected the correct brush after the scroll.
The screenshot shows the slider tracks and thumbs and the corrected brush labels.

The repairs address these causes:

- Text hit-testing accepted points above or below a text line and could activate unrelated controls.
- Overflow hit-testing accepted clipped children.
- The native renderer had no range-input track, thumb, or input defaults.
- The default button flex layout put brush labels beside their images.
- Taffy 0.12.2 calculated grid content bounds from cell-local positions, which prevented scrolling.
- DOM synchronization replaced unchanged stylesheets.
- The paint adapter could wait for 32 serial sample replies before raster publication.

The grid adapter now calculates content bounds from completed container-relative positions.
The paint adapter publishes each ready sample group before it continues with queued input.
Unit regressions check grid scrolling, text bounds, overflow clipping, range controls, and intermediate stroke publication.

The final control sequence took 2,051.758 ms, including six raster probes.
The diagnostic stroke and pointer checks took 287.640 ms and 453.402 ms.
These probe durations do not measure OS input-to-pixel latency or prove a latency improvement.
A manual mouse or tablet latency check remains necessary.
The final regression run passed 83 editor tests, 28 shell tests, three paint-worker tests, and the DOM API tests.
TypeScript, generated-web drift, mdBook, and diff checks also passed.

## UI cadence baseline

The monitor reports approximately 144 Hz.
The shell selected 143.975 Hz, but this deadline did not establish the actual UI frame rate.
The same probe ran first in debug mode, then in release mode, without concurrent compilation.
Each mode has ten initial frames and 120 measured rAF intervals.
Hover input alternates between the brush grid and toolbar.
Slider input changes the radius on each frame.

| Build | UI changes | Mean FPS | p50 interval | p99 interval |
|---|---|---:|---:|---:|
| Debug | None | 143.95 | 6.933 ms | 8.308 ms |
| Debug | Hover | 11.57 | 84.534 ms | 113.281 ms |
| Debug | Slider | 9.78 | 102.061 ms | 115.067 ms |
| Release | None | 143.92 | 6.950 ms | 8.143 ms |
| Release | Hover | 126.05 | 7.751 ms | 12.503 ms |
| Release | Slider | 56.36 | 17.371 ms | 23.363 ms |

Native presentation counts also decreased during the slow UI sequences.
The baseline confirms the reported UI problem, not a display refresh selection error.
These short, synthetic-input checks measure frame production, not physical scanout or OS input-to-pixel latency.
The previous idle-page and control checks did not detect this performance defect.

## Incremental layout and duplicate synchronization

The shell now enables the existing Blitz incremental-layout feature.
A pixel comparison checks incremental and full layout after text, range, style, visibility, viewport, and child-removal changes.
The native control probe then found a cached-overflow error that the initial unit checks missed.
Cached bounds lacked the child position and transform during parent calculations.
A grid-gap regression reproduced this error before the correction.
The corrected incremental build passed 29 shell tests and the native control probe.
It measured 135.84 FPS for hover changes and 93.22 FPS for slider movement.

Temporary instrumentation measured approximately 1.1 ms for snapshot construction and 2.9 ms for native synchronization during slider movement.
Native scene construction took approximately 0.4 ms.
Many frames performed two synchronizations.
Retaining unchanged Layers panel rows did not materially change frame timing, although it removed unnecessary DOM replacement.
The final duplicate-work regression found that LinkeDOM calls its mutation observer with an empty list after `takeRecords()` drains the queue.
The shell incorrectly marked this empty callback as another DOM change.
The correction ignores empty callbacks and still processes later mutations.
The DOM API test failed before this correction and passed afterward.
Temporary timing instrumentation is not part of the final runtime.

Three launches with the empty-callback correction measured hover rates of 126.91–130.76 FPS and slider rates of 126.81–145.05 FPS.
These runs still did not show continuous 144 Hz UI updates.
A second regression found that mouse-event construction forced layout after pointer listeners changed the DOM, even without offset readers.
Native events now calculate target-relative offsets only when code reads them.
The regression checks both deferred synchronization and correct coordinates after dispatch.

## Corrected UI cadence

Three fresh release launches passed the paint and control probes on the RTX 3090 at 1716×1350 pixels.
No compilation ran during measurement.

| Launch | UI changes | Mean FPS | p50 interval | p99 interval |
|---|---|---:|---:|---:|
| 1 | None | 143.98 | 6.940 ms | 7.148 ms |
| 1 | Hover | 138.31 | 6.942 ms | 14.194 ms |
| 1 | Slider | 143.99 | 6.941 ms | 7.158 ms |
| 2 | None | 143.98 | 6.944 ms | 7.155 ms |
| 2 | Hover | 138.22 | 6.943 ms | 15.181 ms |
| 2 | Slider | 144.25 | 6.940 ms | 7.128 ms |
| 3 | None | 143.98 | 6.945 ms | 7.075 ms |
| 3 | Hover | 130.04 | 6.954 ms | 14.916 ms |
| 3 | Slider | 144.25 | 6.944 ms | 7.395 ms |

Slider updates now approximately match the monitor rate in these short checks.
Hover changes still miss frame deadlines, so this is not continuous 144 Hz validation.
Physical scanout, OS input latency, and long soaks remain unmeasured.
The final checks passed 84 editor tests, 29 shell tests, three paint-worker tests, and the DOM API tests.
TypeScript, generated-web drift, mdBook, and diff checks passed.
The final screenshot retains the stroke, layer row, sliders, and brush layout.

## Blitz main update and native controls

The update uses upstream commit `a50cb8971a03fb4cac697b763f8f5d01ee83cefb`.
The release probe passed on the RTX 3090 and produced `blitz-main-controls.png`.
The screenshot shows the select labels, numeric values, sliders, and painted stroke.

The probe checked three select popups, wheel zoom direction, numeric input values, and the button hover transition.
The hover background changed from transparent to `rgb(51, 51, 51)`.
It also passed the previous brush, scrolling, slider, stroke, undo/redo, and PNG checks.

Two popup failures improved the checks:

- The probe initially clicked an option outside the visible popup area. It now scrolls that option into view first.
- A native regression confirmed that content scrolling incorrectly moved the container client rectangle.
  The shared rectangle calculation now excludes the container's own scroll offset.
  Layout offsets use the upstream layout methods, not differences between client rectangles.

All 53 native regressions and 84 editor tests passed.
The DOM API tests, TypeScript check, and release/frontend builds also passed.

| UI changes | Mean FPS | p50 interval | p99 interval |
|---|---:|---:|---:|
| None | 143.00 | 6.929 ms | 9.943 ms |
| Hover | 143.99 | 6.925 ms | 8.412 ms |
| Slider | 144.08 | 6.780 ms | 9.163 ms |

These values describe one launch with 120 rAF intervals per mode, not continuous 144 Hz operation or physical scanout.
OS input latency, native dialogs, tablet input, and long soaks remain unverified.
The earlier cadence tables describe the previous Blitz pin.

## Number-input display correction

The user screenshot showed blank Width and Height fields after the Document panel opened.
The previous probe checked their stored values but did not open that panel or inspect their displayed digits.
Blitz created the number-input text editor but omitted `number` from the intrinsic text-input sizing path.
The fields therefore had zero content height and only 10 pixels of padding and border.

The correction adds `number` to that existing sizing path.
A raster regression checks colored digit pixels with the panel initially open and initially closed.
It failed before the correction and passed afterward.
The native probe now opens the panel and checks sufficient input height.
The RTX 3090 screenshot shows `2048` in both fields.

All 42 shell tests passed, along with TypeScript, frontend/release builds, and the native control/paint probe.
This check does not establish native text-editing parity or OS input latency.

## Evidence

- [Raw build and probe log](probe.log)
- [Native screenshot](pointer.png)
- [Blitz main control probe](blitz-main-controls.log)
- [Blitz main control screenshot](blitz-main-controls.png)
- [Number-input probe](number-inputs.log)
- [Visible document dimensions](number-inputs.png)
- [Batched probe log](batched.log)
- [Batched probe screenshot](batched.png)
- [Native control probe log](controls.log)
- [Native control screenshot](controls.png)
- [Debug and release UI cadence baseline](cadence-baseline.log)
- [Incremental-layout intermediate run](cadence-incremental.log)
- [Temporary synchronization profile](cadence-profile.log)
- [Empty-callback correction, three launches](cadence-empty-callback.log)
- [Corrected UI cadence, three launches and regressions](cadence-final.log)
- [Corrected UI screenshot](cadence-final.png)
- [Normal-page startup log](normal-startup.log)
- [Normal-page screenshot](normal.png)
- Probe source: `prototype/character-editor/src/paint-native-probe.ts`

## Native text and menu repair

The current repair passed 45 shell regressions (36 library and nine binary tests).
The DOM API tests and 84 editor tests (3,429 expectations) also passed.
Native raster checks cover selection, Unicode replacement/deletion, readonly controls, and selection beyond an input border.
Geometry regressions cover nested transforms at scale 1 and 2, initial popup hits, and translated popup hits.

The RTX 3090 probe passed these checks:
- File/Edit/View opening, placement, Escape, keyboard opening, item activation, and outside clicks.
- Width and Height replacement with `1024`, deletion to `102`, and restoration to `2048`.
- Three select popups, brush selection, wheel direction, hover, paint history, and PNG export.

The retained [text-control log](text-controls.log) and [screenshot](text-controls.png) show the completed probe and visible dimension values.
The [menu log](input-menus.log) and [screenshot](input-menus.png) record a second passed run with the View menu open.
These probes dispatch events through the native browser bridge. They do not measure OS input-to-pixel latency or tablet behavior.
Native clipboard, text undo, and IME are not integrated. Chromium-equivalent artist input quality remains an open gate.
See [the pinned Chromium comparison](../../research/native-shell-input-chromium-comparison.md).

## Repeat

From the repository root:

```sh
(cd prototype/character-editor && bun run build)
nix-shell shell.nix --run 'CARGO_BUILD_JOBS=$(nproc) cargo build --release -p afterglow-shell'
nix-shell shell.nix --run 'AFTERGLOW_STARTUP_TIMEOUT_MS=120000 AFTERGLOW_CAPTURE_PATH=/tmp/native-paint.png ./target/release/afterglow-shell --document prototype/character-editor/dist/paint/paint-native-probe.html'
```

A successful process exit alone is not sufficient.
The run must emit `[native-paint-probe] PASS` and create a new screenshot without input-dispatch errors.
