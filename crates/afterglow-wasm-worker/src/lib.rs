//! WASM Web Worker entry for the Physics RPC demo.
//!
//! Compiled with `--target wasm32-unknown-unknown` + `wasm-bindgen --target web`.
//! The generated JS is loaded in a Web Worker; `onmessage` calls `serve` and
//! posts the response back.

use afterglow_rpc_demo::{PhysicsServer, PhysicsWorker};
use wasm_bindgen::prelude::*;

use std::cell::RefCell;

thread_local! {
    static WORKER: RefCell<PhysicsWorker> = RefCell::new(PhysicsWorker);
}

/// Called by the Web Worker's `onmessage`: takes framed request bytes,
/// runs `wasm_serve` (which decodes the frame + calls the generated `serve`
/// dispatch), returns the response bytes.
#[wasm_bindgen]
pub fn serve(msg: &[u8]) -> Vec<u8> {
    WORKER.with_borrow_mut(|w| {
        afterglow_rpc_demo::wasm_serve(w, msg)
            .unwrap_or_else(|e| format!("error: {e}").into_bytes())
    })
}
