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
| `docs/api/cef-shell.md` | `afterglow-cef` game-window shell: `AppBuilder`, `afterglow://` scheme, WebGPU/X11 flags, COOP/COEP, console, startup caveat. |
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
| `docs/research/steam-audio-browser.md` | Dynamic Steam Audio on the 6800U: Web Worker and native OS-worker comparisons. Native balance uses two simulation threads and 64 reflection slots; the real million-triangle Bistro stress scene requires four threads/30 Hz and proves structural acoustic proxies are mandatory. |

## `AGENTS.md`

The repository root `AGENTS.md` is the engineer-facing charter: project
direction, the full build incantations, and the debugging playbook. Reach for it
when you need the exact command or the rationale behind a constraint.

## The code

When in doubt, the code is authoritative. Start from the examples:
`crates/afterglow-cef/examples/minimal.rs` (the CEF app),
`crates/afterglow-web/examples/coep_server.rs` (the web dev server),
`crates/afterglow-rpc-demo/examples/bench_rpc.rs` (the benchmark), and
`crates/afterglow-rpc-demo/src/lib.rs` (the reference `#[rpc]` service).
