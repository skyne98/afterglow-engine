# Benchmarking & Latency

Two benchmarks measure the worker transports; `FrameBench` measures rAF
production intervals; the `latency-tool` measures input→present on a running CEF
app.

## Frame production benchmark

`FrameBench` reserves fixed timestamp and sorting buffers at construction.
`tick(timestamp)` only records intervals in the frame path. Once capture is
complete, diagnostic code calls `finish()` to sort and calculate
p50/p90/p99/max outside the hot path. Invalid sample counts return a typed status
instead of growing storage, and one result object is reused across runs.

## Native service-RPC benchmark

```sh
nix-shell shell.nix --run "cargo run --release --example bench_rpc -p afterglow-rpc-demo"
```

`bench_rpc` exercises four directions over native threads + heap-backed
`RingStorage`:

1. **Main → Worker (write)** — main writes, worker drains.
2. **Worker → Main (read)** — worker fills, main reads.
3. **Round-trip** — main writes a request, worker echoes it, main reads (full
   SPSC round trip, minus the serve step).
4. **Service RPC** — a pre-encoded `Physics::step` request through the generated
   native worker transport, including dispatch and result decode.

Payloads sweep `64 B → 1 MiB` (raw ring) and `1 → 16,384 B` (service RPC). See
[Performance](../workers/performance.md) for the headline numbers.

## Web worker benchmark

Build the optimized web artifacts, then serve with COOP/COEP:

```sh
nix-shell shell.nix --run "cargo run -p xtask -- wasm --release"
nix-shell shell.nix --run "cargo run -p xtask -- serve"
```

Open <http://localhost:8787/worker-bench.html>. It runs the same service-RPC
sweep over the `SharedArrayBuffer` transport and reports latency + bandwidth.

## `latency-tool`: input → present

`latency-tool` is a CDP-based diagnostic for the CEF DevTools endpoint. Attach
it to a running CEF app with `.devtools(port)` set:

```sh
# Measure input→present on 127.0.0.1:9222 (the default)
nix-shell shell.nix --run "cargo run -p latency-tool"

# Eval JS in the page
nix-shell shell.nix --run "cargo run -p latency-tool -- eval 'navigator.userAgent'"

# Navigate
nix-shell shell.nix --run "cargo run -p latency-tool -- nav afterglow://local/index.html"
```

- **measure** records Chromium tracing events, dispatches twelve synthetic mouse
  bursts, and reports input-event-to-next-`SkiaRenderer::SwapBuffers` latency
  plus present cadence. CDP input bypasses the OS input stack, so this is a
  reproducible *lower bound* rather than physical-device latency.
- **eval** uses `Runtime.evaluate` with `awaitPromise` and `returnByValue`.
- **nav** enables Page/Network domains, navigates, and prints loading events for
  2.5 s.

> CEF Views browsers don't appear in `/json/list`. `latency-tool` uses
> `Target.getTargets` + `Target.attachToTarget` instead. If `eval` times out,
> the page may be running a synchronous JS task — use `awaitPromise: true` and
> ensure the JS yields to the event loop.

## Methodology notes

- Service-RPC values are the **median of five independent run averages**;
  raw-ring values are the median of three, all under low background load.
- Bandwidth is binary MiB/s; end-to-end bandwidth counts useful payload in both
  directions (`2 × payload`), not framing/codec overhead.
- Latency is round-trip wall time per call, not one-way.
- Small-payload results vary with scheduler wake-up noise — trust medians.

For the complete tables, see `docs/research/performance-benchmarks.md`.
