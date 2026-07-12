//! Demo worker interface: define the RPC once in Rust; the `#[rpc]` macro
//! generates the server trait (with a provided `serve` dispatch), the Rust
//! client, and (because `worker = PhysicsWorker` is given) the native
//! `spawn_worker` + web wasm exports.
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
        let err = client.transport().call(99, &[]).unwrap_err();
        match err {
            afterglow_rpc::RpcError::Server(m) => assert_eq!(m, "unknown method"),
            other => panic!("expected Server(unknown method), got {other:?}"),
        }
    }
}

/// Two non-worker `#[rpc]` traits in one module: proves the generated
/// `<Name>Server`/`<Name>Client` names coexist without collision, and that
/// single-param (1-tuple) + unit/zero-byte results
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
    fn two_non_worker_traits_dispatch() {
        let (t, _) = spawn_worker_loop(FooImpl, 1 << 16, |s, m, a| s.serve(m, a)).unwrap();
        let foo = FooClient::new(t);
        assert_eq!(foo.echo("hi".into()).unwrap(), "hi!");
        let (t, _) = spawn_worker_loop(BarImpl, 1 << 16, |s, m, a| s.serve(m, a)).unwrap();
        let bar = BarClient::new(t);
        bar.log("x".into()).unwrap(); // unit/zero-byte result via the real envelope
    }
}

/// Sync worker type matrix: every supported type round-trips through the real
/// native worker loop. Exercises the generated `serve` dispatch + `Client`
/// methods + postcard encode/decode for every type.
#[cfg(test)]
mod type_matrix {
    use super::*;
    use afterglow_rpc::native::spawn_worker_loop;

    #[rpc]
    pub trait TypeMatrix {
        fn echo_f32(x: f32) -> f32;
        fn echo_f64(x: f64) -> f64;
        fn echo_u8(x: u8) -> u8;
        fn echo_u16(x: u16) -> u16;
        fn echo_u32(x: u32) -> u32;
        fn echo_u64(x: u64) -> u64;
        fn echo_usize(x: usize) -> usize;
        fn echo_i8(x: i8) -> i8;
        fn echo_i16(x: i16) -> i16;
        fn echo_i32(x: i32) -> i32;
        fn echo_i64(x: i64) -> i64;
        fn echo_isize(x: isize) -> isize;
        fn echo_bool(x: bool) -> bool;
        fn echo_string(s: String) -> String;
        fn echo_vec_u8(v: Vec<u8>) -> Vec<u8>;
        fn echo_vec_f32(v: Vec<f32>) -> Vec<f32>;
        fn echo_vec_f64(v: Vec<f64>) -> Vec<f64>;
        fn multi(a: u32, b: f32, c: String, d: bool) -> u64;
        fn no_args() -> u32;
        fn void(x: u32);
    }

    pub struct TypeMatrixImpl;

    impl TypeMatrixServer for TypeMatrixImpl {
        fn echo_f32(&mut self, x: f32) -> f32 {
            x
        }
        fn echo_f64(&mut self, x: f64) -> f64 {
            x
        }
        fn echo_u8(&mut self, x: u8) -> u8 {
            x
        }
        fn echo_u16(&mut self, x: u16) -> u16 {
            x
        }
        fn echo_u32(&mut self, x: u32) -> u32 {
            x
        }
        fn echo_u64(&mut self, x: u64) -> u64 {
            x
        }
        fn echo_usize(&mut self, x: usize) -> usize {
            x
        }
        fn echo_i8(&mut self, x: i8) -> i8 {
            x
        }
        fn echo_i16(&mut self, x: i16) -> i16 {
            x
        }
        fn echo_i32(&mut self, x: i32) -> i32 {
            x
        }
        fn echo_i64(&mut self, x: i64) -> i64 {
            x
        }
        fn echo_isize(&mut self, x: isize) -> isize {
            x
        }
        fn echo_bool(&mut self, x: bool) -> bool {
            x
        }
        fn echo_string(&mut self, s: String) -> String {
            s
        }
        fn echo_vec_u8(&mut self, v: Vec<u8>) -> Vec<u8> {
            v
        }
        fn echo_vec_f32(&mut self, v: Vec<f32>) -> Vec<f32> {
            v
        }
        fn echo_vec_f64(&mut self, v: Vec<f64>) -> Vec<f64> {
            v
        }
        fn multi(&mut self, a: u32, b: f32, c: String, d: bool) -> u64 {
            (a as u64)
                .wrapping_add((b as u64).wrapping_mul(c.len() as u64))
                .wrapping_add(if d { 1000 } else { 0 })
        }
        fn no_args(&mut self) -> u32 {
            42
        }
        fn void(&mut self, _x: u32) {}
    }

