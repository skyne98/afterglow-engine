//! Demo worker interface: define the RPC once in Rust; the `#[rpc]` macro
//! generates the server trait, the Rust client, the dispatch, and the schema.
//!
//! `#[rpc(worker = PhysicsWorker)]` tells the macro the concrete impl type,
//! so it generates `spawn_worker(impl)` (native), `wasm_init()` (web worker
//! init), `get_client()` (web main thread), and `wasm_serve_frame()` (web
//! worker dispatch). Zero boilerplate.

use afterglow_rpc_macros::rpc;

/// A physics worker. Methods are called by the main thread (or another worker)
/// as if the worker were local — `client.step(...)`.
#[rpc(worker = PhysicsWorker)]
pub trait Physics {
    /// Advance a body's state by dt; returns the new state.
    fn step(state: Vec<f32>, dt: f32) -> Vec<f32>;
    /// Apply a force to a body; returns whether it was accepted.
    fn apply_force(body_id: u32, fx: f32, fy: f32, fz: f32) -> bool;
}

/// A concrete server (the worker's actual implementation).
pub struct PhysicsWorker;
impl PhysicsServer for PhysicsWorker {
    fn step(&mut self, mut state: Vec<f32>, dt: f32) -> Vec<f32> {
        for v in state.iter_mut() {
            *v += dt;
        }
        #[cfg(not(target_arch = "wasm32"))]
        afterglow_rpc::native::push_event(format!("stepped:{}", state.len()).into_bytes());
        state
    }
    fn apply_force(&mut self, body_id: u32, fx: f32, fy: f32, fz: f32) -> bool {
        body_id % 2 == 1 && (fx + fy + fz).abs() > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use afterglow_rpc::Transport;

    #[test]
    fn ring_buffer_worker_round_trip() {
        let (client, events) = spawn_worker(PhysicsWorker);

        let next = client.step(vec![0.0, 1.0, 2.0], 0.5).unwrap();
        assert_eq!(next, vec![0.5, 1.5, 2.5]);

        assert!(client.apply_force(3, 0.0, 9.8, 0.0).unwrap());

        let mut evs = Vec::new();
        events.drain_into(&mut evs);
        assert!(evs.iter().any(|e| e == b"stepped:3"));
    }
}
