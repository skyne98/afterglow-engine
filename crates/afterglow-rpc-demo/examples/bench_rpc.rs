//! Ring buffer stress test: both directions, multiple payloads, latency + bandwidth.
//!
//! Native: native threads + heap-backed ring buffers (`RingStorage`).
//!
//! 1. Main→Worker (write): main writes, worker drains — measures write side.
//! 2. Worker→Main (read): worker fills, main reads — measures read side.
//! 3. Round-trip: main writes a request, worker echoes it as a response,
//!    main reads the response — a full SPSC round trip over two shared ring
//!    buffers (same shape as a real RPC call, minus the serve step).
//! 4. Service RPC: pre-encoded `Physics::step` request through the generated
//!    native worker transport, including dispatch and caller result decode.

use afterglow_rpc::native::RingStorage;
use afterglow_rpc::{RpcError, Transport, decode, encode};
use afterglow_rpc_demo::{PhysicsClient, PhysicsWorker};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const CAPACITY: usize = 8 * 1024 * 1024;
const SIZES: &[usize] = &[64, 256, 1024, 4096, 16384, 65536, 262144, 1048576];
const N: usize = 10_000;
const RPC_COUNTS: &[usize] = &[1, 4, 16, 64, 256, 1024, 4096, 16384];
const RPC_N: usize = 1_000;

fn main() {
    println!("NATIVE RING BUFFER STRESS TEST (native threads + heap memory)");
    println!("capacity: {} MiB", CAPACITY / 1024 / 1024);
    println!("iterations: {N}");
    println!();
    bench_write();
    bench_read();
    bench_roundtrip();
    bench_service_rpc();
}

/// Direction 1: Main → Worker (write latency).
fn bench_write() {
    println!("\nDirection 1: Main → Worker (write)");
    println!("  size  lat_us   MiB/s  writes/s");
    for &size in SIZES {
        let payload = vec![0xAAu8; size];
        let storage = RingStorage::new(CAPACITY).unwrap();
        let (prod, cons) = storage.split();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();

        let handle = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                let _ = cons.read();
            }
        });

        let t0 = Instant::now();
        let mut ok = 0u64;
        for _ in 0..N {
            loop {
                match prod.write(&payload) {
                    Ok(()) => {
                        ok += 1;
                        break;
                    }
                    Err(RpcError::BufferFull) => std::hint::spin_loop(),
                    Err(e) => {
                        eprintln!("producer err: {e}");
                        break;
                    }
                }
            }
        }
        let dt = t0.elapsed();
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();

        print_row(size, ok, dt, 1.0);
    }
}

/// Direction 2: Worker → Main (read latency).
fn bench_read() {
    println!("\nDirection 2: Worker → Main (read)");
    println!("  size  lat_us   MiB/s   reads/s");
    for &size in SIZES {
        let payload = vec![0xBBu8; size];
        let storage = RingStorage::new(CAPACITY).unwrap();
        let (prod, cons) = storage.split();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let payload2 = payload.clone();

        let handle = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                let _ = prod.write(&payload2);
            }
        });

        std::thread::sleep(std::time::Duration::from_millis(10));
        let t0 = Instant::now();
        let mut ok = 0u64;
        for _ in 0..N {
            loop {
                match cons.read() {
                    Ok(_) => {
                        ok += 1;
                        break;
                    }
                    Err(RpcError::BufferEmpty) => std::hint::spin_loop(),
                    Err(e) => {
                        eprintln!("consumer err: {e}");
                        break;
                    }
                }
            }
        }
        let dt = t0.elapsed();
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();

        print_row(size, ok, dt, 1.0);
    }
}

