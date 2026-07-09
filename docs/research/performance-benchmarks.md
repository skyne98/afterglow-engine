# afterglow-engine Performance

> Date: 2026-07-09
> Hardware: NVIDIA Ampere (RTX 3070), NixOS Linux, X11/XWayland
> CEF 149 (Chromium 149), Rust 1.98 nightly, wasm32 +atomics

## 1. Native Ring Buffer (threads + heap, 8 MiB, 2000 iterations)

### Main → Worker (write)

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 64 B | 0.1 µs | 888 MB/s |
| 256 B | 0.1 µs | 2,234 MB/s |
| 1 KB | 0.1 µs | 9,116 MB/s |
| 4 KB | 0.1 µs | 36,043 MB/s |
| 16 KB | 1.0 µs | 16,286 MB/s |
| 64 KB | 5.6 µs | 11,195 MB/s |
| 256 KB | 22.6 µs | 11,043 MB/s |
| 1 MB | 39.4 µs | 25,368 MB/s |

### Worker → Main (read)

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 64 B | 0.2 µs | 353 MB/s |
| 256 B | 0.3 µs | 859 MB/s |
| 1 KB | 0.2 µs | 4,607 MB/s |
| 4 KB | 0.5 µs | 7,829 MB/s |
| 16 KB | 1.0 µs | 15,054 MB/s |
| 64 KB | 1.5 µs | 41,510 MB/s |
| 256 KB | 4.6 µs | 54,323 MB/s |
| 1 MB | 19.8 µs | 50,619 MB/s |

### Round-trip

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 64 B | 0.9 µs | 65 MB/s |
| 256 B | 1.0 µs | 249 MB/s |
| 1 KB | 1.2 µs | 832 MB/s |
| 4 KB | 2.3 µs | 1,710 MB/s |
| 16 KB | 5.5 µs | 2,832 MB/s |
| 64 KB | 5.2 µs | 11,953 MB/s |
| 256 KB | 26.3 µs | 9,503 MB/s |
| 1 MB | 73.5 µs | 13,605 MB/s |

## 2. Web SAB Ring Buffer (wasm, single-thread, 4 MiB, 200 iterations)

### Write (write+drain)

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 64 B | 1.0 µs | 60 MB/s |
| 256 B | 0.9 µs | 279 MB/s |
| 1 KB | 0.9 µs | 1,085 MB/s |
| 4 KB | 1.1 µs | 3,720 MB/s |
| 16 KB | 3.5 µs | 4,529 MB/s |
| 64 KB | 2.5 µs | 24,510 MB/s |
| 256 KB | 7.3 µs | 34,130 MB/s |
| 1 MB | 31.6 µs | 31,671 MB/s |

### Read (write+read)

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 64 B | 1.0 µs | 58 MB/s |
| 256 B | 0.8 µs | 315 MB/s |
| 1 KB | 1.0 µs | 977 MB/s |
| 4 KB | 0.8 µs | 4,735 MB/s |
| 16 KB | 1.3 µs | 12,500 MB/s |
| 64 KB | 2.2 µs | 28,090 MB/s |
| 256 KB | 6.1 µs | 40,650 MB/s |
| 1 MB | 27.9 µs | 35,842 MB/s |

### Round-trip

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 64 B | 1.4 µs | 86 MB/s |
| 256 B | 1.6 µs | 310 MB/s |
| 1 KB | 1.4 µs | 1,395 MB/s |
| 4 KB | 1.6 µs | 5,040 MB/s |
| 16 KB | 2.2 µs | 14,368 MB/s |
| 64 KB | 4.4 µs | 28,571 MB/s |
| 256 KB | 16.5 µs | 30,257 MB/s |
| 1 MB | 57.3 µs | 34,935 MB/s |

## 3. Cross-Thread Worker (Web Worker + SAB, round-trip)

Main thread → SAB ring buffer → Web Worker (own wasm memory, wasm_serve_frame) → SAB → main thread.

| Payload | Latency | Bandwidth |
|---------|---------|-----------|
| 4 B | 1,054 µs | — |
| 16 B | 1,038 µs | — |
| 64 B | 1,040 µs | 0.1 MB/s |
| 256 B | 1,072 µs | 0.5 MB/s |
| 1 KB | 1,045 µs | 1.9 MB/s |
| 4 KB | 1,044 µs | 7.5 MB/s |
| 16 KB | 1,045 µs | 29.9 MB/s |
| 64 KB | 1,057 µs | 118.3 MB/s |

~1ms latency — dominated by the 1ms `Atomics.wait` poll timeout (the worker
checks for requests every 1ms). When `AtomicU32::notify` stabilizes in Rust,
this drops to ~50µs.

## 4. Input→Present (CDP tracing)

| Stat | Value |
|------|-------|
| min | 0.01 ms |
| median | 1.16 ms |
| mean | 2.44 ms |
| p90 | 5.02 ms |
| max | 6.64 ms |
| Present rate | 144 fps (6.95ms interval) |

## Summary

| Path | 64 B | 64 KB | 1 MB |
|------|------|-------|------|
| Native round-trip | 0.9 µs | 5.2 µs | 73.5 µs |
| Web SAB round-trip | 1.4 µs | 4.4 µs | 57.3 µs |
| Cross-thread worker | 1,040 µs | 1,057 µs | — |
| Input→present | — | — | 1.16ms median @ 144fps |
