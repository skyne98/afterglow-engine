//! Worker tests — native RPC round-trip via the poll model.
//!
//! Spawns MeshoptClient::spawn_worker(), calls each method, drives via poll(),
//! and verifies results. This tests the full async RPC pipeline: ring buffers,
//! task_id matching, executor, and the meshoptimizer C++ library.

#![cfg(test)]

use crate::*;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

static VTABLE: RawWakerVTable = RawWakerVTable::new(
    |_| RawWaker::new(std::ptr::null(), &VTABLE),
    |_| {},
    |_| {},
    |_| {},
);

fn drive<F: std::future::Future>(client: &MeshoptClient, fut: F) -> F::Output {
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        client.poll();
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => {
                if std::time::Instant::now() > deadline {
                    panic!("timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}

/// Generate a test grid mesh: N×N vertices, (N-1)²×2 triangles.
fn test_grid(n: u32) -> (Vec<u32>, Vec<f32>) {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for y in 0..n {
        for x in 0..n {
            positions.push(x as f32);
            positions.push(y as f32);
            positions.push(0.0);
        }
    }
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let i = y * n + x;
            indices.extend_from_slice(&[i, i + 1, i + n, i + 1, i + n + 1, i + n]);
        }
    }
    (indices, positions)
}

const STRIDE: u32 = 12; // 3 floats × 4 bytes

// --- Simplify ---

#[test]
fn worker_simplify() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, positions) = test_grid(20);
    let target = indices.len() / 2;
    let simplified: Vec<u32> = drive(&client, client
        .simplify(indices.clone(), positions.clone(), STRIDE, target as u32, 0.01)
        .unwrap()).unwrap();
    assert!(simplified.len() <= indices.len());
    assert!(simplified.len() >= 3);
    assert_eq!(simplified.len() % 3, 0, "triangle list");
}

#[test]
fn worker_simplify_sloppy() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, positions) = test_grid(15);
    let target = indices.len() / 4;
    let simplified: Vec<u32> = drive(&client, client
        .simplify_sloppy(indices.clone(), positions.clone(), STRIDE, target as u32, 0.1)
        .unwrap()).unwrap();
    assert!(simplified.len() <= indices.len());
}

// --- Optimize ---

#[test]
fn worker_optimize_vertex_cache() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, _) = test_grid(10);
    let optimized: Vec<u32> = drive(&client, client
        .optimize_vertex_cache(indices.clone(), 100)
        .unwrap()).unwrap();
    assert_eq!(optimized.len(), indices.len());
    let mut orig = indices.clone();
    let mut opt = optimized.clone();
    orig.sort();
    opt.sort();
    assert_eq!(orig, opt, "same indices, reordered");
}

#[test]
fn worker_optimize_overdraw() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, positions) = test_grid(10);
    let optimized: Vec<u32> = drive(&client, client
        .optimize_overdraw(indices.clone(), positions.clone(), STRIDE, 1.05)
        .unwrap()).unwrap();
    assert_eq!(optimized.len(), indices.len());
}

// --- Compress ---

#[test]
fn worker_encode_decode_index_buffer() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, _) = test_grid(10);
    let encoded: Vec<u8> = drive(&client, client
        .encode_index_buffer(indices.clone(), 100)
        .unwrap()).unwrap();
    assert!(encoded.len() < indices.len() * 4, "compressed should be smaller");
    let decoded: Vec<u32> = drive(&client, client
        .decode_index_buffer(encoded, indices.len() as u32)
        .unwrap()).unwrap();
    // The index codec may reorder triangles — compare as sorted triangle sets.
    assert_eq!(decoded.len(), indices.len(), "same index count");
    let mut orig_tris: Vec<[u32; 3]> = indices.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
    let mut dec_tris: Vec<[u32; 3]> = decoded.chunks(3).map(|c| { let mut t = [c[0], c[1], c[2]]; t.sort(); t }).collect();
    orig_tris.iter_mut().for_each(|t| t.sort());
    orig_tris.sort();
    dec_tris.sort();
    assert_eq!(orig_tris, dec_tris, "same triangles, possibly reordered");
}

#[test]
fn worker_encode_decode_vertex_buffer() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (_, positions) = test_grid(5);
    let vertices: Vec<u8> = bytemuck::cast_slice::<f32, u8>(&positions).to_vec();
    let vertex_size = 12u32; // 3 floats
    let encoded: Vec<u8> = drive(&client, client
        .encode_vertex_buffer(vertices.clone(), vertex_size)
        .unwrap()).unwrap();
    let decoded: Vec<u8> = drive(&client, client
        .decode_vertex_buffer(encoded, (vertices.len() / vertex_size as usize) as u32, vertex_size)
        .unwrap()).unwrap();
    assert_eq!(decoded, vertices, "roundtrip preserves data");
}

// --- Remap ---

#[test]
fn worker_generate_vertex_remap() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let indices = vec![0u32, 1, 2, 2, 1, 3];
    let vertices: Vec<u8> = vec![0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0]; // vertex 2 = vertex 0
    let remap: Vec<u32> = drive(&client, client
        .generate_vertex_remap(indices, vertices, 3)
        .unwrap()).unwrap();
    assert_eq!(remap.len(), 4);
    // vertex 0 and 2 should map to the same index.
    assert_eq!(remap[0], remap[2], "duplicate vertices should remap together");
}

