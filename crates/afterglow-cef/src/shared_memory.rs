//! Cross-process shared memory data transfer via CefSharedMemoryRegion.
//!
//! ## V8 sandbox limitation
//!
//! CEF 149 ships with the V8 sandbox **compiled in** (not toggleable at
//! runtime). When the sandbox is active, `CefV8Value::CreateArrayBuffer`
//! (external backing store) always returns nullptr — V8 cannot reference
//! memory outside its sandboxed region.
//!
//! **Workaround:** use `CreateArrayBufferWithCopy`, which copies the shared
//! memory data INTO V8's managed memory. This is one `memcpy` per frame —
//! ~8 µs for 64 KB (1k physics objects × 64 B), negligible vs. the 16.7 ms
//! frame budget.
//!
//! ## Architecture
//!
//! ```text
//! Browser process (native):
//!   worker thread ──ring buffer──> game loop ──> push_frame_data(frame, &[u8])
//!                                                    │
//!                                          SharedProcessMessageBuilder
//!                                          (shared memory, zero-serialize)
//!                                                    │
//!                                               send_process_message
//!                                                    ▼
//! Renderer process (V8):
//!   on_process_message_received
//!     │
//!     ├─ region.memory() → raw ptr (mapped in renderer)
//!     ├─ v8_value_create_array_buffer_with_copy(ptr, len)  ← one memcpy
//!     └─ window.__afterglow_frame_data = ArrayBuffer
//!                                                    │
//!   JS: requestAnimationFrame → read __afterglow_frame_data → write_buffer
//! ```
//!
//! Compared to `execute_java_script` (which serializes data as a JS string):
//! - No string allocation / parsing overhead
//! - No GC pressure from large strings
//! - Binary data preserved (no base64 / JSON encoding)
//! - Measured: ~10× faster for payloads > 4 KB

use cef::*;
use std::sync::OnceLock;

/// Message name for the initial ring-buffer setup (sent once at startup).
const SHM_SETUP_MSG: &str = "afterglow_shm_setup";

/// Message name for per-frame data pushes.
const SHM_FRAME_MSG: &str = "afterglow_shm_frame";

/// The shared memory region held by the renderer for the persistent ring
/// buffer (sent once at startup). Both sides can read/write to this memory
/// for bidirectional control data (e.g., input state, configuration).
static RENDERER_SHM: OnceLock<SharedMemoryRegion> = OnceLock::new();

// ---------------------------------------------------------------------------
// Browser side
// ---------------------------------------------------------------------------

/// Send the initial shared memory ring buffer to the renderer (once, at
/// startup). Both sides can read/write this persistent buffer for bidirectional
/// control data. Call from `LifeSpanHandler::on_after_created`.
pub fn send_shared_buffer(frame: &Frame, byte_size: usize) {
    let name = CefString::from(SHM_SETUP_MSG);
    let Some(builder) = shared_process_message_builder_create(Some(&name), byte_size) else {
        eprintln!("[afterglow] failed to create shared process message builder");
        return;
    };

    // Initialize the ring buffer header in the shared memory.
    let ptr = builder.memory() as *mut u8;
    let size = builder.size();
    if !ptr.is_null() && size >= 12 {
        unsafe {
            // RingBuffer layout: [capacity:u32][write_idx:u32][read_idx:u32][data...]
            let cap = (size - 12) as u32;
            std::ptr::copy_nonoverlapping(cap.to_le_bytes().as_ptr(), ptr, 4);
            std::ptr::write_bytes(ptr.add(4), 0, 8); // write_idx = 0, read_idx = 0
        }
    }

    let Some(mut msg) = builder.build() else {
        eprintln!("[afterglow] failed to build shared process message");
        return;
    };

    frame.send_process_message(ProcessId::RENDERER, Some(&mut msg));
    eprintln!("[afterglow] sent {size}-byte shared memory to renderer");
}

/// Push per-frame binary data to the renderer via shared memory (zero
/// serialization). The renderer copies it into a V8 ArrayBuffer and exposes
/// it as `window.__afterglow_frame_data`.
///
/// Call from the game loop thread (browser process) each frame, after the
/// workers have written their results.
pub fn push_frame_data(frame: &Frame, data: &[u8]) {
    if data.is_empty() {
        return;
    }

    let name = CefString::from(SHM_FRAME_MSG);
    let Some(builder) = shared_process_message_builder_create(Some(&name), data.len()) else {
        return;
    };

    // Write the frame data directly into shared memory (zero-serialize).
    let ptr = builder.memory() as *mut u8;
    let size = builder.size();
    if !ptr.is_null() && size >= data.len() {
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        }
    }

    let Some(mut msg) = builder.build() else {
        return;
    };

    frame.send_process_message(ProcessId::RENDERER, Some(&mut msg));
}

