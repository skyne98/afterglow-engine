# Web (Wasm)

Build the wasm artifacts and serve them with COOP/COEP. The same `#[rpc]`
service runs in the browser over a `SharedArrayBuffer`.

## Build the artifacts

### Dev (via xtask)

```sh
nix-shell shell.nix --run "cargo run -p xtask wasm"
```

`xtask wasm` builds `afterglow-web` and worker crates for
`wasm32-unknown-unknown` with `-Zbuild-std` and the shared-memory flags. It
stages byte-identical wasm files in `crates/afterglow-web/web/assets/`, refreshes
generated clients in `web/src/workers/`, and runs `scripts/build-web.ts`.

The web tree has one-way ownership:

| Directory | Contents |
|---|---|
| `web/src/engine/` | Authored engine TypeScript, organized by subsystem |
| `web/src/demos/` | Pure game/presentation examples |
| `web/src/workers/` | Worker runtime and generated typed clients |
| `web/public/` | Authored HTML and static public files |
| `web/assets/` | Cooked assets and wasm staging inputs |
| `web/contracts/` | Deployment, architecture, and allocation contracts |
| `www/` | Disposable generated deployment; never author files here |

`bun scripts/build-web.ts` deletes and reconstructs `www/`. `--check` compares
the complete staged tree, so missing, stale, and undeclared deployment files all
fail conformance.

### Optimized (for shipping/benchmarking)

```sh
cargo build -p afterglow-web --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort --profile wasm-release
cargo build -p afterglow-rpc-demo --target wasm32-unknown-unknown \
  -Zbuild-std=core,alloc,std,panic_abort --profile wasm-release
```

Then copy `afterglow_web.wasm` and `afterglow_rpc_demo.wasm` (from
`target/wasm32-unknown-unknown/wasm-release/`) into your served directory.

## The shared-memory config

The wasm modules must use **imported shared memory**. The workspace
`.cargo/config.toml` supplies this:

```toml
[target.wasm32-unknown-unknown]
rustflags = [
  '-C', 'target-feature=+atomics,+bulk-memory,+mutable-globals',
  '-C', 'link-arg=--import-memory',
  '-C', 'link-arg=--max-memory=67108864',
  '-C', 'link-arg=--shared-memory',
]
```

> **The `.cargo/config.toml` must be at the WORKSPACE ROOT.** Cargo searches
> *up* from the crate dir, not down. A config inside a subcrate is silently not
> found, and the wasm module creates its own non-shared memory —
> `SharedArrayBuffer` then won't work. The JS memory's `maximum` must be ≤ the
> module's `--max-memory` (64 MiB).

## Serve with COOP/COEP

`SharedArrayBuffer` requires a **cross-origin isolated** page. Every response
must carry:

```http
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
Cross-Origin-Resource-Policy: same-origin
```

### Dev server

```sh
nix-shell shell.nix --run "cargo run -p xtask -- serve"
```

Serves `crates/afterglow-web/www/` on <http://localhost:8787> with the headers,
four fixed workers, and sixteen bounded queued connections per worker.
Open <http://localhost:8787/worker-test.html> to verify the round trip. Check
in the console: `self.crossOriginIsolated === true`.

### Caddy origin (HTTP/1.1, HTTP/2, HTTP/3)

`deploy/web/Caddyfile` is the static-origin configuration. It applies the
three isolation headers and `Accept-Ranges: bytes`, keeps HTTP/1.1 persistent,
and explicitly enables `h1 h2 h3`. HTTP/2/3 multiplex ranges over one
connection per player; HTTP/3 uses QUIC's negotiated idle timeout rather than
an HTTP `Connection: keep-alive` header.

For the local single-client gate:

```sh
nix-shell shell.nix --run \
  'caddy run --config deploy/web/Caddyfile --adapter caddyfile'
```

This defaults to `https://localhost:8443` with Caddy's local CA and deliberately
has no privileged port-80 redirect listener. A public origin sets
`AFTERGLOW_WEB_ADDRESS` to its DNS hostname, uses Caddy's normal ACME TLS, and
must expose both TCP and UDP on its HTTPS port. QUIC-blocked clients fall back
to HTTP/2 or HTTP/1.1. `coep_server` remains development-only; never deploy its
four blocking workers as the public asset server.

> **Subresource caveat:** `Cross-Origin-Embedder-Policy: require-corp` means
> every cross-origin resource must opt in with
> `Cross-Origin-Resource-Policy: cross-origin` (or be loaded with `crossorigin`
> and CORs-enabled). Host Three.js, textures, and wasm same-origin when you can.

## The page

```html
<script type="module" src="./game.js"></script>
```

Author and bundle `game.ts`; HTML never contains authored inline JavaScript:

```ts
import * as THREE from 'three/webgpu';
import { PhysicsClient } from './workers/physics.client.ts';

const physics = await PhysicsClient.spawn({ workerWasmUrl: 'physics_worker.wasm' });
const result = await physics.step(new Float32Array([0, 1, 2]), 0.5);
physics.close();
```

See [Web Workers](../workers/web-workers.md) for the full `Rpc` transport API
and the generated TS client.

## Next

- [Web Workers](../workers/web-workers.md) — the JS client API.
- [Your First Worker](../guides/first-worker.md) — define + call a service
  end-to-end.
