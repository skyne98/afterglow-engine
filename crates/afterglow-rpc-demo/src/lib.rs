//! Demo worker interface: define the RPC once in Rust; the `#[rpc]` macro
//! generates the server trait, the Rust client, the dispatch, and the schema.
//! The build system turns the schema into a TypeScript client.

use afterglow_rpc_macros::rpc;

/// A physics worker. Methods are called by the main thread (or another worker)
/// as if the worker were local — `PhysicsClient::new(transport).step(...)`.
#[rpc]
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
        // Worker -> main game context push (native: over the engineered event
        // channel; no web/JS messages).
        #[cfg(not(target_arch = "wasm32"))]
        afterglow_rpc::native::push_event(format!("stepped:{}", state.len()).into_bytes());
        state
    }
    fn apply_force(&mut self, body_id: u32, fx: f32, fy: f32, fz: f32) -> bool {
        // accept nonzero forces on odd ids (just to have logic)
        body_id % 2 == 1 && (fx + fy + fz).abs() > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use afterglow_rpc::{Loopback, Transport};
    use std::cell::RefCell;

    #[test]
    fn client_calls_server_over_loopback() {
        let server = RefCell::new(PhysicsWorker);
        let transport = Loopback(|svc: &str, method: u32, args: &[u8]| {
            assert_eq!(svc, "Physics");
            // generated dispatch fn
            serve(&mut *server.borrow_mut(), method, args)
        });
        let client = PhysicsClient::new(transport);

        // main -> worker: apply_force
        let accepted = client.apply_force(3, 0.0, 9.8, 0.0).unwrap();
        assert!(accepted);

        // main -> worker: step
        let next = client.step(vec![0.0, 1.0, 2.0], 0.5).unwrap();
        assert_eq!(next, vec![0.5, 1.5, 2.5]);
    }

    #[test]
    fn native_worker_thread_over_channels() {
        // Spawn the worker as a native thread; it talks to the main game
        // context over engineered mpsc channels (no web/JS messages).
        let (client, events) = spawn_worker(PhysicsWorker);

        // main -> worker (RPC over the request/response channel)
        let next = client.step(vec![0.0, 1.0, 2.0], 0.5).unwrap();
        assert_eq!(next, vec![0.5, 1.5, 2.5]);
        assert!(client.apply_force(3, 0.0, 9.8, 0.0).unwrap());

        // worker -> main (event push, drained each frame)
        let mut evs = Vec::new();
        events.drain_into(&mut evs);
        assert!(evs.iter().any(|e| e == b"stepped:3"));
    }
}
