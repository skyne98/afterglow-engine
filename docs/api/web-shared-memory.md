# afterglow-web API — SharedArrayBuffer Ring Buffer

> Status: **working** — verified end-to-end on CEF 149 (used as the browser),
> with COOP/COEP headers enabling `SharedArrayBuffer` + `crossOriginIsolated`.

## Overview

Web target for afterglow-engine. Uses `SharedArrayBuffer`-backed shared wasm
memory for zero-copy ring buffers between Web Workers and the main thread.

**True zero-copy** — no IPC, no serialization, no copies. The `RingBuffer`
from `afterglow-rpc` operates directly on shared memory using `AtomicU32`
with `Acquire/Release` ordering.

## Architecture

```text
Main thread (renderer):
  JS creates WebAssembly.Memory({ shared: true })
  JS instantiates wasm module with --import-memory
  wasm.init_ring_buffer()
  JS polls wasm.has_data() → wasm.read_frame()
  ↑ shares the same SharedArrayBuffer-backed wasm memory

Web Worker (physics):
  Receives { module, memory } via postMessage
  Instantiates the same module with the same shared memory
  wasm.write_frame(data) → writes directly to shared memory
  ↑ no IPC, no copy — just atomic memory writes
```

## Native vs Web comparison

| Aspect | Native (CEF) | Web (SharedArrayBuffer) |
|--------|:---:|:---:|
| Shared memory | `CefSharedMemoryRegion` (cross-process) | `SharedArrayBuffer` (cross-thread) |
| V8 sandbox issue | ✗ blocks external ArrayBuffers → one copy | ✓ no issue — SAB is designed for sharing |
| IPC per frame | yes (process message) | **none** — shared memory |
| Copies per frame | 1 (memcpy ~20µs for 64KB) | **0** — true zero-copy |
| Atomics | `std::sync::atomic` | `std::sync::atomic` (same) |
| RingBuffer | same `afterglow-rpc::RingBuffer` | same |

## Build

```sh
# The .cargo/config.toml at the workspace root sets:
#   --import-memory  (module uses JS-provided shared memory)
#   --shared-memory  (memory is a SharedArrayBuffer)
#   --max-memory=64MiB
#   +atomics,+bulk-memory,+mutable-globals

cargo build -p afterglow-web \
  --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort \
  --profile wasm-dev
```

## Requirements

- **COOP/COEP headers** — required for `SharedArrayBuffer`:
  ```http
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
  ```
  On native (CEF), these are set by the scheme handler (`resources.rs`).
  On web, the web server must set them (see `examples/coep_server.rs`).

- **Shared `WebAssembly.Memory`** — JS must create the memory with
  `shared: true` and pass it to the module as `env.memory` (the module
  is compiled with `--import-memory`).

## Wasm exports

| Export | Purpose |
|--------|---------|
| `init_ring_buffer()` | Initialize the ring buffer header (call once) |
| `get_ring_buffer_ptr() -> usize` | Offset of ring buffer in wasm memory |
| `get_ring_buffer_size() -> usize` | Total size (8 MiB) |
| `ring_buffer_capacity() -> u32` | Data area capacity |
| `write_frame(ptr, len) -> i32` | Write a frame (Web Worker side) |
| `read_frame(ptr, max_len) -> i32` | Read a frame (main thread) |
| `has_data() -> i32` | Non-blocking poll |
| `read_header_cap() -> u32` | Diagnostic: read capacity from header |
| `write_test_marker() -> u32` | Diagnostic: write test bytes |

## JS interface

```js
// 1. Create shared memory
const memory = new WebAssembly.Memory({
  shared: true, initial: 256, maximum: 1024,
});

// 2. Instantiate the wasm module with the shared memory
const instance = await WebAssembly.instantiate(module, { env: { memory } });
const wasm = instance.exports;

// 3. Initialize the ring buffer
wasm.init_ring_buffer();
const offset = wasm.get_ring_buffer_ptr();

// 4. Share the module + memory with a Web Worker
const worker = new Worker('worker.js');
worker.postMessage({ type: 'init', module, memory });

// 5. Poll for data on the main thread
function poll() {
  while (wasm.has_data()) {
    const readBuf = offset + size - 512; // scratch area
    const n = wasm.read_frame(readBuf, 512);
    if (n > 0) {
      const data = new Uint8Array(memory.buffer, readBuf, n);
      // Use data for device.queue.write_buffer(), etc.
    }
  }
  requestAnimationFrame(poll);
}
```
