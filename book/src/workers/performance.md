# Performance

What to expect from the worker transports. Full methodology and numbers live in
`docs/research/performance-benchmarks.md`; this is the summary you need.

## Headline numbers

Host: AMD Ryzen 9 9950X3D, Linux 6.18, Rust nightly 2026-06-30, Chromium 150.
Native `release`; wasm `wasm-release`.

### End-to-end worker service RPC

Both paths call `Physics::step(Vec<f32>, f32) -> Vec<f32>` with a pre-encoded
request. Timing includes ring copies, postcard decode/encode, the worker
method, envelope decode, and caller result decode.

| Payload each way | Native latency | Native bandwidth | Web latency | Web bandwidth |
|---:|---:|---:|---:|---:|
| 64 B | 2.4 µs | 51.6 MiB/s | 10.9 µs | 11.2 MiB/s |
| 1 KiB | 3.4 µs | 574.5 MiB/s | 14.4 µs | 135.8 MiB/s |
| 16 KiB | 21.3 µs | 1,466.6 MiB/s | 34.6 µs | 903.8 MiB/s |
| 64 KiB | 76.4 µs | 1,635.5 MiB/s | 106.5 µs | 1,174.0 MiB/s |

At 64 B, native is about **4.5× lower latency**. At 64 KiB, native is about
**1.4× lower latency / higher useful bandwidth**. Small-payload results vary
with scheduler wake-up noise, so medians (over five run averages) are more
meaningful than a single run.

## What this means

1. Small web service round trips take **~11 µs** (the old ~1 ms was a polling
   timeout, fixed with lossless `postMessage` wake-up).
2. Native service RPC is faster throughout, but the gap narrows as payloads grow
   (4.5× at 64 B → 1.4× at 64 KiB) — the per-call fixed cost (wake-up, dispatch)
   dominates at small sizes; the copy dominates at large sizes.
3. The web worker has separate wasm memory, so calls copy SAB → worker wasm →
   SAB.

> The current demo worker emits a native event from `Physics::step`; wasm does
> not. So the native number slightly understates native transport performance in
> a transport-only comparison.

## Reproduce

See [Benchmarking & Latency](../building/benchmarking.md) for the full
walkthrough. Quick version:

```sh
# Native optimized service-RPC benchmark
cargo run --release --example bench_rpc -p afterglow-rpc-demo
```

For the complete tables and methodology, see
`docs/research/performance-benchmarks.md`.
