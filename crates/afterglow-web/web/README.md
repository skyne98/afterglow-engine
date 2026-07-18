# Afterglow web workspace

This directory is the authored/staged input to the browser deployment.

- `src/engine/<subsystem>/` — engine TypeScript and colocated unit tests
- `src/workers/` — worker transport plus generated typed clients
- `src/demos/<name>/` — pure game/presentation entrypoints
- `public/` — authored HTML
- `assets/` — cooked assets and wasm staging inputs
- `contracts/` — deployment, architecture, and allocation contracts

`../www/` is generated output. Never add source, tests, package state, manifests,
or vendored libraries there.

```sh
bun install --cwd crates/afterglow-web/web --frozen-lockfile
bun scripts/build-web.ts
bun scripts/build-web.ts --check
cargo run -p xtask -- conformance
```
