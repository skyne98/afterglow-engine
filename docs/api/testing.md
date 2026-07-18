# Engine testing API and lanes

Afterglow gates observable behavior rather than maintaining generic TypeScript
diagnostic or source-style debt baselines.

## Canonical commands

```sh
cargo run -p xtask -- conformance
cargo run -p xtask -- test
cargo run -p xtask -- release-gate
```

`xtask test` runs the Rust workspace, the generated-deployment RPC transport
tests, focused conformance contracts, and `bun test crates/afterglow-web/web/src`.
The recursive Bun invocation discovers every colocated `*.test.ts`; no test path
allowlist is maintained.

## Test levels

- **Unit tests** cover one bounded primitive, codec, parser, queue, allocator, or
  state transition and its deterministic failure behavior.
- **Vertical-integration tests** cross the real ownership boundary for a feature.
  Existing examples include BIG header/session/worker/store lifecycle,
  runtime/renderer registration and sealing, VT feedback-to-residency scheduling,
  generated RPC transport lifecycle, and rigged packed-asset composition.
- **Browser end-to-end tests** instantiate the generated deployment and real
  worker or renderer. `worker-test.html` covers the shared-memory worker path;
  `scripts/test-{dungeon,rigged-vt,vt,lod}-gpu.sh` cover the visual paths.
- **Soak and release evidence** prove bounded queues, stable memory, current
  artifact hashes, and real-GPU behavior. `release-gate` validates the recorded
  evidence before packaging.

A regression should be captured at the cheapest level that reproduces it. Changes
to subsystem ownership, transport, asset flow, rendering, or lifecycle also need
a vertical test proving the complete path, not only tests of internal helpers.

Focused architecture, allocation-effect, artifact, and import-boundary contracts
remain because they express engine invariants not reliably observable in short
runtime tests. Generic compiler diagnostics and stylistic escape-hatch inventories
are not conformance or release gates.
