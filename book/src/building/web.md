# Web (Wasm)

Build the wasm artifacts and serve them with COOP/COEP. The same `#[rpc]`
service runs in the browser over a `SharedArrayBuffer`.

## Build the artifacts

### Dev (via xtask)

```sh
nix-shell shell.nix --run "cargo run -p xtask wasm"
```

`xtask wasm` builds `afterglow-web` and `afterglow-rpc-demo` to
`wasm32-unknown-unknown` with `-Zbuild-std` and the shared-memory flags, then
copies them deterministically into `crates/afterglow-web/www/`:

| Build output | Copied to |
|---|---|
| `afterglow_web.wasm` | `www/afterglow_web.wasm` |
| `afterglow_rpc_demo.wasm` | `www/physics_worker.wasm` |

The copies are byte-identical to the `target/` artifacts.

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
<script type="module">
  import * as THREE from '/three.webgpu.js';
  import { Rpc } from '/rpc.js';
  import { PhysicsClient } from '/physics.client.js';

  const transport = await Rpc.create({
    mainWasmUrl: '/afterglow_web.wasm',
    workerJsUrl: '/worker.js',
    workerWasmUrl: '/physics_worker.wasm',
    timeoutMs: 5000,
  });

  const physics = new PhysicsClient(transport);
  const result = await physics.step(new Float32Array([0, 1, 2]), 0.5);
  // Float32Array [0.5, 1.5, 2.5]
</script>
```

See [Web Workers](../workers/web-workers.md) for the full `Rpc` transport API
and the generated TS client.

## Next

- [Web Workers](../workers/web-workers.md) — the JS client API.
- [Your First Worker](../guides/first-worker.md) — define + call a service
  end-to-end.
