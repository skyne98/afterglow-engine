# afterglow-cef API — Cross-Process Shared Memory

> Status: **working** — verified end-to-end on CEF 149 (Chromium 149), NVIDIA/Ampere, Linux/X11.

## Overview

Cross-process data transfer from the browser process (native game loop) to the
renderer process (JS/Three.js) via `CefSharedMemoryRegion`. Zero serialization
— the browser writes raw bytes to shared memory, the renderer reads them and
copies them into a V8 `ArrayBuffer`.

## V8 Sandbox Limitation

**CEF 149 ships with the V8 sandbox compiled in** (not toggleable at runtime).
When the sandbox is active, `CefV8Value::CreateArrayBuffer` (external backing
store) **always returns nullptr** — V8 cannot reference memory outside its
sandboxed region.

**Workaround:** use `CreateArrayBufferWithCopy`, which copies the shared memory
data INTO V8's managed memory. This is one `memcpy` per frame — **~20 µs for
64 KB**, negligible vs. the 16.7 ms frame budget.

The `--disable-v8-sandbox` flag is kept in `flags.rs` for future CEF versions
that might make the sandbox toggleable at runtime.

## Architecture

```text
Browser process (native):
  worker thread ──ring buffer──> game loop ──> push_frame_data(frame, &[u8])
                                                   │
                                         SharedProcessMessageBuilder
                                         (shared memory, zero-serialize)
                                                   │
                                              send_process_message
                                                   ▼
Renderer process (V8):
  on_process_message_received
    │
    ├─ region.memory() → raw ptr (mapped in renderer)
    ├─ v8_value_create_array_buffer_with_copy(ptr, len)  ← one memcpy
    └─ window.__afterglow_frame_data = ArrayBuffer
                                                   │
  JS: __afterglow_on_frame_data() callback → write_buffer / render
```

## Public API

### `send_shared_buffer(frame: &Frame, byte_size: usize)`

Send a persistent shared memory ring buffer to the renderer (once, at
startup). Both sides can read/write this memory for bidirectional control data.
The renderer exposes it as `window.__afterglow_buffer`.

Called from `LifeSpanHandler::on_after_created` (the runtime does this
automatically with an 8 MiB buffer).

### `push_frame_data(frame: &Frame, data: &[u8])`

Push per-frame binary data to the renderer via a new shared memory message.
The renderer copies it into a V8 `ArrayBuffer` and exposes it as
`window.__afterglow_frame_data`, then calls `window.__afterglow_on_frame_data()`
if the callback exists.

Called from the game loop thread each frame.

### `MAIN_BROWSER: Mutex<Option<Browser>>`

Global handle to the main browser, set in `on_after_created`. Other threads
(game loop, push thread) use `browser.main_frame()` to get the frame for
`push_frame_data`.

### `AppBuilder::on_ready(f: impl Fn() + Send + Sync + 'static)`

Callback fired once after the browser is created (on the UI thread). Use this
to spawn game-loop / push threads. **Spawning threads before CEF init
(`execute_process`) crashes the GPU process** — always use `on_ready`.

## Performance (measured)

| Payload | Latency | Throughput |
|---------|---------|------------|
| 64 B | 88 µs | 0.7 MB/s |
| 4 KB | 7.5 µs | 523 MB/s |
| 64 KB | 20 µs | 3.1 GB/s |
| 256 KB | 60 µs | 4.2 GB/s |
| 1 MB | 194 µs | 5.2 GB/s |

Small-payload latency (~80-90 µs for 64-256 B) is dominated by the fixed
overhead of `shared_process_message_builder_create` + `build` + `send_process_message`.
For 60 FPS with 64 KB physics data, 20 µs per push is negligible.

## JS Interface

```js
// Persistent ring buffer (sent once at startup):
window.__afterglow_buffer  // ArrayBuffer, 8 MiB
// Layout: [capacity:u32][write_idx:u32][read_idx:u32][data...]

// Per-frame data (updated each frame):
window.__afterglow_frame_data  // ArrayBuffer, variable size

// Callback (called when new frame data arrives):
window.__afterglow_on_frame_data = function() {
  const data = new Uint8Array(window.__afterglow_frame_data);
  // ... use data for device.queue.write_buffer, etc.
};
```

## Message Names

| Name | Direction | Purpose |
|------|-----------|---------|
| `afterglow_shm_setup` | browser → renderer | Initial persistent ring buffer (once) |
| `afterglow_shm_frame` | browser → renderer | Per-frame physics data (each frame) |
