# Testing

Afterglow prioritizes executable behavior across complete feature paths. The
repository is currently `converging`: canonical demo source architecture is
separate from visual and release conformance.

```sh
cargo run -p xtask -- test
```

This runs the Rust workspace, generated RPC deployment tests, the
`afterglow-shell` DOM API contract, conformance-tool tests, every `*.test.ts`
under `crates/afterglow-web/web/src/`, and the Steam Audio WASM prototype tests.
Web test discovery is recursive, so a newly added subsystem, worker, or demo
test is included without editing the orchestrator. The unified telemetry
mechanism can be gated directly with:

```sh
cargo test -p afterglow-telemetry
cargo clippy -p afterglow-telemetry --all-targets -- -D warnings
cd crates/afterglow-web/web && bun test src/engine/telemetry/telemetry.test.ts
```

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
   ```

   Historical per-demo `scripts/test-*-gpu.sh` commands are absent and are not
   valid lanes. A single manifest-driven `xtask visual` command is the required
   replacement and remains a convergence gate.

4. **Release evidence** schema v2 requires each visual demo on web and native,
   current artifact and screenshot hashes, `GameReady`, coherent resize
   dimensions, semantic plus tolerant-reference pixel checks, frame/resource/
   queue results, and 30-minute plateaued soak scenarios. Version-one boolean
   success records are rejected. Validate recorded evidence with:

   ```sh
   cargo run -p xtask -- release-gate
   ```

Generic TypeScript diagnostic and source-style debt baselines are not gates.
Focused architecture, allocation, import-boundary, and deployment contracts remain
because they enforce engine invariants that short runtime tests cannot prove.
