//! Ring buffer stress test: both directions, multiple payloads, latency + bandwidth.
//!
//! Native: native threads + heap ring buffer (Arc<Vec<u8>>).
//!
//! Directions:
//!   1. Main→Worker (write): main writes, worker drains — measures write side
//!   2. Worker→Main (read): worker fills, main reads — measures read side
//!   3. Round-trip: main writes request, worker echoes, main reads response

use afterglow_rpc::{RingBuffer, RpcError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

const CAPACITY: usize = 8 * 1024 * 1024;
const SIZES: &[usize] = &[64, 256, 1024, 4096, 16384, 65536, 262144, 1048576];
const N: usize = 2000;

fn main() {
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  NATIVE RING BUFFER STRESS TEST (native threads + heap memory)  ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!("  capacity: {} MiB, iterations: {}", CAPACITY / 1024 / 1024, N);
    println!();

    bench_write();
    bench_read();
    bench_roundtrip();
}

fn make_buf(capacity: usize) -> Arc<Vec<u8>> {
    let mut buf = vec![0u8; capacity + 12];
    RingBuffer::init(&mut buf);
    Arc::new(buf)
}

/// Direction 1: Main → Worker (write latency)
fn bench_write() {
    println!("┌─ Direction 1: Main → Worker (write) ──────────────────────────┐");
    println!("│ {:>8}  {:>10}  {:>12}  {:>10} │", "payload", "latency", "bandwidth", "writes/s");
    println!("│─────────────────────────────────────────────────────────────────│");

    for &size in SIZES {
        let payload = vec![0xAAu8; size];
        let buf = make_buf(CAPACITY);
        let buf2 = buf.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();

        let handle = std::thread::spawn(move || {
            let rb = RingBuffer::new(&buf2[..]);
            while !stop2.load(Ordering::Relaxed) {
                let _ = rb.read();
            }
        });

        let rb = RingBuffer::new(&buf[..]);
        let t0 = Instant::now();
        let mut ok = 0u64;
        for _ in 0..N {
            loop {
                match rb.write(&payload) {
                    Ok(()) => { ok += 1; break; }
                    Err(RpcError::BufferFull) => std::hint::spin_loop(),
                    _ => break,
                }
            }
        }
        let dt = t0.elapsed();
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();

        print_row(size, ok, dt);
    }
    println!("└─────────────────────────────────────────────────────────────────┘");
    println!();
}

/// Direction 2: Worker → Main (read latency)
fn bench_read() {
    println!("┌─ Direction 2: Worker → Main (read) ────────────────────────────┐");
    println!("│ {:>8}  {:>10}  {:>12}  {:>10} │", "payload", "latency", "bandwidth", "reads/s");
    println!("│─────────────────────────────────────────────────────────────────│");

    for &size in SIZES {
        let payload = vec![0xBBu8; size];
        let buf = make_buf(CAPACITY);
        let buf2 = buf.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let payload2 = payload.clone();

        let handle = std::thread::spawn(move || {
            let rb = RingBuffer::new(&buf2[..]);
            while !stop2.load(Ordering::Relaxed) {
                let _ = rb.write(&payload2);
            }
        });

        std::thread::sleep(std::time::Duration::from_millis(10));
        let rb = RingBuffer::new(&buf[..]);
        let t0 = Instant::now();
        let mut ok = 0u64;
        for _ in 0..N {
            loop {
                match rb.read() {
                    Ok(_) => { ok += 1; break; }
                    Err(RpcError::BufferEmpty) => std::hint::spin_loop(),
                    _ => break,
                }
            }
        }
        let dt = t0.elapsed();
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();

        print_row(size, ok, dt);
    }
    println!("└─────────────────────────────────────────────────────────────────┘");
    println!();
}

/// Round-trip: Main → Worker → Main
fn bench_roundtrip() {
    println!("┌─ Round-trip: Main → Worker → Main ─────────────────────────────┐");
    println!("│ {:>8}  {:>10}  {:>12}  {:>10} │", "payload", "latency", "bandwidth", "calls/s");
    println!("│─────────────────────────────────────────────────────────────────│");

    for &size in SIZES {
        let payload = vec![0xCCu8; size];
        let req = make_buf(CAPACITY);
        let resp = make_buf(CAPACITY);
        let req2 = req.clone();
        let resp2 = resp.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();

        let handle = std::thread::spawn(move || {
            let req_rb = RingBuffer::new(&req2[..]);
            let resp_rb = RingBuffer::new(&resp2[..]);
            while !stop2.load(Ordering::Relaxed) {
                match req_rb.read() {
                    Ok(data) => {
                        while resp_rb.write(&data).is_err() {
                            std::hint::spin_loop();
                        }
                    }
                    Err(RpcError::BufferEmpty) => std::hint::spin_loop(),
                    _ => break,
                }
            }
        });

        let req_rb = RingBuffer::new(&req[..]);
        let resp_rb = RingBuffer::new(&resp[..]);
        let t0 = Instant::now();
        let mut ok = 0u64;
        for _ in 0..N {
            loop {
                match req_rb.write(&payload) {
                    Ok(()) => break,
                    Err(RpcError::BufferFull) => std::hint::spin_loop(),
                    _ => break,
                }
            }
            loop {
                match resp_rb.read() {
                    Ok(_) => { ok += 1; break; }
                    Err(RpcError::BufferEmpty) => std::hint::spin_loop(),
                    _ => break,
                }
            }
        }
        let dt = t0.elapsed();
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();

        print_row(size, ok, dt);
    }
    println!("└─────────────────────────────────────────────────────────────────┘");
}

fn print_row(size: usize, ok: u64, dt: std::time::Duration) {
    let lat_us = dt.as_micros() as f64 / ok as f64;
    let mbps = (size as f64 * ok as f64) / dt.as_secs_f64() / 1024.0 / 1024.0;
    let ops = ok as f64 / dt.as_secs_f64();
    println!("│ {:>6} B  {:>7.1} µs  {:>8.1} MB/s  {:>8.0} │", size, lat_us, mbps, ops);
}
