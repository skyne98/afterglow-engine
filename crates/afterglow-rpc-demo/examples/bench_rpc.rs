//! Ring buffer stress test: both directions, multiple payloads, latency + bandwidth.
//!
//! Native: native threads + heap ring buffer (Arc<Vec<u8>>).
//!
//! Directions:
//!   1. Main→Worker (write): main writes, worker drains — measures write side
//!   2. Worker→Main (read): worker fills, main reads — measures read side
//!   3. Round-trip: main writes request, worker echoes, main reads response

use afterglow_rpc::{RingBuffer, RingBufferTransport, Transport, RpcError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const CAPACITY: usize = 8 * 1024 * 1024; // 8 MiB ring buffer
const SIZES: &[usize] = &[64, 256, 1024, 4096, 16384, 65536, 262144, 1048576];
const N: usize = 2000;

/// A shared ring buffer backed by Arc<Vec<u8>>. The Arc keeps the memory
/// alive; the RingBuffer borrows from it with an extended lifetime.
struct SharedRing {
    _buf: Arc<Vec<u8>>,
    rb: *const RingBuffer<'static>,
}

unsafe impl Send for SharedRing {}
unsafe impl Sync for SharedRing {}

impl SharedRing {
    fn new(capacity: usize) -> Self {
        let mut buf = vec![0u8; capacity + 12];
        RingBuffer::init(&mut buf);
        let arc = Arc::new(buf);
        let rb = RingBuffer::new(&arc[..]);
        // Leak the borrow — the Arc keeps the memory alive.
        let rb = Box::into_raw(Box::new(rb)) as *const RingBuffer<'static>;
        Self { _buf: arc, rb }
    }
    fn rb(&self) -> &RingBuffer<'static> { unsafe { &*self.rb } }
}

impl Drop for SharedRing {
    fn drop(&mut self) {
        unsafe { drop(Box::from_raw(self.rb as *mut RingBuffer<'static>)) }
    }
}

fn main() {
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  NATIVE RING BUFFER STRESS TEST (native threads + heap memory)  ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!("  ring buffer capacity: {} MiB, iterations per size: {}", CAPACITY / 1024 / 1024, N);
    println!();

    // --- 1. Main→Worker (write latency) ---
    println!("┌─ Direction 1: Main → Worker (write) ──────────────────────────┐");
    println!("│ {:>8}  {:>10}  {:>12}  {:>10} │", "payload", "latency", "bandwidth", "writes/s");
    println!("│─────────────────────────────────────────────────────────────────│");
    for &size in SIZES {
        let payload = vec![0xAAu8; size];
        let rb = SharedRing::new(CAPACITY);
        let rb2 = SharedRing::new(CAPACITY);
        // Worker drains: read from rb, discard
        // We need two SharedRings pointing to the same memory... actually
        // just use one: main writes, worker reads from the same buffer.
        let buf = Arc::new({
            let mut b = vec![0u8; CAPACITY + 12];
            RingBuffer::init(&mut b);
            b
        });
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
            // Spin until write succeeds (buffer may be full if worker is slow)
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

        let lat_us = dt.as_micros() as f64 / ok as f64;
        let mbps = (size as f64 * ok as f64) / dt.as_secs_f64() / 1024.0 / 1024.0;
        let wps = ok as f64 / dt.as_secs_f64();
        println!("│ {:>6} B  {:>7.1} µs  {:>8.1} MB/s  {:>8.0} │", size, lat_us, mbps, wps);
    }
    println!("└─────────────────────────────────────────────────────────────────┘");
    println!();

    // --- 2. Worker→Main (read latency) ---
    println!("┌─ Direction 2: Worker → Main (read) ────────────────────────────┐");
    println!("│ {:>8}  {:>10}  {:>12}  {:>10} │", "payload", "latency", "bandwidth", "reads/s");
    println!("│─────────────────────────────────────────────────────────────────│");
    for &size in SIZES {
        let payload = vec![0xBBu8; size];
        let buf = Arc::new({
            let mut b = vec![0u8; CAPACITY + 12];
            RingBuffer::init(&mut b);
            b
        });
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
        // Give the worker a moment to fill the buffer
        std::thread::sleep(std::time::Duration::from_millis(10));

        let rb = RingBuffer::new(&buf[..]);
        let t0 = Instant::now();
        let mut ok = 0u64;
        for _ in 0..N {
            match rb.read() {
                Ok(_) => { ok += 1; }
                Err(RpcError::BufferEmpty) => {
                    // Worker hasn't written yet, spin
                    loop {
                        match rb.read() {
                            Ok(_) => { ok += 1; break; }
                            Err(RpcError::BufferEmpty) => std::hint::spin_loop(),
                            _ => break,
                        }
                    }
                }
                _ => break,
            }
        }
        let dt = t0.elapsed();
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();

        let lat_us = dt.as_micros() as f64 / ok as f64;
        let mbps = (size as f64 * ok as f64) / dt.as_secs_f64() / 1024.0 / 1024.0;
        let rps = ok as f64 / dt.as_secs_f64();
        println!("│ {:>6} B  {:>7.1} µs  {:>8.1} MB/s  {:>8.0} │", size, lat_us, mbps, rps);
    }
    println!("└─────────────────────────────────────────────────────────────────┘");
    println!();

    // --- 3. Round-trip (both directions) ---
    println!("┌─ Round-trip: Main → Worker → Main ─────────────────────────────┐");
    println!("│ {:>8}  {:>10}  {:>12}  {:>10} │", "payload", "latency", "bandwidth", "calls/s");
    println!("│─────────────────────────────────────────────────────────────────│");
    for &size in SIZES {
        let payload = vec![0xCCu8; size];

        // Create request + response ring buffers
        let req = Arc::new({
            let mut b = vec![0u8; CAPACITY + 12];
            RingBuffer::init(&mut b);
            b
        });
        let resp = Arc::new({
            let mut b = vec![0u8; CAPACITY + 12];
            RingBuffer::init(&mut b);
            b
        });

        let req2 = req.clone();
        let resp2 = resp.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();

        let handle = std::thread::spawn(move || {
            // Worker: read from request, write to response (echo)
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

        // Main: write to request, read from response
        let req_rb = RingBuffer::new(&req[..]);
        let resp_rb = RingBuffer::new(&resp[..]);
        let t0 = Instant::now();
        let mut ok = 0u64;
        for _ in 0..N {
            // Write request
            loop {
                match req_rb.write(&payload) {
                    Ok(()) => break,
                    Err(RpcError::BufferFull) => std::hint::spin_loop(),
                    _ => break,
                }
            }
            // Read response
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

        let lat_us = dt.as_micros() as f64 / ok as f64;
        let mbps = (size as f64 * ok as f64 * 2.0) / dt.as_secs_f64() / 1024.0 / 1024.0;
        let cps = ok as f64 / dt.as_secs_f64();
        println!("│ {:>6} B  {:>7.1} µs  {:>8.1} MB/s  {:>8.0} │", size, lat_us, mbps, cps);
    }
    println!("└─────────────────────────────────────────────────────────────────┘");
}
