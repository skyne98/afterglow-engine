# afterglow-engine communication performance

> Refreshed: 2026-07-10
>
> Host: AMD Ryzen 9 9950X3D (16C/32T), Linux 6.18.38, `powersave` governor
> with normal boost, no CPU affinity
>
> Toolchain: Rust 1.98.0-nightly (2026-06-30), Chromium 150.0.7871.46
>
> Builds: native `release`; wasm `wasm-release` (`opt-level = 2`, shared-memory
> atomics)

## Method

- Service-RPC values are the **median of five independent run averages**;
  raw-ring values are the median of three, all under low background load.
- Service-RPC tests perform 100 warm-up calls followed by 1,000 measured calls
  per payload. Immutable request bytes are encoded once before timing. Timed
  work includes transport, worker dispatch, and caller result decode; every
  result is validated outside the timer and was valid.
- Native raw-ring tests use 10,000 operations per payload. Web local-ring tests
  use 2,000 operations per payload after 1,000 warm-up iterations.
- Bandwidth is binary MiB/s. End-to-end and round-trip bandwidth counts useful
  payload in both directions (`2 × payload`), not framing/codec overhead.
- Latency is round-trip wall time per call, not one-way latency.

## 1. End-to-end worker service RPC

This is the most representative communication comparison. Both paths call
`Physics::step(Vec<f32>, f32) -> Vec<f32>` with a pre-encoded immutable request.
Timing includes request/response ring copies, worker-side postcard argument
decode and result encode, the worker method, response-envelope decode, and
caller result decode. Validation is outside the timer.

- **Native:** generated `PhysicsClient` → heap-backed rings → native worker
  thread, with `park`/`unpark` wake-up.
- **Web:** JS `Rpc` → shared wasm-memory rings → Web Worker, with payload-free
  `postMessage` wake-up. The worker has separate wasm memory, so the path also
  copies SAB → worker wasm → SAB.

| Payload each way | Native latency | Native bandwidth | Web latency | Web bandwidth |
|---:|---:|---:|---:|---:|
| 4 B | 2.2 µs | 3.5 MiB/s | 11.7 µs | 0.7 MiB/s |
| 16 B | 2.1 µs | 14.5 MiB/s | 11.4 µs | 2.7 MiB/s |
| 64 B | 2.4 µs | 51.6 MiB/s | 10.9 µs | 11.2 MiB/s |
| 256 B | 3.0 µs | 161.8 MiB/s | 11.4 µs | 42.7 MiB/s |
| 1 KiB | 3.4 µs | 574.5 MiB/s | 14.4 µs | 135.8 MiB/s |
| 4 KiB | 8.0 µs | 974.5 MiB/s | 16.9 µs | 462.0 MiB/s |
| 16 KiB | 21.3 µs | 1,466.6 MiB/s | 34.6 µs | 903.8 MiB/s |
| 64 KiB | 76.4 µs | 1,635.5 MiB/s | 106.5 µs | 1,174.0 MiB/s |

At 64 B, native is about **4.5× lower latency**. At 64 KiB, native is about
**1.4× lower latency / higher useful bandwidth**. Small-payload results vary
with scheduler wake-up noise, so medians are more meaningful than one run.

The current demo worker emits a native event from `Physics::step`; wasm does
not. Consequently this measures the current real behavior, but slightly
understates native transport performance in a transport-only comparison.

## 2. Native raw ring baseline (cross-thread)

Main thread writes one heap-backed ring, a native thread echoes into a second,
and main reads it. This omits postcard dispatch and the service workload.

| Payload | Round-trip latency | Aggregate bandwidth |
|---:|---:|---:|
| 64 B | 0.2 µs | 569 MiB/s |
| 256 B | 0.9 µs | 567 MiB/s |
| 1 KiB | 1.1 µs | 1,857 MiB/s |
| 4 KiB | 1.4 µs | 5,539 MiB/s |
| 16 KiB | 2.2 µs | 14,334 MiB/s |
| 64 KiB | 4.1 µs | 30,199 MiB/s |
| 256 KiB | 14.9 µs | 33,670 MiB/s |
| 1 MiB | 73.1 µs | 27,357 MiB/s |

Median peak directional throughput was approximately **50,496 MiB/s write**
and **54,696 MiB/s read**. The raw test allocates a `Vec` on native reads, so
it is a ring/allocator baseline rather than an allocation-free upper bound.

## 3. Web local ring baseline (same thread; not worker RPC)

This test calls the optimized wasm ring exports locally. It measures ring
framing/copies without worker scheduling, `postMessage`, postcard, or a service
method. It is useful only as an upper bound on the primitive.

| Payload | Round-trip latency | Aggregate bandwidth |
|---:|---:|---:|
| 64 B | 0.2 µs | 588 MiB/s |
| 256 B | 0.2 µs | 2,537 MiB/s |
| 1 KiB | 0.2 µs | 11,323 MiB/s |
| 4 KiB | 0.3 µs | 29,206 MiB/s |
| 16 KiB | 0.6 µs | 49,020 MiB/s |
| 64 KiB | 3.3 µs | 37,341 MiB/s |
| 256 KiB | 12.3 µs | 40,775 MiB/s |

Median peak directional throughput was approximately **45,290 MiB/s write**
and **53,879 MiB/s read**.

## Conclusions

1. The old ~1 ms web-worker result was caused by a 1 ms polling timeout. Small
   web service round trips now take **~11 µs** with lossless `postMessage` wake-up.
2. The ring primitive itself is not the web bottleneck: local aggregate
   useful-payload throughput reaches **49,020 MiB/s (~47.9 GiB/s)**.
   Service-RPC throughput reaches
   **1,174 MiB/s** at 64 KiB each way despite separate worker wasm memory.
3. Native service RPC is faster throughout, but the gap narrows as payloads
   grow: roughly 4.5× at 64 B and 1.4× at 64 KiB.

## Reproduce

```sh
# Native optimized benchmark
cargo run --release --example bench_rpc -p afterglow-rpc-demo

# Build optimized web artifacts
cargo build -p afterglow-web --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort --profile wasm-release
cargo build -p afterglow-rpc-demo --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort --profile wasm-release

# Serve worker-bench.html and bench.html with COOP/COEP headers after placing
# those release artifacts at afterglow_web.wasm and physics_worker.wasm.
```

## Historical input-to-present result (2026-07-09)

This is retained for context and was **not** rerun during the communication
refresh. On the prior RTX 3070 / CEF 149 setup: minimum 0.01 ms, median 1.16 ms,
mean 2.44 ms, p90 5.02 ms, maximum 6.64 ms at a 144 fps present rate.
