# Conformance, test, and release gates

## Commands

```sh
cargo run -p xtask conformance
cargo run -p xtask test
cargo run -p xtask release-gate
```

On NixOS, run them inside `nix-shell shell.nix` so native C++ test libraries are
available.

`conformance` validates the authored artifact/page manifest, zero-tolerance demo
architecture, strict TypeScript and source-hygiene ratchets, allocation effects,
single-Three bundle identity, and generated JavaScript drift.

`test` runs workspace Rust tests, raw RPC transport tests, conformance, contract
tooling tests, and all authored engine/Bun tests. CI invokes the canonical
`conformance` and `test` commands; merge-base checks separately prevent edits
from expanding the remaining TypeScript or source-hygiene baselines.

`release-gate` runs `test`, validates `docs/benchmarks/release-evidence.json`, and
builds the mdBook. Packaging must not bypass it.

## Release evidence

Evidence schema version 1 records:

- one successful real-GPU result for every manifest `visual-demo`;
- the exact generated JavaScript artifact name and SHA-256;
- capture timestamp, adapter, and driver identity;
- Dungeon `stable`, `traverse`, and `thrash` soaks of at least 600 seconds;
- zero soak errors, queue overflow, and pending work at completion.

Evidence expires after 30 days and immediately becomes stale when a generated
visual artifact changes. `scripts/check-release-evidence.ts` performs validation.
Missing evidence is a deterministic release-gate failure, not a warning.
