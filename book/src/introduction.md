# Introduction

**afterglow-engine** is a web-based game engine. You write the render loop in
JavaScript (Three.js + WebGPU) and the heavy computation in Rust workers, and
the two communicate over a lock-free ring buffer.

This book is a **user guide**. It covers how to *use* the engine: the APIs you
call, the build steps you run, and the developer workflow. It is not an
architecture document — for the internal design and the research behind each
decision, see `docs/api/` and `docs/research/` in the repository.

## What you do with it

1. **Open a game window** with the native CEF shell — WebGPU on the real GPU,
   assets served from a custom scheme, no HTTP server.
2. **Define worker services** in Rust with the `#[rpc]` macro — physics, AI,
   asset decoding, anything heavy — and call them from the page as if local.
3. **Build** for desktop (CEF) or the web (wasm + `SharedArrayBuffer`) — the
   same worker code runs on both.

## The two things you'll touch

- **`AppBuilder`** — the native window API. One builder, one `.run()`:
  ```rust
  AppBuilder::new()
      .title("my game")
      .size(1920, 1080)
      .index_html(include_bytes!("index.html"))
      .run();
  ```
- **`#[rpc]`** — the worker service macro. One trait generates a typed client,
  a native spawn constructor, and the wasm exports a Web Worker calls:
  ```rust
  #[rpc(worker = PhysicsWorker)]
  pub trait Physics {
      fn step(state: Vec<f32>, dt: f32) -> Vec<f32>;
  }
  ```

## How to read this book

- **Setup** — get the toolchain, run the example to confirm it works.
- **The Game Window** — the `AppBuilder` API: windows, assets, graphics.
- **Worker Services** — the `#[rpc]` macro and the native/web worker APIs.
- **Building** — native and web build steps, benchmarking.
- **Guides** — end-to-end walkthroughs.
- **Reference** — crate map, debugging, deeper reading.

> When this book and the `docs/api/*.md` files disagree, `docs/api/` is
> canonical — it is checked against the current source. This book summarizes
> for usability; the API docs are the precise reference.
