# Testing

Afterglow prioritizes executable behavior across complete feature paths.

```sh
cargo run -p xtask -- test
```

This runs the Rust workspace, generated RPC deployment tests, the
`afterglow-shell` DOM API contract, conformance-tool tests, every `*.test.ts`
under `crates/afterglow-web/web/src/`, and the Steam Audio WASM prototype tests.
Web test discovery is recursive, so a newly added subsystem, worker, or demo
test is included without editing the orchestrator.

Tests are organized in four levels:

1. **Unit tests** verify bounded primitives and deterministic failures.
2. **Vertical tests** connect real components, such as BIG parsing through
   session/store ownership, VT feedback through residency scheduling, or RPC
   framing through client lifecycle.
3. **Browser/GPU tests** launch generated pages and real workers or WebGPU.
   The native shell additionally runs unmodified official Three.js documents
   through its deterministic `browser_test` executable:

   ```sh
   cargo build -p afterglow-shell --example browser_test
   ./target/debug/examples/browser_test /tmp/threejs webgpu_materials_basic /tmp/out.png

   DISPLAY=:0 ./scripts/test-dungeon-gpu.sh
   DISPLAY=:0 ./scripts/test-rigged-vt-gpu.sh
   DISPLAY=:0 ./scripts/test-vt-gpu.sh
   DISPLAY=:0 ./scripts/test-lod-gpu.sh
   ```

4. **Release evidence** records current artifact hashes, adapter identity, every
   visual demo, and the required Dungeon soaks. Validate it with:

   ```sh
   cargo run -p xtask -- release-gate
   ```

Generic TypeScript diagnostic and source-style debt baselines are not gates.
Focused architecture, allocation, import-boundary, and deployment contracts remain
because they enforce engine invariants that short runtime tests cannot prove.
