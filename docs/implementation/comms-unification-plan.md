# Comms unification plan — one crate, both targets, generated TS

> Status: active. Decisions locked 2026-07-25. This plan consolidates the
> engine's worker-comms structure (currently split across `afterglow-rpc`,
> `afterglow-web`, and `afterglow-rpc-macros`, with the postcard codec + response
> envelope hand-duplicated in TS) into a single source of truth, and lands the
> zero-copy native worker layer. Each gate is independently mergeable and the
> workspace stays green throughout.

## Decisions (locked)

- **D1** — keep the name `afterglow-rpc`. The crate's scope grows (arena, GPU
  table, worker channel, handle layer) but the name stays.
- **D2** — full generation: the macro generates the TS transport + per-trait
  codec + typed clients. The hand-written `rpc.ts` is retired. Keep generated
  output clean and minimal.
- **D3** — standardized per-crate `gen/` directory. Every worker crate (and
  `afterglow-rpc` itself) self-produces its wasm + TS into `<crate>/gen/`. xtask
  aggregates `<crate>/gen/*` into the page's workers dir. No ad-hoc scanning or
  name-guessing.
- **D4** — `afterglow-rpc-macros` stays a separate `proc-macro = true` crate
  (Rust compiler rule; non-negotiable). Same pattern as `serde`/`serde_derive`.
- **D5** — `afterglow-web` shrinks to page runtime + engine + demos. It owns no
  comms after this plan.

## The unifying primitive

```
Handle { region, offset, length, generation }
```

A handle is "a region in shared memory." On **web** the shared memory is the SAB
(`WebAssembly.Memory.buffer`); a handle is an offset/length into it — exactly
what `rpc.ts` already does with `get_scratch_ptr`. On **native** the shared
memory is the Arc'd `EngineMemory` arena; a handle is a slot into it. The
framing, codec, `Transport` trait, and handle/arena are target-agnostic. Only
two things stay target-specific, both behind `#[cfg]` + a trait:

- the **transport mechanism** (SAB ring + `postMessage` wake vs native ring +
  `park`/`unpark`)