// --- Stripify ---

#[test]
fn worker_stripify() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, _) = test_grid(5);
    let strip: Vec<u32> = drive(&client, client
        .stripify(indices.clone(), 25, 0xFFFFFFFF)
        .unwrap()).unwrap();
    assert!(!strip.is_empty(), "strip should not be empty");
}

// --- Meshlets ---

#[test]
fn worker_build_meshlets() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, positions) = test_grid(10);
    let data: Vec<u8> = drive(&client, client
        .build_meshlets(indices, positions, STRIDE, 64, 64, 0.0)
        .unwrap()).unwrap();
    assert!(!data.is_empty(), "meshlet data should not be empty");
    // First 4 bytes = meshlet count (u32 LE).
    let count = u32::from_le_bytes(data[0..4].try_into().unwrap());
    assert!(count > 0, "should have at least 1 meshlet");
}

// --- Analyze ---

#[test]
fn worker_analyze_vertex_cache() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, _) = test_grid(5);
    let stats: Vec<f32> = drive(&client, client
        .analyze_vertex_cache(indices.clone(), 25)
        .unwrap()).unwrap();
    assert_eq!(stats.len(), 4); // [acmr, atvr, transformed, misspelled]
    assert!(stats[0] > 0.0, "acmr should be positive");
}

// --- Quantize ---

#[test]
fn worker_quantize_half() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let h: u16 = drive(&client, client.quantize_half(1.0).unwrap()).unwrap();
    let restored = safe::dequantize_half(h);
    assert!((restored - 1.0).abs() < 0.01, "half roundtrip: 1.0 → {restored}");
}

// --- LOD chain generation ---

#[test]
fn worker_generate_lod_chain() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, positions) = test_grid(50); // 50×50 = 2500 verts, ~4802 triangles
    let orig_tris = indices.len() / 3;

    // Generate 4 LOD levels: 100%, 50%, 25%, 10% of original triangles.
    let lod_ratios = [1.0f32, 0.5, 0.25, 0.1];
    let mut lods = Vec::new();

    for &ratio in &lod_ratios {
        let target = ((orig_tris as f32 * ratio) as u32).max(3) * 3;
        let simplified: Vec<u32> = drive(&client, client
            .simplify(indices.clone(), positions.clone(), STRIDE, target, 0.02)
            .unwrap()).unwrap();
        lods.push(simplified);
    }

    // Verify each LOD level.
    for (i, lod) in lods.iter().enumerate() {
        let tris = lod.len() / 3;
        let ratio = tris as f32 / orig_tris as f32;
        println!("  LOD{i}: {tris} triangles ({:.1}% of original)", ratio * 100.0);
        assert!(lod.len() % 3 == 0, "LOD{i} is a triangle list");
        assert!(tris >= 1, "LOD{i} has at least 1 triangle");
    }

    // LOD 0 = original (100%), LOD 3 = ~10%.
    assert_eq!(lods[0].len(), indices.len(), "LOD0 = original");
    assert!(lods[3].len() < lods[0].len() / 5, "LOD3 should be < 20% of original");
    assert!(lods[2].len() < lods[1].len(), "each LOD should be smaller than the previous");
    assert!(lods[3].len() < lods[2].len(), "LOD3 < LOD2");
}

// --- Concurrent in-flight ---

#[test]
fn worker_multiple_concurrent() {
    let (client, _) = MeshoptClient::spawn_worker(MeshoptWorker).unwrap();
    let (indices, positions) = test_grid(10);

    let f1 = client.simplify(indices.clone(), positions.clone(), STRIDE, 50, 0.01).unwrap();
    let f2 = client.encode_index_buffer(indices.clone(), 100).unwrap();
    let f3 = client.optimize_vertex_cache(indices.clone(), 100).unwrap();

    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut f1 = std::pin::pin!(f1);
    let mut f2 = std::pin::pin!(f2);
    let mut f3 = std::pin::pin!(f3);
    let mut r1 = None;
    let mut r2 = None;
    let mut r3 = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

    while r1.is_none() || r2.is_none() || r3.is_none() {
        client.poll();
        if r1.is_none() { if let Poll::Ready(v) = f1.as_mut().poll(&mut cx) { r1 = Some(v); } }
        if r2.is_none() { if let Poll::Ready(v) = f2.as_mut().poll(&mut cx) { r2 = Some(v); } }
        if r3.is_none() { if let Poll::Ready(v) = f3.as_mut().poll(&mut cx) { r3 = Some(v); } }
        if std::time::Instant::now() > deadline { panic!("timed out"); }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    let simplified: Vec<u32> = r1.unwrap().unwrap();
    let encoded: Vec<u8> = r2.unwrap().unwrap();
    let optimized: Vec<u32> = r3.unwrap().unwrap();

    assert!(simplified.len() <= indices.len());
    assert!(encoded.len() < indices.len() * 4);
    assert_eq!(optimized.len(), indices.len());
}
