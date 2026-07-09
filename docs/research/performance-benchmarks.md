# afterglow-engine Performance Measurements

> Date: 2026-07-09
> Hardware: NVIDIA Ampere (RTX 3070), NixOS Linux, X11/XWayland
> Software: CEF 149 (Chromium 149), Rust 1.98 nightly, wasm32-unknown-unknown +atomics

## Architecture

There is **one communication mechanism**: `SharedArrayBuffer` ring buffers via
the `afterglow-rpc::RingBuffer`. This works identically on both targets
because CEF IS Chromium — both support Web Workers + `SharedArrayBuffer`.

```
┌─────────────────────────────────────────────────────┐
│  Web page (JS / Three.js)                            │
│  ┌───────────────┐     ┌─────────────────────────┐  │
│  │ Main thread    │◄───►│ Web Worker (wasm)       │  │
│  │ requestAnim-   │ SAB │ RingBuffer write/read   │  │
│  │ ationFrame     │     │                         │  │
│  │ Three.js render│     │ ┌────────────────────┐ │  │
│  └───────────────┘     │ │ Web Worker (wasm)   │◄┼──┤
│         ▲               │ │ RingBuffer write/read│ │  │
│         │               │ └────────────────────┘ │  │
│         │ SAB           └─────────────────────────┘  │
│         │ (shared wasm memory)                       │
└─────────┼───────────────────────────────────────────┘
          │
     WebGPU / GPU
```

- **Website ↔ Worker**: `RingBuffer` on `SharedArrayBuffer`-backed wasm memory
- **Worker ↔ Worker**: same `RingBuffer`, different ring buffer instances in the same shared memory
- **No IPC, no serialization, no copies** — just atomic memory writes

---

## 1. Ring Buffer (SharedArrayBuffer, cross-thread)

**Path**: Web Worker → Main thread (same browser tab)
**Mechanism**: `SharedArrayBuffer`-backed `WebAssembly.Memory` + `AtomicU32`
Acquire/Release. Zero-copy.

| Payload | Latency | Throughput |
|---------|---------|------------|
| 64 B | 4.4 µs | 14 MB/s |
| 256 B | 4.0 µs | 55 MB/s |
| 1 KB | 4.3 µs | 228 MB/s |
| 4 KB | 4.0 µs | 909 MB/s |

**~4µs per write+read round-trip** — dominated by function call overhead,
not data transfer. True zero-copy (no V8 sandbox issue on web).

## 2. Ring Buffer RPC (same-process, native threads)

**Path**: Worker thread → Worker thread (same OS process, native)
**Mechanism**: `afterglow-rpc::RingBuffer` on heap memory + postcard codec

| Payload | Latency | Throughput |
|---------|---------|------------|
| 0 B | 2 µs | — |
| 64 B | 4 µs | 32 MB/s |
| 256 B | 8 µs | 59 MB/s |
| 1 KB | 28 µs | 69 MB/s |
| 4 KB | 108 µs | 72 MB/s |
| 16 KB | 428 µs | 73 MB/s |
| 64 KB | 1.73 ms | 73 MB/s |

Throughput plateaus at ~72 MB/s — limited by postcard serialize/deserialize +
Vec allocations, not memory bandwidth.

## 3. Input→Present Latency (CDP tracing)

**Path**: CDP `Input.dispatchMouseEvent` → `SkiaRenderer::SwapBuffers`

| Stat | Value |
|------|-------|
| min | 0.07 ms |
| median | 3.71 ms |
| mean | 2.87 ms |
| p90 | 6.63 ms |
| max | 6.83 ms |

**Present rate**: 144 fps (6.95ms interval, matching monitor refresh rate).

---

## How to Reproduce

### Ring Buffer (SharedArrayBuffer)
```sh
# Build wasm
cargo build -p afterglow-web --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort --profile wasm-dev
# Run in CEF
nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
# Navigate to afterglow://local/web-test.html
```

### Ring Buffer RPC (same-process)
```sh
cargo run --example bench_rpc -p afterglow-rpc-demo
```

### Input→Present Latency
```sh
nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11" &
./target/debug/latency-tool
```
