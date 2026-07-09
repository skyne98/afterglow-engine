# afterglow-engine Performance — Ring Buffer Stress Test

> Date: 2026-07-09
> Hardware: NVIDIA Ampere (RTX 3070), NixOS Linux, X11/XWayland
> Software: CEF 149 (Chromium 149), Rust 1.98 nightly, wasm32 +atomics

## Native (native threads + heap ring buffer)

Ring buffer capacity: 8 MiB, iterations: 2000 per size.

### Direction 1: Main → Worker (write)

| Payload | Latency | Bandwidth | Writes/s |
|---------|---------|-----------|----------|
| 64 B | 0.2 µs | 373 MB/s | 6,110,191 |
| 256 B | 0.1 µs | 2,481 MB/s | 10,162,395 |
| 1 KB | 0.3 µs | 3,830 MB/s | 3,921,653 |
| 4 KB | 0.8 µs | 4,828 MB/s | 1,236,060 |
| 16 KB | 1.8 µs | 8,786 MB/s | 562,316 |
| 64 KB | 6.0 µs | 10,364 MB/s | 165,830 |
| 256 KB | 4.9 µs | 50,651 MB/s | 202,603 |
| 1 MB | 46.9 µs | 21,331 MB/s | 21,331 |

### Direction 2: Worker → Main (read)

| Payload | Latency | Bandwidth | Reads/s |
|---------|---------|-----------|---------|
| 64 B | 0.1 µs | 921 MB/s | 15,082,729 |
| 256 B | 0.2 µs | 1,474 MB/s | 6,038,100 |
| 1 KB | 0.3 µs | 3,503 MB/s | 3,587,129 |
| 4 KB | 0.3 µs | 11,249 MB/s | 2,879,687 |
| 16 KB | 1.1 µs | 14,537 MB/s | 930,339 |
| 64 KB | 1.5 µs | 41,265 MB/s | 660,237 |
| 256 KB | 13.4 µs | 18,719 MB/s | 74,876 |
| 1 MB | 24.9 µs | 40,197 MB/s | 40,197 |

### Round-trip: Main → Worker → Main

| Payload | Latency | Bandwidth | Calls/s |
|---------|---------|-----------|---------|
| 64 B | 0.3 µs | 355 MB/s | 2,904,663 |
| 256 B | 1.0 µs | 493 MB/s | 1,009,809 |
| 1 KB | 1.2 µs | 1,577 MB/s | 807,564 |
| 4 KB | 1.1 µs | 7,083 MB/s | 906,671 |
| 16 KB | 5.3 µs | 5,869 MB/s | 187,810 |
| 64 KB | 10.1 µs | 12,416 MB/s | 99,324 |
| 256 KB | 24.3 µs | 20,590 MB/s | 41,181 |
| 1 MB | 84.5 µs | 23,655 MB/s | 11,828 |

## Web (SharedArrayBuffer + wasm ring buffer)

Ring buffer capacity: 4 MiB per buffer, iterations: 200 per size.

### Direction 1: Write (write+drain)

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 64 B | 1.3 µs | 48 MB/s |
| 256 B | 1.0 µs | 257 MB/s |
| 1 KB | 0.9 µs | 1,085 MB/s |
| 4 KB | 1.0 µs | 3,906 MB/s |
| 16 KB | 4.0 µs | 3,956 MB/s |
| 64 KB | 2.4 µs | 25,773 MB/s |
| 256 KB | 9.1 µs | 27,548 MB/s |
| 1 MB | 29.1 µs | 34,394 MB/s |

### Direction 2: Read (write+read response)

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 64 B | 1.8 µs | 34 MB/s |
| 256 B | 0.8 µs | 305 MB/s |
| 1 KB | 1.7 µs | 583 MB/s |
| 4 KB | 1.6 µs | 2,480 MB/s |
| 16 KB | 1.3 µs | 12,255 MB/s |
| 64 KB | 4.2 µs | 14,970 MB/s |
| 256 KB | 8.7 µs | 28,818 MB/s |
| 1 MB | 28.7 µs | 34,874 MB/s |

### Round-trip (write request + read + write response + read response)

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 64 B | 1.5 µs | 83 MB/s |
| 256 B | 1.5 µs | 326 MB/s |
| 1 KB | 1.4 µs | 1,371 MB/s |
| 4 KB | 3.0 µs | 2,604 MB/s |
| 16 KB | 4.0 µs | 7,813 MB/s |
| 64 KB | 7.1 µs | 17,668 MB/s |
| 256 KB | 17.3 µs | 28,944 MB/s |
| 1 MB | 58.6 µs | 34,144 MB/s |

## Input→Present Latency (CDP tracing)

| Stat | Value |
|------|-------|
| min | 0.07 ms |
| median | 3.71 ms |
| mean | 2.87 ms |
| p90 | 6.63 ms |
| max | 6.83 ms |
| Present rate | 144 fps |

## How to Reproduce

```sh
# Native (native threads + heap ring buffer)
cargo run --example bench_rpc -p afterglow-rpc-demo

# Web (SharedArrayBuffer + wasm)
nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
# Navigate to afterglow://local/bench.html
# Results appear in stderr ([console] [bench] ...)

# Input→present latency
./target/debug/latency-tool
```

## Cross-Thread Worker (Web Worker + SAB, with Atomics.wait/notify)

**Path**: Main thread → SharedArrayBuffer ring buffer → Web Worker (own wasm memory) → SAB → Main thread
**Mechanism**: JS worker reads from SAB via `DataView` + `Atomics`, calls `wasm_serve_frame` in its own wasm instance (allocations safe — own heap), writes response back to SAB. Main thread notified via `Atomics.notify`.

| f32 count | Payload | Latency | Bandwidth |
|-----------|---------|---------|-----------|
| 1 | 4 B | 31.7 µs | 0.2 MB/s |
| 4 | 16 B | 15.8 µs | 1.9 MB/s |
| 16 | 64 B | 20.7 µs | 5.9 MB/s |
| 64 | 256 B | 48.6 µs | 10.0 MB/s |
| 256 | 1 KB | 6.5 µs | 302.8 MB/s |
| 1024 | 4 KB | 6.6 µs | 1183.7 MB/s |
| 4096 | 16 KB | 9.1 µs | 3424.7 MB/s |
| 16384 | 64 KB | 16.9 µs | 7418.4 MB/s |

The worker has its OWN wasm memory — `serve`'s allocations (postcard decode/encode) are completely isolated. No allocator conflict. `Atomics.wait`/`notify` provides near-zero-latency wake-up (vs ~4ms with `setTimeout(0)`).

### Comparison: Native vs Web Worker round-trip

| Payload | Native | Web Worker |
|---------|--------|------------|
| 64 B | 0.3 µs | 20.7 µs |
| 256 B | 1.0 µs | 48.6 µs |
| 1 KB | 1.2 µs | 6.5 µs |
| 4 KB | 1.1 µs | 6.6 µs |
| 16 KB | 5.3 µs | 9.1 µs |
| 64 KB | 10.1 µs | 16.9 µs |

Native is ~2-70× faster for small payloads (OS thread wake-up is cheaper than Web Worker `Atomics.wait`). For large payloads (16KB+), the gap narrows to ~2× (memcpy dominates). Both are well within the 16.7ms frame budget.
