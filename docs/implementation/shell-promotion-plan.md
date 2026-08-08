# `afterglow-shell` promotion plan — native host parity after CEF removal

> Status: active, updated 2026-07-26. `afterglow-shell` is the sole native host.
> Asset-root loading and native asset/texture/mesh composition are implemented;
> audio, packaging, diagnostics, and release evidence remain. Final lifecycle,
> readiness, visual evidence, and deletion ordering are owned by
> [`clean-unified-engine-convergence-plan.md`](clean-unified-engine-convergence-plan.md).

## Boundary

Native targets use generated native `afterglow-rpc` clients on real OS threads.
They never instantiate an engine service as WASM or a Web Worker. Public web is
the only WASM/Web Worker fallback.

The shell library is a host mechanism. It provides the winit/rusty_v8/WebGPU
environment, a generic worker registry, named bootstrap metadata, and Deno op
adapters. Concrete game services are selected by explicit application
bootstrap, not by `rpc_bridge.rs` or authored TypeScript.

## Gates

### G1 — Asset-root loading ✅

The shell canonically confines one `AssetRoot` before module evaluation. The
generated native `AssetLoaderClient` serves JS-visible `size` and positional
`read` calls on a real OS worker. `createPlatformRangeLoader()` selects those
ops and splits large V8-owned responses into at most 512 KiB ring payloads.

Native BIG header, resident texture, and raw GLB reads are working. The removed
CEF scheme/range bridge is not reintroduced.

Acceptance evidence:

- generated Dungeon HTML/modules load from `www/`;
- `dungeon.big`, resident blue noise, displacement, and raw assets load through
  the confined root;
- no whole BIG file is copied for a page range;
- no reusable native slot is released by V8 GC.

Remaining G1 release work: packaged embedded-first source policy.

### G2 — Native worker composition (assets/texture/mesh complete)

`ShellBuilder::with_workers` is the explicit pre-gameplay composition hook.
The command-line application currently registers:

- Physics reference worker: id 0;
- four named Texture workers: ids 1–4;
- one named Meshopt worker: id 5;
- the singleton native AssetLoader service.

IDs are Rust bootstrap details. `WorkerRegistry::register_named_async()` stores
service order, and authored TypeScript resolves `op_afterglow_worker_ids()`.
There are no worker-number constants in the web engine.

`NativeRpcTransport` drives asynchronous generated clients through
`op_afterglow_rpc_call_async`. A full async response ring retries under bounded
backpressure; completions are never silently dropped.

#### Native VT composition

Every texture worker opens a confined BIG source during bootstrap and retains a
generational `AssetSourceHandle` in a fixed `AssetSourceTable`. Runtime page
jobs contain only:

```text
source, offset, length, target_format
```

The OS worker performs `pread` and Basis transcode. Encoded source bytes never
enter V8. Only the final GPU-format page crosses back for the current Three.js
atlas upload.

The previous native arena/V8 external-backing path was deleted. It tied a
16-slot capacity to garbage collection and, despite its local zero-copy view,
immediately copied bytes into generated texture RPC arguments.

Acceptance evidence (2026-07-26, RTX 3090):

- Dungeon renderer ready in approximately 395 active ms;
- 400 native range-transcode calls completed in a 15-second smoke run;
- zero VT page failures;
- the run passed the former approximately 48-page/16-arena-slot wall;
- no texture WASM worker was selected.

Remaining G2 work:

- compose native audio through the same bootstrap;
- decide direct native atlas upload only after measuring the remaining final-
  page transfer;
- event-drain API if a composed service requires JS-visible events.

### G3 — Production bootstrap/configuration ✅

`ShellBuilder` configures root page, asset root, size, title, DevTools port, and
one worker-composition hook. The shell runs winit as the persistent scheduler,
advances module evaluation non-blockingly, and propagates runtime errors to a
fatal exit.

The shell library owns no reference worker lifecycle. The binary/application
bootstrap explicitly composes the services it needs.

Remaining product work is packaging policy rather than a second bootstrap API.

### G4 — Release evidence re-establishment

Re-create or retarget the removed CEF harnesses against the shell:

- VT feedback validation and 600-frame scenarios;
- Dungeon and rigged-VT correctness;
- 10/30/60-minute stable/traverse/thrash soaks;
- low-core POM Radeon 680M gate;
- input, resize, suspend, and device-loss gates;
- native audio physical-device gate;
- latency-tool attachment through a shell diagnostics endpoint.

Acceptance: evidence in `docs/benchmarks/` is re-cited against the shell. Old
CEF numbers remain explicitly historical until repeated.

### G5 — Documentation completion

`docs/api/asset-system.md`, `docs/api/virtual-texturing.md`,
`docs/api/afterglow-shell.md`, and the corresponding book chapters describe the
source-backed native path. Remaining chapters that still offer CEF commands must
be migrated or clearly labeled historical.

A permanent contract must enforce:

- native engine services never spawn WASM/Web Workers;
- demos import public engine APIs only;
- authored TypeScript contains no native worker IDs;
- generated `www/` output has no drift;
- native VT encoded bytes do not pass through the JS range loader.

## Out of scope

- Native audio implementation details are owned by
  `docs/implementation/spatial-audio-integration-plan.md`.
- Box3D and the editor remain deferred behind audio.
