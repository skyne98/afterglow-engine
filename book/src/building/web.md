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

### Production

Configure your real web server (nginx, a CDN, etc.) to send the same three
headers on every response from your game's origin.

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
