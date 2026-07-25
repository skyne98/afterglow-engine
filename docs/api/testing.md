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
tests, the afterglow-shell DOM API contract, focused conformance contracts, and recursive Bun tests under
`crates/afterglow-web/web/src` plus the Steam Audio WASM prototype. Every
colocated web `*.test.ts` is discovered without a subsystem/demo path allowlist.

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
- **Native shell compatibility tests** use `afterglow-shell`'s `browser_test`
  example to execute official Three.js HTML/module/addon code unchanged and
  compare PNG output. The windowed smoke lane runs the same documents through
  the direct winit/wgpu presenter; its HUD path remains GPU-only.
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

## Open asset/VT gate gaps (2026-07-22)

The existing tests prove the source-sorting `createPageRangeReader()` helper and
the bounded `BigAssetSession` provider independently, but no vertical test proves
that the live session provider uses the helper—it currently does not. The
950.2 MiB/s CEF result is an explicitly sorted transport diagnostic, not that
missing vertical gate. Add a shuffled live-provider test that asserts source
ordering, adjacent `pread` collapse, caller-order restoration, and response
bounds before promoting the benchmark to gameplay evidence.

CEF GPU scripts also do not currently reject `texture.wasm` Web Worker startup.
A mandatory target-boundary gate must prove that CEF starts the generated native
`afterglow-texture` client from `AppBuilder::on_ready`, and that public web still
uses the generated WASM worker. The CEF release artifact/startup path must fail
if an engine service with a native implementation is instantiated as WASM.
The same target-boundary gate is required for `afterglow-shell` before it can
replace CEF: native engine services must be generated native clients backed by
OS workers, never the public-web WASM worker path.
