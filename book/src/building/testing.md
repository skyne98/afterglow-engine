# Testing

Afterglow prioritizes executable behavior across complete feature paths.

```sh
cargo run -p xtask -- test
```

This runs the Rust workspace, generated RPC deployment tests, conformance-tool
tests, and every `*.test.ts` under `crates/afterglow-web/web/src/`. Test discovery
is recursive, so a newly added subsystem, worker, or demo test is included without
editing the build orchestrator.

Tests are organized in four levels:

1. **Unit tests** verify bounded primitives and deterministic failures.
2. **Vertical tests** connect real components, such as BIG parsing through
   session/store ownership, VT feedback through residency scheduling, or RPC
   framing through client lifecycle.
3. **Browser/GPU tests** launch generated pages and real workers or WebGPU:

   ```sh
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
