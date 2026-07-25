# Further Reading

This book is the user guide. The repository holds two deeper layers of
documentation that this book summarizes rather than duplicates.

## `docs/api/` — source-checked API reference

Precise, up-to-the-source API surface per system. When this book and an API doc
disagree, the API doc is canonical (it is checked against the current source).

| File | Covers |
|---|---|
| `docs/api/ring-buffer.md` | `afterglow-rpc` ring buffer + native transport: SPSC framing, owned halves, worker transport, events, poison/timeout. |
| `docs/api/rpc-macro.md` | `afterglow-rpc-macros` `#[rpc]` attribute: server/typed client/dispatch generation, native spawn + wasm exports, reserved names. |
| `docs/api/web-shared-memory.md` | `afterglow-web` wasm exports, JS client/worker contract, build, and COOP/COEP headers. |
| `docs/api/assets.md` | `afterglow-assets` shared `guess_mime`/`resolve`: path/MIME + canonical confinement. |
| `docs/api/afterglow-shell.md` | The sole native host: presenter, browser environment, lifecycle, validation path, and native host gates. |
| `docs/api/latency-tool.md` | CDP diagnostic CLI commands and measurement semantics. |

## `docs/research/` — decision records

The research notes are the canonical record of *why* each choice was made.

| File | Question / verdict |
|---|---|
| `docs/research/threejs-webgpu-stability.md` | How stable and usable is the Three.js WebGPU renderer? |
| `docs/research/native-runtime-linux-steam.md` | Native runtime options to ship on Linux + Steam. |
| `docs/research/cef-rs-tauri-binding.md` | `tauri-apps/cef-rs` is a mature Rust CEF binding. |
| `docs/research/cef-rs-webgpu-prototype-findings.md` | Built & ran a cef-rs WebGPU prototype. Wayland+Vulkan incompatible in CEF 149 → `--ozone-platform=x11`. |
| `docs/research/cef-games-latency-footprint-debugging.md` | CEF for games: latency pipeline, footprint, debugging. |
| `docs/research/performance-benchmarks.md` | Optimized communication results (full tables + methodology). |
| `docs/research/steam-overlay-cef.md` | How the Steam Overlay works with CEF. |
| `docs/research/steamworks-native-worker.md` | Steamworks as a native Rust worker via `#[rpc(worker = SteamWorker)]`. |
| `docs/research/steam-audio-browser.md` | Dynamic Steam Audio on the 6800U: Web Worker and native OS-worker comparisons. Native Embree kept full-Bistro p99 below 3.90 ms at 512×2. Web obvhs SIMD128 + two pthreads measured 4.47 ms mean / 6.235 ms p99 on 10K geometry; the complete 1.0–2.8M-triangle Bistro package also produced valid output, but package-worst 512×2 p99 rose to 27.88 ms. Structural proxies remain required. |

## `AGENTS.md`

The repository root `AGENTS.md` is the engineer-facing charter: project
direction, the full build incantations, and the debugging playbook. Reach for it
when you need the exact command or the rationale behind a constraint.

## The code

When in doubt, the code is authoritative. Start from the examples:
`crates/afterglow-shell/src/main.rs` (the native shell presenter),
`crates/afterglow-web/examples/coep_server.rs` (the web dev server),
`crates/afterglow-rpc-demo/examples/bench_rpc.rs` (the benchmark), and
`crates/afterglow-rpc-demo/src/lib.rs` (the reference `#[rpc]` service).
