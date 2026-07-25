# `afterglow-shell` promotion plan — native host parity after CEF removal

> Status: active. `afterglow-cef` has been removed (2026-07-25). This plan is
> the canonical sequence for bringing `afterglow-shell` to full native-host
> parity and re-establishing the release gates that ran under CEF.

## Context

`afterglow-cef` was the transitional Chromium/cef-rs native host. It has been
removed in full: the crate, its five example launchers, the CEF native range
bridge (`afterglowNativeReadRanges` / `cef-native-range.ts`), the CEF GPU soak
scripts, `shell.nix` CEF wiring, the `cef` workspace dependency, and
`docs/api/cef-shell.md`. The research notes under `docs/research/cef-*.md`
remain as the historical decision record.

`afterglow-shell` (rusty_v8 + Deno WebGPU/wgpu-core + winit + LinkeDOM +
Blitz + Vello) is now the **sole** native host. It is not yet at parity: it
presents Three.js WebGPU and a Vello HUD, but it does not compose native
`afterglow-rpc` workers, load Afterglow asset roots, expose a production game
bootstrap API, or carry release evidence. Until the gates below close, there is
**no** native host that wires native worker clients, and the engine's GPU soak
harnesses are offline.

The native target boundary is unchanged and non-negotiable: native targets use
generated native `afterglow-rpc` clients on real OS threads; they never
instantiate a service as WASM or a Web Worker. Closing these gates must not be
"shortcut" by moving worker services back into WASM on the native target.

## Gates (ordered)

Each gate is independently mergeable. Do not skip ahead.

### G1 — Asset-root loader + packaged-resource serving

The shell must serve embedded-first / FS-fallback assets through an
`afterglow-assets`-backed loader equivalent to the removed `afterglow://`
scheme.