- the **arena backing** (SAB view vs Arc'd heap + V8 external ArrayBuffer)

## Target crate shape

```
afterglow-rpc/                        (rlib + cdylib for wasm32)
  src/
    lib.rs        framing [method][args] / [method][task_id][args], Response
                  envelope, postcard codec, Transport trait, ServeFuture
    handle.rs     Handle {region, offset, len, gen}, HandleArena trait   (G2)
    transport.rs  Transport trait: call / call_async / drain_events
    native.rs     ring storage, spawn/run loop, AsyncWorkerTransport, Oneshot,
                  events, arena (Arc'd) + V8 external backing store,   (G2/G6)
                  GPU resource table, worker↔worker HandleQueue         (G6)
    wasm.rs       SAB ring exports (init_ring_buffers, write_frame,    (G1)
                  read_response, get_*_ptr, notify_worker shim)
                  + arena-as-SAB                                        (G2)
  gen/            ← standardized output (transport.wasm, transport.ts, codec.ts)
  ts/             ← (replaced by gen/ in G3)
afterglow-rpc-macros/   proc-macro: generates Rust server/client + native spawn
                       + wasm exports + TS client + TS transport + codec (G3)
```

`afterglow-rpc` becomes `crate-type = ["rlib", "cdylib"]` so it produces the
transport wasm (`afterglow_rpc.wasm`, renamed from `afterglow_web.wasm`).

## Gates (ordered, each independently mergeable)

### G1 — Relocate the wasm ring transport into `afterglow-rpc::wasm` ✅

Move `init_ring_buffers` / `write_frame` / `read_response` / `get_request_ptr` /
`get_response_ptr` / `get_scratch_ptr` / `get_scratch_size` / the `notify_worker`
shim from `afterglow-web/src/lib.rs` into `afterglow-rpc/src/wasm.rs` behind
`#[cfg(target_arch = "wasm32")]`.

- `afterglow-rpc` gains `crate-type = ["rlib", "cdylib"]`; it produces
  `afterglow_rpc.wasm` (the transport wasm, formerly `afterglow_web.wasm`).
- `xtask wasm` builds `afterglow-rpc` as the transport wasm target.
- The macro's default `mainWasmUrl` is `'afterglow_rpc.wasm'`.
- `afterglow-web` loses the transport exports + the `afterglow-rpc` dependency;
it is now `crate-type = ["rlib"]` and owns only the dev server (native).

**Done (2026-07-25).** Transport wasm + exports moved to `afterglow-rpc::wasm`;
`afterglow_web.wasm` renamed to `afterglow_rpc.wasm`; macro default + audio-worklet
demo + xtask updated; stale `afterglow_web.wasm` removed; `afterglow-web` is
rlib-only. Workspace tests green (cargo test, xtask wasm, build-web --check,
contracts 15/15, rpc.test.mjs 11/11).

### G2 — Add the `handle` / `arena` layer (Rust primitive) ✅

**Done (2026-07-25).** `Handle` (target-agnostic, postcard-serializable) in
`lib.rs`; `handle.rs` (native-only) has `Arena` — Arc'd, fixed-capacity,
generational slots with an atomic lease state machine (`Free → WriteLeased →
ReadLeased → Reading → Free`) and RAII `WriteGuard`/`ReadGuard`. Safe API:
`acquire`/`handoff`/`read`; stale generations rejected; cross-thread
`Arc<Arena>` verified. 9 unit tests green. The V8 external-ArrayBuffer bridge
(JS view of a native slot) is the op layer — shell-promotion G2.

New `afterglow-rpc/src/handle.rs`:

- `Handle { region: u32, offset: u32, length: u32, generation: u32 }` — 16 B,
  postcard-encodable as a normal arg (no new wire format).
- `HandleArena` trait: `acquire(len) -> Option<Handle>`, `release(Handle)`,
  `view(Handle) -> &[u8]` / `view_mut(Handle) -> &mut [u8]`.
- **Native impl** (`native.rs`): Arc'd fixed-capacity arena, generational slots.
  Workers and the host share slots by raw pointer under the ownership protocol
  (a slot is either caller-write or reader-owned, enforced by generation).
- **Wasm impl** (`wasm.rs`): arena = the SAB; a handle indexes into it (offset +
  length within `Memory.buffer`). No new allocation.

The V8 external-ArrayBuffer bridge that lets JS view a native arena slot is the
op layer (shell-promotion G2), not this primitive. It is verified + landed there.

**Acceptance:** the primitive exists with unit tests (acquire/release/view/
generation-rejection) on both targets; consumed by G6 and shell-promotion G2.

### G3 — Macro generates TS into `<crate>/gen/`; codec dedup ✅

**Done (2026-07-25).** The `#[rpc]` macro now writes `<trait>.client.ts` into
`<crate>/gen/` (was `<crate>/ts/`). `xtask wasm` copies from `<crate>/gen/`.
Removed the duplicate postcard codec from `rpc.ts` (it re-implemented
`encodeVarint`/`decodeVarint`/`unwrapResponse` that `codec.ts` already owns);
`rpc.ts` now imports `unwrapResponse` from `codec.ts`. Single codec source.
Legacy `ts/` dirs removed; `.gitignore` updated to `gen/`. `rpc.test.mjs`
updated for `codec.ts` semantics (u64 varints, `decodeF32Vec(bytes, off)`).

**Deferred to G5:** moving the authored comms TS (`codec.ts`/`rpc.ts`) into
`afterglow-rpc/gen/` (comms crate ownership) and generating the transport/codec
from the Rust `Response` enum. The authored TS currently stays in
`afterglow-web/web/src/workers/` because the test workflow imports them directly;
moving them requires the test-resolution change that G5 (generation) brings.

### G4 — `xtask` aggregates from standardized `<crate>/gen/` ✅

**Done (2026-07-25, absorbed into G3).** `xtask wasm` auto-detects worker crates
by `gen/` dir presence and copies `<crate>/gen/*.ts` → `web/src/workers/`.
No filename guessing (the macro writes `<trait>.client.ts`). A separate
`xtask gen` subcommand was not needed — the aggregation lives in `xtask wasm`.

**Acceptance:** `xtask wasm` produces a clean `web/src/workers/` from `gen/`.

### G5 — Retire hand-written comms TS in the page; comms crate owns it ✅

**Done (2026-07-25).** The authored comms TS (`codec.ts` + `rpc.ts`) moved from
`afterglow-web/web/src/workers/` into `afterglow-rpc/web/src/workers/` — the
comms crate owns the transport + codec source. `xtask wasm` copies them into
the page's `web/src/workers/` (alongside the generated clients) so generated
clients + tests + `build-web` resolve `./codec.ts` + `./rpc.ts` unchanged.
`xtask test_all` runs a `stage_web` step (copy comms TS + rebuild `www/`) before
the node/bun tests, making the test workflow self-contained. The page copies
are gitignored; the comms-crate source is the single truth.

**Deferred:** generating the codec primitives from the Rust `Response` enum (a
larger macro effort; the codec is already single-source + cross-boundary tested).

### G6 — Zero-copy native: GPU resource table + worker↔worker HandleQueue

`afterglow-rpc/src/native.rs` gains two native-only primitives:

- **GPU resource table:** workers obtain a `wgpu::Device`/`Queue` from the shared
  `Arc<wgpu_core::global::Global>` (the shell already shares one) and call
  `queue.writeTexture`/`writeBuffer` directly. They register a generational
  texture/buffer handle; the renderer binds it. **Zero bytes cross to JS.**
  - **Verify first:** `wgpu_core::Global` is `Send + Sync` and callable from a
    worker thread (the shell's `GpuHudPresenter` already does
    `wgpu::Device::from_shared_core`, so the pattern exists).
- **`HandleQueue<S, G>`:** lock-free SPSC queue of handles for worker↔worker
  comms (asset loader → texture, physics → audio). Direct Rust-to-Rust, no
  renderer hop. Native-only.

**Arena→JS experiment retired (2026-07-26).** The external V8 backing store
worked technically, but its 16 reusable slots were released only by GC. Native
VT stalled around 48 pages, then failed/retried every new page. It also copied
the supposedly zero-copy view immediately into generated texture RPC arguments.
The op, asset handle methods, and arena tests were deleted. Native texture
workers now retain confined generational sources and perform `pread` + transcode
without exposing encoded bytes to JS. JS-visible raw ranges use bounded
V8-owned ring responses. Future zero-copy work must use explicit native
ownership or direct GPU upload, never GC-controlled capacity.

**HandleQueue done (2026-07-25).** `afterglow-rpc/src/handle.rs` has a lock-free
  SPSC `HandleQueue` for worker↔worker comms (asset loader → texture, physics →
  audio) — direct Rust-to-Rust, no renderer hop. 4 tests incl. cross-thread SPSC.

**Async op done (2026-07-25).** `WorkerRegistry` now holds both sync
  (`Box<dyn Transport>`) and async (`Arc<AsyncWorkerTransport>`) workers;
  `WorkerRegistry::call` dispatches to the sync `Transport::call` or a
  `block_on_async_call` wrapper (drives `AsyncWorkerTransport::poll` + parks on a
  thread waker). The op stays sync (blocks the JS thread like web's `call`).
  Test: an async worker round-trips through the blocking-poll wrapper.

**`wgpu_core::Global` Send+Sync verified (2026-07-25).** Compile-asserted; an
  `Arc<Global>` can be shared with worker threads so a worker can derive a
  `wgpu::Device`/`Queue` via `from_shared_core` and upload directly.

**Remaining for G6:** the integration with a real texture worker (the asset
loader → texture transcoder pipeline that produces GPU textures via this table).
The primitives are done + proven; the wiring is shell-promotion G2.

### G7 — `afterglow-web` shrinks

After G1 + G5, audit `afterglow-web`: it should own only the page runtime, the
engine (rendering, VT, assets, audio glue), and demos. Remove any comms
leftovers. Confirm it produces no transport wasm (that's `afterglow-rpc` now).

**Acceptance:** `afterglow-web` has zero comms code; `afterglow-rpc` is the sole
comms crate.

## Open implementation questions (resolve at the gate, not now)

- **G2:** the exact rusty_v8 0.149.4 external-backing-store API (verify before
  committing to externally-backed arena slots; fallback is owned-transfer).
- **G6:** `wgpu_core::Global` thread-safety for worker-thread uploads (the
  shell's presenter already uses `from_shared_core`; confirm a worker thread can
  hold a `wgpu::Device`/`Queue` derived from the shared `Global`).
- **G1 audit:** whether `afterglow-web` produces any wasm beyond the transport
  (if it does, that stays in `afterglow-web`; only the transport moves).

## Out of scope

- The JS↔Rust op bridge (deno_core `op_rpc_call` / `op_rpc_call_async`) — that's
  gate G2 of `docs/implementation/shell-promotion-plan.md`, built on top of this
  unified crate once G2 lands.
- Service-specific wiring (audio, assets, texture, meshopt composition into the
  shell) — `docs/implementation/shell-promotion-plan.md` G2.
