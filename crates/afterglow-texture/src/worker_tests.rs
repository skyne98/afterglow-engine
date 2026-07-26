//! Worker tests — native RPC round-trip via the poll model.

#![cfg(test)]

use crate::*;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

static VTABLE: RawWakerVTable = RawWakerVTable::new(
    |_| RawWaker::new(std::ptr::null(), &VTABLE),
    |_| {},
    |_| {},
    |_| {},
);

fn drive<F: std::future::Future>(client: &TextureClient, fut: F) -> F::Output {
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

fn make_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push((x * 255 / w) as u8); // R: gradient
            data.push((y * 255 / h) as u8); // G: gradient
            data.push(((x + y) * 128 / (w + h)) as u8); // B
            data.push(255); // A: opaque
        }
    }
    data
}

// --- generate_mips ---

#[test]
fn worker_generate_mips() {
    let (client, _) = TextureClient::spawn_worker(TextureWorker::default()).unwrap();
    let rgba = make_rgba(64, 64);
    let result: Vec<u8> = drive(&client, client.generate_mips(rgba, 64, 64).unwrap()).unwrap();

    // Parse: [count(u32)][w0][h0][len0][data0...][w1]...
    assert!(result.len() > 4, "should have serialized mips");
    let count = u32::from_le_bytes(result[0..4].try_into().unwrap());
    assert!(
        count >= 4,
        "should have at least 4 mip levels for 64×64, got {count}"
    );
}

#[test]
fn worker_generate_mips_small() {
    let (client, _) = TextureClient::spawn_worker(TextureWorker::default()).unwrap();
    let rgba = make_rgba(2, 2);
    let result: Vec<u8> = drive(&client, client.generate_mips(rgba, 2, 2).unwrap()).unwrap();
    let count = u32::from_le_bytes(result[0..4].try_into().unwrap());
    assert_eq!(count, 2, "2×2 should have 2 mips (2×2 + 1×1)");
}

// --- downscale ---

#[test]
fn worker_downscale() {
    let (client, _) = TextureClient::spawn_worker(TextureWorker::default()).unwrap();
    let rgba = make_rgba(32, 32);
    let result: Vec<u8> = drive(
        &client,
        client.downscale(rgba.clone(), 32, 32, 16, 16).unwrap(),
    )
    .unwrap();
    // Result should be 16×16×4 = 1024 bytes.
    assert_eq!(result.len(), 16 * 16 * 4, "downscaled to 16×16 RGBA");
}

#[test]
fn worker_downscale_non_uniform() {
    let (client, _) = TextureClient::spawn_worker(TextureWorker::default()).unwrap();
    let rgba = make_rgba(64, 32);
    let result: Vec<u8> = drive(&client, client.downscale(rgba, 64, 32, 32, 16).unwrap()).unwrap();
    assert_eq!(result.len(), 32 * 16 * 4, "downscaled to 32×16 RGBA");
}

#[test]
fn worker_downscale_preserves_color() {
    let (client, _) = TextureClient::spawn_worker(TextureWorker::default()).unwrap();
    // All-white 4×4 → 2×2 should still be all-white.
    let rgba = vec![255; 4 * 4 * 4];
    let result: Vec<u8> = drive(&client, client.downscale(rgba, 4, 4, 2, 2).unwrap()).unwrap();
    for &b in &result {
        assert_eq!(b, 255, "all-white downscale should stay white");
    }
}

// --- transcode (invalid data) ---

#[test]
fn worker_transcode_invalid_returns_error() {
    let (client, _) = TextureClient::spawn_worker(TextureWorker::default()).unwrap();
    let result = drive(&client, client.transcode(vec![0; 10], FORMAT_BC7).unwrap());
    assert!(result.is_err(), "invalid data should error");

    let result = drive(&client, client.transcode(vec![0; 10], FORMAT_ASTC).unwrap());
    assert!(result.is_err());

    let result = drive(&client, client.transcode(vec![0; 10], FORMAT_RGBA).unwrap());
    assert!(result.is_err());
}

#[test]
fn worker_transcode_unknown_format() {
    let (client, _) = TextureClient::spawn_worker(TextureWorker::default()).unwrap();
    let result = drive(&client, client.transcode(vec![0; 10], 99).unwrap());
    assert!(result.is_err());
}

#[test]
fn worker_transcodes_a_confined_source_range_without_caller_bytes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../afterglow-web/web/assets");
    let basis = root.join("checker.basis");
    let Ok(metadata) = std::fs::metadata(&basis) else {
        return;
    };
    TextureWorker::set_asset_root(afterglow_assets::AssetRoot::new(&root).unwrap());
    let (client, _) = TextureClient::spawn_worker(TextureWorker::default()).unwrap();
    let source = drive(&client, client.open_source("checker.basis".into()).unwrap()).unwrap();
    let result: Vec<u8> = drive(
        &client,
        client
            .transcode_range(source, 0, metadata.len() as u32, FORMAT_BC7)
            .unwrap(),
    )
    .unwrap();
    assert!(result.len() >= 16);
    assert!(u32::from_le_bytes(result[0..4].try_into().unwrap()) >= 1);
    let oversized = drive(
        &client,
        client
            .transcode_range(source, 0, (TEXTURE_INPUT_BYTES + 1) as u32, FORMAT_BC7)
            .unwrap(),
    );
    assert!(
        oversized
            .unwrap_err()
            .to_string()
            .contains("input capacity")
    );
}

// --- concurrent ---

#[test]
fn worker_multiple_concurrent() {
    let (client, _) = TextureClient::spawn_worker(TextureWorker::default()).unwrap();
    let rgba = make_rgba(16, 16);

    let f1 = client.generate_mips(rgba.clone(), 16, 16).unwrap();
    let f2 = client.downscale(rgba.clone(), 16, 16, 8, 8).unwrap();
    let f3 = client.downscale(rgba, 16, 16, 4, 4).unwrap();

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
        if r1.is_none() {
            if let Poll::Ready(v) = f1.as_mut().poll(&mut cx) {
                r1 = Some(v);
            }
        }
        if r2.is_none() {
            if let Poll::Ready(v) = f2.as_mut().poll(&mut cx) {
                r2 = Some(v);
            }
        }
        if r3.is_none() {
            if let Poll::Ready(v) = f3.as_mut().poll(&mut cx) {
                r3 = Some(v);
            }
        }
        if std::time::Instant::now() > deadline {
            panic!("timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    // Verify results.
    let mips: Vec<u8> = r1.unwrap().unwrap();
    assert!(mips.len() > 4, "mips result non-empty");

    let ds8: Vec<u8> = r2.unwrap().unwrap();
    assert_eq!(ds8.len(), 8 * 8 * 4, "8×8 downscale");

    let ds4: Vec<u8> = r3.unwrap().unwrap();
    assert_eq!(ds4.len(), 4 * 4 * 4, "4×4 downscale");
}