**Started (2026-07-25): page loading.** The shell now loads built demo HTML
pages from `www/` (not just `native_game.ts`): `extract_module_script` handles
external `<script type="module" src="...">` (the afterglow demos) in addition
to inline module scripts (official three.js). The shell provides
`globalThis.location` (a `URL` over the page's file URL). Verified: `engine-demo.html`
loads + its built JS module evaluates on the RTX 3090 (adapter confirmed).

**Done (2026-07-25): production host scheduling for `runtime.warm()`.**
The root cause was Three.js `renderer.compileAsync()` yielding through rAF while
the old synchronous module evaluator could not admit a presentation turn. The
prototype loop that ignored deno errors has been deleted.

winit is now the persistent outer scheduler. Game-module evaluation is stored
in `App` and advances non-blockingly across real redraws; startup/frame/input/
resize paths poll one bounded deno_core turn instead of running to idle. A real
coalesced winit waker replaces the no-op waker, and errors propagate unchanged
to a nonzero fatal exit. The authored `raf.ts` queue has 1,024 fixed slots, O(1)
request/cancel, shared timestamps, next-frame registration, deterministic
overflow and telemetry. One `ExternalOpsTracker` token keeps deno_core alive
while callbacks remain pending. The host no longer invokes the renderer through
a parallel direct-frame path.

Verified on the RTX 3090: `native_game.ts` reports `OP_BRIDGE_OK`; a two-await
probe advances on separate redraw timestamps; and a rejected-TLA sentinel
preserves its original error and exits nonzero. The fixed-rAF Bun regression
covers ordering, cancellation, callback exception isolation, capacity and
overflow.

The correct pure-rAF gate showed that presentation-granularity continuation was
too coarse: `engine-demo.html` had not completed real Three.js `compileAsync`
after 90 active seconds. The user therefore explicitly admitted only the
standards-shaped `scheduler.yield()` subset, backed by `deno_web::op_defer` and
the bounded Tokio/winit turn; `postTask` and `TaskController` remain absent.
10,000 continuation probes completed in 193.3 ms, and the real 5,000-entity
engine demo reached renderer readiness in 145 active ms under the default
30-second deadline. The Scheduler API-shape/asynchrony regression passes. G1's
scheduling blocker is closed; release soak/hardware evidence remains G4.

A decision on whether to reintroduce a native range-read bridge for `.big` bulk
reads (the removed `readCefNativeRanges` path) or rely on the in-process
`FsSource` + serving-layer path is still open (D2 in the removal discussion);
recommended default is the in-process `FsSource` path.

- A shell host op (or in-process loader) that resolves `AssetRoot` paths via
  `afterglow-assets::resolve` and streams `FsSource`/`BytesSource` bytes into
  the JS module loader and `fetch` polyfill.
- COOP/COEP-equivalent isolation so `SharedArrayBuffer` works on the native
  target (the shell is single-process; confirm whether headers are needed or
  whether SAB is available by construction).
- A decision on whether to reintroduce a native range-read bridge for `.big`
  bulk reads (the removed `readCefNativeRanges` path) or rely on the in-process
  `FsSource` + serving-layer path. **Open decision** — see D2 in the removal
  discussion; recommended default is the in-process `FsSource` path (no V8
  sandbox, no shared-memory bridge needed) unless a measurement shows bulk read
  is a bottleneck.

Acceptance: `big-parser.ts`'s `createFetchRangeLoader` works against the shell
host for both singleton and bulk reads of a cooked `.big`, with no CEF code.

### G2 — Native `afterglow-rpc` worker composition

The shell must spawn native worker clients (audio, assets, texture, meshopt)
from an explicit native bootstrap hook — the equivalent of CEF's
`AppBuilder::on_ready`. **Depends on `docs/implementation/comms-unification-plan.md`**
(G2 handle/arena for zero-copy, G6 GPU table + worker↔worker queue). Concrete
work:

**Started (2026-07-25): the op bridge.** `afterglow-shell/src/rpc_bridge.rs`
exposes a `WorkerRegistry` (in `OpState`) + `op_afterglow_rpc_call` — the
same `Transport::call(method, args)` surface the web transport exposes,
returning a V8 `Uint8Array` via ownership transfer (no memcpy). Registered in
`engine_ext`; `WorkerRegistry` put in `OpState` at startup. Proven via 6 unit
tests: a native `Physics` worker round-trips (`step(vec![0,1,2], 0.5)` →
`[0.5,1.5,2.5]`) through `WorkerRegistry::call`; unknown worker id + unknown
method are clean errors; an async worker round-trips through the blocking-poll
wrapper; a JsRuntime test proves JS reads a native arena slot in place via V8
external backing store (zero copy) + the slot releases on GC/teardown.

**End-to-end on the GPU (2026-07-25).** The shell runs on the fox-workstation's
NVIDIA RTX 3090 via Wayland (adapter confirmed; the first-frame "Invalid Surface
Status" is a one-time Wayland configure hiccup, recovered on frame 2). A
Physics worker is spawned natively at startup (`register_physics`); a JS probe
module calls `op_afterglow_rpc_call(0, 0, args)` from a real run and logs
`OP_BRIDGE_OK [0.5,1.5,2.5]`. The full native worker composition path is
proven end-to-end on the real GPU. (`NativeRpcTransport` TS lets the generated
TS clients call the op; `op_afterglow_arena_view` provides the zero-copy arena
path; `block_on_async_call` handles async workers.)

**Remaining for G2:**
- spawn real engine workers (assets, texture, audio) from a bootstrap hook +
  assign stable ids;
- `op_rpc_call_async` (await the `Oneshot` on deno_core's tokio) for async
  workers (`AsyncWorkerTransport`);
- `op_rpc_drain_events`;
- a `NativeRpcTransport` TS class so the generated clients call the op;
- the shared-arena zero-copy path: worker writes an `afterglow_rpc::handle::Arena`
  slot, returns a `Handle`, the op creates a V8 external ArrayBuffer over the slot
  (verify the rusty_v8 0.149.4 external-backing-store API at this step).

- A shell bootstrap boundary that fires after the winit window + wgpu device are
  ready and before gameplay sealing.
- Generated native `Client::spawn_worker` calls wired for each service the
  loaded game requests.
- The `BigAssetSession` target-aware factory fix: on the native target,
  `afterglow-texture` runs as an OS worker via its generated native client, not
  `texture.wasm` Web Workers. This closes the audited `BigAssetSession` defect
  that previously existed under CEF.
- A native CDP/DevTools endpoint (or equivalent) so `latency-tool` and
  `?bench=300` can attach.

Acceptance: a native-shell run loads `dungeon.big` through
`AssetLoaderClient` (native) and transcodes VT pages through the native texture
worker, with no WASM worker on the native target.

### G3 — Production game bootstrap / configuration API ✅

**Done (2026-07-25).** `afterglow-shell/src/builder.rs` provides `ShellBuilder`:
`root` (HTML/module path), `size`, `title`, `devtools` port, + a
`with_workers(FnOnce(&mut OpState))` composition hook — the native equivalent
of CEF's `AppBuilder::on_ready`. The shell's `main` constructs a `ShellBuilder`
with `reference_composition()` (spawns Physics id 0 + the real `Texture`
transcoder id 1 natively at startup). The hook runs after the winit window +
wgpu device are ready, before gameplay sealing.

**Real engine service composed:** the `Texture` transcoder (async `#[rpc]`,
Basis → BC7/ASTC/etc.) is spawned natively + registered via the hook — a real
non-demo service. Unit test proves it composes + dispatches (unknown method →
"unknown method" from the texture worker). The shell run on the 3090 composes
both Physics + Texture (`OP_BRIDGE_OK`).

**Remaining:** rehome the five removed example launchers (`minimal`, `dungeon`,
`lod-demo`, `vt-demo`, `rigged-vt-demo`) as one-liner `ShellBuilder` programs
(they were 12-line `AppBuilder` wrappers; the former `compileAsync` blocker is
closed). Composing `assets` + `audio` workers through the same hook is
the G2-finish (the hook structure is in place; `register_texture` is the
reference for `register_assets`/`register_audio`).

Acceptance: each of the five demo pages launches and renders under the shell
with hardware WebGPU (NVIDIA RTX 3090 on this workstation), no WebGL fallback.

### G4 — Release evidence re-establishment

Rehome the GPU soak/validation harnesses that were deleted with CEF:

- `scripts/test-vt-gpu.sh`, `scripts/test-dungeon-gpu.sh`,
  `scripts/test-rigged-vt-gpu.sh`, `scripts/test-lod-gpu.sh`,
  `scripts/run-dungeon.sh`, and the dungeon soak scripts — re-created against
  the shell host.
- Re-run the canonical evidence: VT feedback validation (9,216 pixels), the
  600-frame rAF timing per scenario, the sealed VT 10/30/60-min soaks, the
  low-core POM 680M gate, and the audio native gate (currently open — see
  `docs/implementation/spatial-audio-integration-plan.md`).
- `latency-tool` re-targeted and re-measured against the shell's CDP endpoint.

Acceptance: the release-gate evidence in `docs/benchmarks/` and AGENTS.md is
re-cited against the shell host, not the removed CEF host. Stale CEF-era
numbers must be re-run or explicitly marked historical.

### G5 — Documentation completion

- Rewrite the `book/` chapters that gave CEF build/setup commands
  (`setup/prerequisites.md`, `setup/verify.md`, `building/native.md`,
  `building/afterglow-shell.md`, `window/app-builder.md`,
  `guides/game-window.md`, `reference/crate-map.md`, `reference/debugging.md`,
  `reference/further-reading.md`, `window/asset-system.md`,
  `window/virtual-texturing.md`) to use the shell host. **The book currently
  tells users to run a deleted crate — this is the highest-priority doc debt.**
- Complete the `docs/api/asset-system.md` native-loader rewrite (the CEF bridge
  prose is now trimmed to removal notes; the native shell loader from G1 must
  be documented once it exists).
- Re-establish a native-target contract test in `scripts/contracts.test.ts`
  (the removed `native CEF target contract` block enforced the no-WASM-on-native
  rule; re-author it against the shell once G2 lands).
- Update `docs/implementation/demo-to-engine-feature-audit.md` (references the
  removed `afterglow-cef/examples/*.rs` entrypoints).
- Refresh `AGENTS.md` benchmarks/soak numbers once G4 re-runs them.

## Out of scope for this plan

- Audio native composition final CEF integration — the audio plan's native gate
  is separately tracked in `docs/implementation/spatial-audio-integration-plan.md`.
  CEF removal means that gate now targets the shell, not CEF.
- Box3D physics, editor — still deferred behind audio.
