# afterglow-web API — SharedArrayBuffer Ring Buffer

> Status: **working** — verified on CEF 149 (native) and web browser.

## Overview

The `afterglow-rpc::RingBuffer` is the single communication mechanism for
website ↔ worker and worker ↔ worker. It operates on raw pointers + `AtomicU32`
with `Acquire/Release` ordering. The backing store differs by target:

| Target | Worker type | Memory backing | Spawn mechanism |
|--------|-------------|----------------|-----------------|
| Native (CEF) | Native thread (`std::thread`) | Heap (`Arc<Vec<u8>>`) | `afterglow_rpc::native::spawn_worker` |
| Web | Web Worker (wasm) | `SharedArrayBuffer` | `new Worker('worker.js')` |

Both use the same `RingBuffer::new()`, `write()`, `read()`, `has_data()` API.
No IPC, no serialization per call — just atomic memory writes.

## Native (CEF)

Workers are **native threads** — real OS threads with full `std`, no wasm
overhead. The ring buffer is in heap memory (`Arc<Vec<u8>>`), shared between
threads via `Arc`. The `afterglow-rpc::native` module provides `spawn_worker`,
`WorkerTransport`, and `run_worker_loop`.

```
Browser process:
  ┌─────────────┐     heap ring buffer     ┌──────────────┐
  │ Main thread │◄─── Arc<Vec<u8>> ──────►│ Worker thread │
  └─────────────┘                          └──────────────┘
```

The CEF shell (`afterglow-cef`) provides the window + WebGPU + scheme handler
+ COOP/COEP headers. It does NOT contain any worker or communication code —
that's all in `afterglow-rpc`.

## Web

Workers are **Web Workers** running compiled wasm. The ring buffer is in
`SharedArrayBuffer`-backed `WebAssembly.Memory`. JS creates the shared memory,
instantiates the wasm module with `--import-memory`, and shares the module +
memory with Web Workers via `postMessage`.

```
Renderer (same process):
  ┌─────────────┐   SharedArrayBuffer    ┌──────────────┐
  │ Main thread │◄── (wasm memory) ────►│ Web Worker    │
  │ (JS/Three.js)│                      │ (wasm)       │
  └─────────────┘                       └──────────────┘
```

## Build (web target)

```sh
# .cargo/config.toml at workspace root:
# [target.wasm32-unknown-unknown]
# rustflags = ['-C', 'target-feature=+atomics,+bulk-memory,+mutable-globals',
#              '-C', 'link-arg=--import-memory',
#              '-C', 'link-arg=--max-memory=67108864',
#              '-C', 'link-arg=--shared-memory']

cargo build -p afterglow-web \
  --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort \
  --profile wasm-dev
```

## Requirements

- **COOP/COEP headers** — required for `SharedArrayBuffer` on web. On native
  (CEF), set by the scheme handler in `afterglow-cef/src/resources.rs`.
- **Shared `WebAssembly.Memory`** (web only) — JS must create the memory with
  `shared: true` and pass it to the module as `env.memory`.

## Wasm exports (web target)

| Export | Purpose |
|--------|---------|
| `init_ring_buffer()` | Initialize the ring buffer header |
| `get_ring_buffer_ptr() -> usize` | Offset of ring buffer in wasm memory |
| `get_ring_buffer_size() -> usize` | Total size (8 MiB) |
| `ring_buffer_capacity() -> u32` | Data area capacity |
| `write_frame(ptr, len) -> i32` | Write a frame (worker side) |
| `read_frame(ptr, max_len) -> i32` | Read a frame (main thread) |
| `has_data() -> i32` | Non-blocking poll |