    #[test]
    fn all_types_round_trip() {
        let (t, _) = spawn_worker_loop(TypeMatrixImpl, 1 << 20, |s, m, a| s.serve(m, a)).unwrap();
        let c = TypeMatrixClient::new(t);

        // Primitives.
        assert_eq!(c.echo_f32(3.14).unwrap(), 3.14);
        assert_eq!(c.echo_f64(2.718281828459045).unwrap(), 2.718281828459045);
        assert_eq!(c.echo_u8(255).unwrap(), 255);
        assert_eq!(c.echo_u16(65535).unwrap(), 65535);
        assert_eq!(c.echo_u32(4294967295).unwrap(), 4294967295);
        assert_eq!(c.echo_u64(u64::MAX).unwrap(), u64::MAX);
        assert_eq!(c.echo_usize(usize::MAX / 2).unwrap(), usize::MAX / 2);
        assert_eq!(c.echo_i8(-128).unwrap(), -128);
        assert_eq!(c.echo_i16(-32768).unwrap(), -32768);
        assert_eq!(c.echo_i32(-2147483648).unwrap(), -2147483648);
        assert_eq!(c.echo_i64(i64::MIN).unwrap(), i64::MIN);
        assert_eq!(c.echo_isize(-42).unwrap(), -42);
        assert_eq!(c.echo_bool(true).unwrap(), true);
        assert_eq!(c.echo_bool(false).unwrap(), false);

        // String.
        assert_eq!(c.echo_string("héllo 世界".into()).unwrap(), "héllo 世界");

        // Vectors.
        assert_eq!(
            c.echo_vec_u8(vec![1, 2, 3, 250]).unwrap(),
            vec![1, 2, 3, 250]
        );
        assert_eq!(
            c.echo_vec_f32(vec![1.5, -2.5, 3.0]).unwrap(),
            vec![1.5, -2.5, 3.0]
        );
        assert_eq!(
            c.echo_vec_f64(vec![1e100, -1e100]).unwrap(),
            vec![1e100, -1e100]
        );

        // Multi-param mixed types.
        assert_eq!(
            c.multi(10, 2.0, "hello".into(), true).unwrap(),
            10 + 10 + 1000
        );

        // No args + return.
        assert_eq!(c.no_args().unwrap(), 42);

        // Void return (unit result).
        assert!(c.void(99).is_ok());
    }

    #[test]
    fn empty_vectors_round_trip() {
        let (t, _) = spawn_worker_loop(TypeMatrixImpl, 1 << 20, |s, m, a| s.serve(m, a)).unwrap();
        let c = TypeMatrixClient::new(t);
        assert!(c.echo_vec_u8(vec![]).unwrap().is_empty());
        assert!(c.echo_vec_f32(vec![]).unwrap().is_empty());
        assert_eq!(c.echo_string("".into()).unwrap(), "");
    }

    #[test]
    fn large_vector_round_trip() {
        let (t, _) = spawn_worker_loop(TypeMatrixImpl, 1 << 20, |s, m, a| s.serve(m, a)).unwrap();
        let c = TypeMatrixClient::new(t);
        // 100K f32s = 400KB payload + framing. Exercises ring wraparound.
        let big: Vec<f32> = (0..100_000).map(|i| i as f32 * 1.5).collect();
        let result = c.echo_vec_f32(big.clone()).unwrap();
        assert_eq!(result.len(), 100_000);
        assert_eq!(result, big);
    }
}