/// Round-trip: Main → Worker → Main over two ring buffers.
fn bench_roundtrip() {
    println!("\nRound-trip: Main → Worker → Main");
    println!("  size  lat_us agg_MiB/s calls/s");
    for &size in SIZES {
        let payload = vec![0xCCu8; size];
        let req = RingStorage::new(CAPACITY).unwrap();
        let resp = RingStorage::new(CAPACITY).unwrap();
        let (req_prod, req_cons) = req.split();
        let (resp_prod, resp_cons) = resp.split();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();

        let handle = std::thread::spawn(move || {
            // Worker: read request, echo it as a response.
            while !stop2.load(Ordering::Relaxed) {
                match req_cons.read() {
                    Ok(data) => {
                        while resp_prod.write(&data).is_err() {
                            std::hint::spin_loop();
                        }
                    }
                    Err(RpcError::BufferEmpty) => std::hint::spin_loop(),
                    Err(e) => {
                        eprintln!("worker err: {e}");
                        break;
                    }
                }
            }
        });

        let t0 = Instant::now();
        let mut ok = 0u64;
        for _ in 0..N {
            loop {
                match req_prod.write(&payload) {
                    Ok(()) => break,
                    Err(RpcError::BufferFull) => std::hint::spin_loop(),
                    Err(e) => {
                        eprintln!("producer err: {e}");
                        break;
                    }
                }
            }
            loop {
                match resp_cons.read() {
                    Ok(_) => {
                        ok += 1;
                        break;
                    }
                    Err(RpcError::BufferEmpty) => std::hint::spin_loop(),
                    Err(e) => {
                        eprintln!("consumer err: {e}");
                        break;
                    }
                }
            }
        }
        let dt = t0.elapsed();
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();

        print_row(size, ok, dt, 2.0);
    }
}

/// End-to-end service transport: pre-encoded request → request ring → worker
/// dispatch → response ring → postcard result decode. This is directly
/// comparable to the web worker benchmark. Validation runs outside the timer.
fn bench_service_rpc() {
    println!("\nService RPC: Physics::step (dispatch + result decode)");
    println!("  f32 count  payload  lat_us   MiB/s  valid/total");

    let (client, _events) = PhysicsClient::spawn_worker(PhysicsWorker).unwrap();
    let dt = 0.016_f32;
    // Match the web benchmark's warm-up so neither path measures one-time
    // worker startup, allocation, or code-cache effects.
    let warmup_args = encode(&(vec![0.0_f32; 64], dt)).unwrap();
    for _ in 0..100 {
        let bytes = client.transport().call("Physics", 0, &warmup_args).unwrap();
        let _: Vec<f32> = decode(&bytes).unwrap();
    }
    for &count in RPC_COUNTS {
        let input: Vec<f32> = (0..count).map(|i| i as f32).collect();
        // Match web: encode the immutable request once, then benchmark transport,
        // worker-side argument decode/service/result encode, and caller decode.
        let args = encode(&(input.clone(), dt)).unwrap();
        let mut elapsed = std::time::Duration::ZERO;
        let mut ok = 0u64;
        for _ in 0..RPC_N {
            let started = Instant::now();
            let bytes = client.transport().call("Physics", 0, &args).unwrap();
            let output: Vec<f32> = decode(&bytes).unwrap();
            elapsed += started.elapsed();
            if output.len() == count
                && output
                    .iter()
                    .enumerate()
                    .all(|(i, &v)| (v - (input[i] + dt)).abs() < 1e-6)
            {
                ok += 1;
            }
        }
        let latency_us = elapsed.as_secs_f64() * 1e6 / RPC_N as f64;
        let bandwidth = count as f64 * size_of::<f32>() as f64 * RPC_N as f64 * 2.0
            / elapsed.as_secs_f64()
            / 1024.0
            / 1024.0;
        println!(
            "{:>11} {:>7} B {:>7.1} {:>7.1} {ok}/{RPC_N}",
            count,
            count * size_of::<f32>(),
            latency_us,
            bandwidth,
        );
    }
}

fn print_row(size: usize, ok: u64, dt: std::time::Duration, byte_factor: f64) {
    let lat_us = dt.as_secs_f64() * 1e6 / ok as f64;
    let mbps = (size as f64 * ok as f64 * byte_factor) / dt.as_secs_f64() / 1024.0 / 1024.0;
    let ops = ok as f64 / dt.as_secs_f64();
    println!("{:>5} {:>6.1} {:>7.1} {:>7.0}", size, lat_us, mbps, ops);
}
