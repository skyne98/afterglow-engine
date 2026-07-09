//! Ring buffer RPC benchmark (same-process baseline).
use afterglow_rpc_demo::{spawn_worker, PhysicsWorker};

fn main() {
    let (client, _events) = spawn_worker(PhysicsWorker);
    let next = client.step(vec![0.0, 1.0, 2.0], 0.5).unwrap();
    assert_eq!(next, vec![0.5, 1.5, 2.5]);

    println!("=== Ring Buffer RPC (same-process, native) ===");
    println!("payload   latency   throughput");
    let n = 5000;
    for size in [0, 16, 64, 256, 1024, 4096, 16384, 65536, 262144] {
        let state: Vec<f32> = (0..size).map(|i| i as f32).collect();
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            let _ = client.step(state.clone(), 0.0).unwrap();
        }
        let dt = t0.elapsed();
        let lat_us = dt.as_micros() as f64 / n as f64;
        let bytes = size * 4;
        let mbps = if bytes > 0 {
            (bytes as f64 * n as f64 * 2.0) / dt.as_secs_f64() / 1024.0 / 1024.0
        } else { 0.0 };
        println!("{:6} B  {:7.3} ms  {:7.1} MB/s", bytes, lat_us / 1000.0, mbps);
    }
}
