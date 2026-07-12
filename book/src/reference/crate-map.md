# Crate Map

The workspace's crates and how they relate. The `Cargo.toml` is the source of
truth.

| Crate | Purpose |
|---|---|
| `afterglow-rpc` | Core: `RingBuffer`, `Transport` trait, postcard codec, `Response` envelope. The `native` module has `RingStorage`, `spawn_worker_loop`, `run_worker_loop`, and event rings. |
| `afterglow-rpc-macros` | The `#[rpc]` proc macro: generates the server trait, typed Rust client, dispatch, native spawn, and thin wasm exports. |
| `afterglow-rpc-demo` | Demo `Physics` service + `bench_rpc` stress test. The reference example of a `#[rpc(worker = ...)]` service. |
| `afterglow-assets` | Shared asset-path/MIME helpers (`AssetRoot`, `guess_mime`, `resolve`). The single security boundary for FS asset confinement. |
| `afterglow-web` | Wasm target: page-side `#[no_mangle]` exports over two static rings in shared wasm memory. Includes `worker.js`, `rpc.js`, and the `coep_server` example. |
| `afterglow-cef` | Thin CEF shell: window + WebGPU flags + `afterglow://` scheme + COOP/COEP headers. No worker code, no IPC, no input. |
| `latency-tool` | CDP-based input→present latency measurement. |
| `xtask` | Build orchestrator: `build`, `wasm`, `check`, `test`, `bench`. |

## How they relate

```mermaid
flowchart TB
  rpc["afterglow-rpc<br/>(ring buffer + Transport)"]
  mac["afterglow-rpc-macros<br/>(#[rpc])"]
  demo["afterglow-rpc-demo<br/>(Physics service)"]
  web["afterglow-web<br/>(page wasm + JS)"]
  cef["afterglow-cef<br/>(window + scheme)"]
  assets["afterglow-assets<br/>(confinement + MIME)"]

  rpc --> mac
  mac -->|"generates #[rpc] code"| demo
  demo --> web
  web --> cef
  cef -->|"serves afterglow-web's www/ + your page"| assets
  assets -.->|"shared by cef + web dev server"| web
```

- **`afterglow-rpc`** is the foundation: the ring primitive, the `Transport`
  trait, the postcard codec, the `Response` envelope.
- **`afterglow-rpc-macros`** turns a trait into a server + client + (optionally)
  native spawn / wasm exports.
- **`afterglow-rpc-demo`** is the canonical example: a `Physics` service defined
  once, runnable as a native thread or a wasm worker.
- **`afterglow-web`** is the web transport — page-side wasm exports + the JS
  worker/client.
- **`afterglow-cef`** is the native shell — a window with WebGPU, the
  `afterglow://` scheme, and COOP/COEP.
- **`afterglow-assets`** is the shared security boundary for serving filesystem
  assets, used by both the CEF scheme handler and the web dev server.

## The `xtask` orchestrator

```sh
cargo run -p xtask build   # build the native CEF host + examples
cargo run -p xtask wasm    # build afterglow-web + afterglow-rpc-demo to wasm,
                           #   copy artifacts into crates/afterglow-web/www/
cargo run -p xtask check   # cargo check the whole workspace
cargo run -p xtask test    # cargo test --workspace + node --test on rpc.test.mjs
cargo run -p xtask bench   # run the native ring buffer stress test
```

## Where to start in the code

| You want to | Look at |
|---|---|
| Configure a window | `crates/afterglow-cef/src/config.rs` (`AppBuilder`) |
| Understand asset serving | `crates/afterglow-cef/src/resources.rs`, `crates/afterglow-assets/` |
| See a worker service | `crates/afterglow-rpc-demo/src/lib.rs` |
| Use the `#[rpc]` macro | `crates/afterglow-rpc-macros/src/lib.rs` |
| Call a worker from JS | `crates/afterglow-web/www/rpc.js`, `worker.js` |
| Build wasm | `xtask/src/main.rs`, `.cargo/config.toml` |
