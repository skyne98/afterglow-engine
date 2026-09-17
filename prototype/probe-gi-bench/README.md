# Probe-GI BVH traversal benchmark

Raw WebGPU compute benchmark that measured whether dynamic probe GI is affordable
on the AMD Radeon 680M. Results and interpretation:
`docs/research/probe-based-gi.md` §7.3.

It builds an axis-aligned-box scene, a median-split binary BVH (leaf 4), and a
closest-hit traversal kernel shaped like a DDGI probe update: P probes x 64
spherical-Fibonacci rays, with and without a hit-shading evaluation. Before
timing it validates GPU traversal against a CPU brute-force reference for a
sample of rays.

## Run

The native shell evaluates the page directly (no web build, no server):

```sh
nix-shell shell.nix --run \
  "AFTERGLOW_STARTUP_TIMEOUT_MS=230000 RUST_LOG=warn stdbuf -oL -eL \
   ./target/release/afterglow-shell prototype/probe-gi-bench/index.html"
```

Results are printed to stdout (one row per configuration) and mirrored into the
page. The run ends with a startup-timeout panic because the page never signals
readiness to the shell lifecycle — that is expected and happens after `DONE`.

## Method notes

- 25 dispatches per configuration, one fence at the end. Per-dispatch fencing
  inflates results by several milliseconds (see the doc's §C.6); do not lower the
  iteration count to "save time".
- It requests a default device. The engine's device requests far higher limits
  (`maxStorageBufferBindingSize` 2 GiB vs the 128 MiB default here), so a
  production kernel must request limits explicitly.
- This is a binary-BVH proxy for cost estimation, not the CWBVH8 traversal the
  engine will ship: see `docs/research/probe-gi-placement-leaks-packing.md` §C.1
  for the exact 80-byte node layout to port.
