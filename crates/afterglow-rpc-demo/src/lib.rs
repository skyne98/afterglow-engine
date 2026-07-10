//! Demo worker interface: define the RPC once in Rust; the `#[rpc]` macro
//! generates the server trait (with a provided `serve` dispatch), the Rust
//! client, the `PHYSICS_SCHEMA` static, and (because `worker = PhysicsWorker`
//! is given) the native `spawn_worker` + web wasm exports.
//!
//! `#[rpc(worker = PhysicsWorker)]` tells the macro the concrete impl type.
//! Methods are written without a receiver; the macro injects `&mut self` into
//! the generated `PhysicsServer` trait. `PhysicsWorker: Default` is required
//! for wasm construction.

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
#[derive(Default)]
pub struct PhysicsWorker;

impl PhysicsServer for PhysicsWorker {
    fn step(&mut self, mut state: Vec<f32>, dt: f32) -> Vec<f32> {
        for v in state.iter_mut() {
            *v += dt;
        }
        // Surface event-push failure rather than silently dropping it.
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(e) =
            afterglow_rpc::native::push_event(format!("stepped:{}", state.len()).as_bytes())
        {
            eprintln!("[physics] event push failed: {e}");
        }
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
        let (client, events) = PhysicsClient::spawn_worker(PhysicsWorker).unwrap();

        let next = client.step(vec![0.0, 1.0, 2.0], 0.5).unwrap();
        assert_eq!(next, vec![0.5, 1.5, 2.5]);

        assert!(client.apply_force(3, 0.0, 9.8, 0.0).unwrap());

        let mut evs = Vec::new();
        events.drain_into(&mut evs);
        assert!(evs.iter().any(|e| e == b"stepped:3"));
    }

    #[test]
    fn unknown_method_returns_server_error() {
        let (client, _events) = PhysicsClient::spawn_worker(PhysicsWorker).unwrap();
        // Call an unknown method id (99) via the transport directly.
        let err = client.transport().call("Physics", 99, &[]).unwrap_err();
        match err {
            afterglow_rpc::RpcError::Server(m) => assert_eq!(m, "unknown method"),
            other => panic!("expected Server(unknown method), got {other:?}"),
        }
    }

    #[test]
    fn schema_describes_methods() {
        let s = PHYSICS_SCHEMA;
        assert_eq!(s.name, "Physics");
        assert_eq!(s.methods.len(), 2);
        assert_eq!(s.methods[0].id, 0);
        assert_eq!(s.methods[0].name, "step");
        assert_eq!(s.methods[1].id, 1);
        assert_eq!(s.methods[1].name, "apply_force");
    }
}

/// Two non-worker `#[rpc]` traits in one module: proves the generated
/// `<Name>Server`/`<Name>Client`/`<NAME>_SCHEMA` names coexist without
/// collision, and that single-param (1-tuple) + unit/zero-byte results
/// round-trip through the real native worker loop. `#[cfg(test)]` so the
/// wasm build (one worker service per cdylib) is unaffected.
#[cfg(test)]
mod multi_trait {
    use super::*;
    use afterglow_rpc::native::spawn_worker_loop;

    #[rpc]
    pub trait Foo {
        fn echo(s: String) -> String;
    }
    #[rpc]
    pub trait Bar {
        fn log(msg: String);
    }

    pub struct FooImpl;
    impl FooServer for FooImpl {
        fn echo(&mut self, s: String) -> String {
            s + "!"
        }
    }
    pub struct BarImpl;
    impl BarServer for BarImpl {
        fn log(&mut self, _msg: String) {}
    }

    #[test]
    fn two_non_worker_traits_dispatch_and_schema() {
        let (t, _) = spawn_worker_loop(FooImpl, 1 << 16, |s, m, a| s.serve(m, a)).unwrap();
        let foo = FooClient::new(t);
        assert_eq!(foo.echo("hi".into()).unwrap(), "hi!");
        let (t, _) = spawn_worker_loop(BarImpl, 1 << 16, |s, m, a| s.serve(m, a)).unwrap();
        let bar = BarClient::new(t);
        bar.log("x".into()).unwrap(); // unit/zero-byte result via the real envelope
        assert_eq!(FOO_SCHEMA.name, "Foo");
        assert_eq!(BAR_SCHEMA.name, "Bar");
        assert_ne!(FOO_SCHEMA as *const _, BAR_SCHEMA as *const _);
    }
}
