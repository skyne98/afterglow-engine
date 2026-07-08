//! # afterglow-engine-core
//!
//! The engine's shared logic, compiled two ways from one codebase:
//! - **native** (`cfg(not(target_arch = "wasm32"))`): linked into the CEF host
//!   (`afterglow-cef`) and called directly from Rust. Runs full-speed on the
//!   CPU/native threads.
//! - **WASM** (`cfg(target_arch = "wasm32")`): compiled with `wasm-bindgen` to
//!   a `cdylib`, loaded in a Web Worker on the web build. The same `simulate`
//!   logic runs off the JS main thread.
//!
//! This is the conditional-compilation seam: the host (CEF or the browser's
//! worker) is different, the engine core is identical.

use serde_json::Value;

/// One simulation step. Pure function: takes serialized state + input, returns
/// the next serialized state. Same code path on native and WASM.
pub fn simulate(state: &Value, input: &Value, dt_ms: f64) -> Value {
    // Placeholder core: advance a counter + echo input. Real systems (physics,
    // netcode, ECS) plug in here and are shared across both targets.
    let tick = state["tick"].as_f64().unwrap_or(0.0) + dt_ms;
    serde_json::json!({ "tick": tick, "echo": input })
}

// --- native entry: called by the CEF host ---------------------------------
#[cfg(not(target_arch = "wasm32"))]
pub mod native {
    use super::*;
    /// Called from the CEF host's game loop. Returns JSON to push to the page.
    pub fn step(state_json: &str, input_json: &str, dt_ms: f64) -> String {
        let state: Value = serde_json::from_str(state_json).unwrap_or(Value::Null);
        let input: Value = serde_json::from_str(input_json).unwrap_or(Value::Null);
        simulate(&state, &input, dt_ms).to_string()
    }
}

// --- WASM entry: exposed to the web worker via wasm-bindgen ----------------
#[cfg(target_arch = "wasm32")]
pub mod wasm {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn step(state_json: &str, input_json: &str, dt_ms: f64) -> String {
        let state: Value = serde_json::from_str(state_json).unwrap_or(Value::Null);
        let input: Value = serde_json::from_str(input_json).unwrap_or(Value::Null);
        simulate(&state, &input, dt_ms).to_string()
    }
}