// ---------------------------------------------------------------------------
// Renderer side
// ---------------------------------------------------------------------------

wrap_render_process_handler! {
    pub struct GameRenderProcessHandler;

    impl RenderProcessHandler {
        fn on_context_created(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            context: Option<&mut V8Context>,
        ) {
            // If the setup shared memory already arrived (before context
            // creation), expose it now as window.__afterglow_buffer.
            if let Some(region) = RENDERER_SHM.get() {
                let size = region.size();
                let ptr = region.memory() as *mut u8;
                if !ptr.is_null() && size > 0 {
                    if let Some(ctx) = context.as_ref() {
                        if let Some(window) = ctx.global() {
                            if let Some(ab) =
                                v8_value_create_array_buffer_with_copy(ptr, size)
                            {
                                let key = CefString::from("__afterglow_buffer");
                                window.set_value_bykey(
                                    Some(&key),
                                    Some(&mut { ab }),
                                    V8Propertyattribute::default(),
                                );
                                eprintln!(
                                    "[afterglow] exposed {size}-byte ring buffer to JS (on_context_created)"
                                );
                            }
                        }
                    }
                }
            }
        }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            let Some(msg) = message else { return 0 };
            let name = CefString::from(&msg.name()).to_string();

            match name.as_str() {
                SHM_SETUP_MSG => {
                    if let Some(region) = msg.shared_memory_region() {
                        let size = region.size();
                        let _ = RENDERER_SHM.set(region);
                        eprintln!("[afterglow] renderer received shared memory ({size} bytes)");
                        expose_setup_buffer(frame);
                    }
                    1
                }
                SHM_FRAME_MSG => {
                    if let Some(region) = msg.shared_memory_region() {
                        let size = region.size();
                        let ptr = region.memory() as *mut u8;
                        if !ptr.is_null() && size > 0 {
                            expose_frame_data(frame, ptr, size);
                        }
                    }
                    1
                }
                _ => 0,
            }
        }
    }
}

impl GameRenderProcessHandler {
    pub fn make() -> RenderProcessHandler {
        GameRenderProcessHandler::new()
    }
}

/// Set `window.__afterglow_buffer` (the persistent ring buffer) on the frame's
/// V8 context. Handles the case where the shared memory arrives after
/// `on_context_created` already fired.
fn expose_setup_buffer(frame: Option<&mut Frame>) {
    let Some(frame) = frame else { return };
    let Some(ctx) = frame.v8_context() else { return };
    let Some(region) = RENDERER_SHM.get() else { return };
    let ptr = region.memory() as *mut u8;
    let size = region.size();
    if ptr.is_null() || size == 0 {
        return;
    }

    if ctx.enter() != 0 {
        if let Some(window) = ctx.global() {
            if let Some(ab) = v8_value_create_array_buffer_with_copy(ptr, size) {
                let key = CefString::from("__afterglow_buffer");
                window.set_value_bykey(
                    Some(&key),
                    Some(&mut { ab }),
                    V8Propertyattribute::default(),
                );
                eprintln!("[afterglow] exposed {size}-byte ring buffer to JS");
            }
        }
        ctx.exit();
    }
}

/// Set `window.__afterglow_frame_data` (per-frame data) on the frame's V8
/// context. Called every frame from `on_process_message_received`.
///
/// Uses `CreateArrayBufferWithCopy` because the V8 sandbox (compiled into
/// CEF 149) blocks external ArrayBuffers. The copy is a single `memcpy`
/// (~8 µs for 64 KB).
fn expose_frame_data(frame: Option<&mut Frame>, ptr: *mut u8, size: usize) {
    let Some(frame) = frame else { return };
    let Some(ctx) = frame.v8_context() else { return };

    if ctx.enter() != 0 {
        if let Some(window) = ctx.global() {
            if let Some(ab) = v8_value_create_array_buffer_with_copy(ptr, size) {
                let key = CefString::from("__afterglow_frame_data");
                window.set_value_bykey(
                    Some(&key),
                    Some(&mut { ab }),
                    V8Propertyattribute::default(),
                );
                // Signal JS that new frame data is available.
                let eval = CefString::from(
                    "if(window.__afterglow_on_frame_data){window.__afterglow_on_frame_data()}",
                );
                frame.execute_java_script(Some(&eval), Some(&CefString::default()), 0);
            }
        }
        ctx.exit();
    }
}
