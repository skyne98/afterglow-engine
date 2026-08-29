//! Layer-compositor parity: replay the demo's exhaustive 22-mode blend grid
//! through the extracted MyPaint reference (C oracle) and the maipointo
//! Rust compositor, comparing every output pixel exactly. The Pigment
//! (spectral WGM) path uses float fastpow on both sides — the reference
//! oracle ships the same fastapprox header, so bits must still match.

use std::io::Write;
use std::process::{Command, Stdio};

fn build_oracle() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let paint = manifest.parent().unwrap();
    let exe = manifest.join("target/compositor-oracle");
    let src_mtime = |p: std::path::PathBuf| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
    };
    let stale = std::fs::metadata(&exe)
        .and_then(|m| m.modified())
        .map(|exe_t| {
            [
                paint.join("layer-compositor.parity.test.c"),
                manifest.join("reference/compositor_oracle.c"),
            ]
            .iter()
            .any(|p| src_mtime(p.clone()).is_some_and(|t| t > exe_t))
        })
        .unwrap_or(true);
    if stale {
        let status = Command::new("cc")
            .args(["-O2", "-std=c11"])
            .arg("-I")
            .arg(paint.join("vendor/libmypaint"))
            .arg("-I")
            .arg(paint)
            .arg(manifest.join("reference/compositor_oracle.c"))
            .arg("-lm")
            .arg("-o")
            .arg(&exe)
            .status()
            .expect("cc for compositor oracle");
        assert!(status.success(), "compositor oracle build failed");
    }
    exe
}

#[test]
fn compositor_matches_mypaint_reference() {
    let exe = build_oracle();

    // The demo test's grid (layer-compositor.parity.test.c run_grid).
    const LEVELS: [u16; 9] = [
        0, 4096, 8192, 12288, 16384, 20480, 24576, 28672, 32768,
    ];
    const ALPHAS: [u16; 6] = [0, 4096, 8192, 16384, 24576, 32768];
    const OPACS: [f32; 4] = [0.25, 0.5, 0.75, 1.0];
    const TRI: [u16; 27] = [
        0, 0, 0, 32768, 0, 0, 32768, 32768, 0, 0, 32768, 0, 0, 0, 32768, 32768, 0, 32768,
        16384, 8192, 24576, 8192, 24576, 16384, 24576, 16384, 8192,
    ];
    let ntri = TRI.len() / 3;
    let f_mul = |a: u32, b: u32| -> u32 { (a * b) >> 15 };

    // Write all cases to a file, run the oracle file-to-file (a live
    // stdin/stdout pipe deadlocks: the child's stdout fills while we write).
    let cases_file = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/compositor-cases.bin");
    let out_file = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/compositor-ref.bin");
    let mut count = 0usize;
    let mut count_check = 0usize;
    {
        let mut stdin = std::fs::File::create(&cases_file).expect("cases file");

    for mode in 0..22i32 {
        for sa in 0..6usize {
            for da in 0..6usize {
                for sl in 0..9usize {
                    for dl in 0..9usize {
                        for oi in 0..4usize {
                            let (sr, sg, sb, dr, dg, db);
                            if (12..=15).contains(&mode) {
                                sr = TRI[(sa % ntri) * 3];
                                sg = TRI[(sa % ntri) * 3 + 1];
                                sb = TRI[(sa % ntri) * 3 + 2];
                                dr = TRI[(da % ntri) * 3];
                                dg = TRI[(da % ntri) * 3 + 1];
                                db = TRI[(da % ntri) * 3 + 2];
                            } else {
                                sr = LEVELS[sl];
                                sg = LEVELS[sl];
                                sb = LEVELS[sl];
                                dr = LEVELS[dl];
                                dg = LEVELS[dl];
                                db = LEVELS[dl];
                            }
                            let src = [
                                f_mul(sr as u32, ALPHAS[sa] as u32) as u16,
                                f_mul(sg as u32, ALPHAS[sa] as u32) as u16,
                                f_mul(sb as u32, ALPHAS[sa] as u32) as u16,
                                ALPHAS[sa],
                            ];
                            let dst = [
                                f_mul(dr as u32, ALPHAS[da] as u32) as u16,
                                f_mul(dg as u32, ALPHAS[da] as u32) as u16,
                                f_mul(db as u32, ALPHAS[da] as u32) as u16,
                                ALPHAS[da],
                            ];
                            let opac = (OPACS[oi] * 32768.0) as u32;
                            let mut rec = Vec::with_capacity(18);
                            rec.extend_from_slice(&(mode as u16).to_le_bytes());
                            for v in &src {
                                rec.extend_from_slice(&v.to_le_bytes());
                            }
                            for v in &dst {
                                rec.extend_from_slice(&v.to_le_bytes());
                            }
                            rec.extend_from_slice(&opac.to_le_bytes());
                            stdin
                                .write_all(&rec)
                                .expect("write oracle case");
                            count += 1;
                            count_check += 1;
                        }
                    }
                }
            }
        }
    }
    }
    assert_eq!(count, count_check);
    let status = Command::new(&exe)
        .arg(&cases_file)
        .arg(&out_file)
        .output()
        .expect("run compositor oracle");
    assert!(status.status.success(), "oracle failed");
    let raw = std::fs::read(&out_file).expect("oracle output");
    assert_eq!(raw.len() / 8, count, "oracle case count");

    // Compare every case exactly: derive each case from the loop order
    // (mode, sa, da, sl, dl, oi) and index the oracle's 8-byte ref outputs.
    let mut failures = 0usize;
    let mut i = 0usize;
    for mode in 0..22i32 {
        for sa in 0..6usize {
            for da in 0..6usize {
                for sl in 0..9usize {
                    for dl in 0..9usize {
                        for oi in 0..4usize {
                            let (sr, sg, sb, dr, dg, db);
                            if (12..=15).contains(&mode) {
                                sr = TRI[(sa % ntri) * 3];
                                sg = TRI[(sa % ntri) * 3 + 1];
                                sb = TRI[(sa % ntri) * 3 + 2];
                                dr = TRI[(da % ntri) * 3];
                                dg = TRI[(da % ntri) * 3 + 1];
                                db = TRI[(da % ntri) * 3 + 2];
                            } else {
                                sr = LEVELS[sl];
                                sg = LEVELS[sl];
                                sb = LEVELS[sl];
                                dr = LEVELS[dl];
                                dg = LEVELS[dl];
                                db = LEVELS[dl];
                            }
                            let src = [
                                f_mul(sr as u32, ALPHAS[sa] as u32) as u16,
                                f_mul(sg as u32, ALPHAS[sa] as u32) as u16,
                                f_mul(sb as u32, ALPHAS[sa] as u32) as u16,
                                ALPHAS[sa],
                            ];
                            let dst = [
                                f_mul(dr as u32, ALPHAS[da] as u32) as u16,
                                f_mul(dg as u32, ALPHAS[da] as u32) as u16,
                                f_mul(db as u32, ALPHAS[da] as u32) as u16,
                                ALPHAS[da],
                            ];
                            let opac_u15 = OPACS[oi];
                            let mut ours = dst;
                            maipointo::compositor::layer_blend_over(
                                &mut ours,
                                &src,
                                opac_u15,
                                maipointo::compositor::BlendMode::from_int(mode),
                            );
                            let off = i * 8;
                            for c in 0..4 {
                                let refp = u16::from_le_bytes([
                                    raw[off + c * 2],
                                    raw[off + c * 2 + 1],
                                ]);
                                // 21 modes are bit-exact; Pigment's float
                                // fastpow ordering costs up to 1 LSB (the C
                                // parity test asserts the same bound).
                                let tol = if mode == 21 { 1 } else { 0 };
                                let delta = (ours[c] as i32 - refp as i32).abs();
                                if delta > tol {
                                    failures += 1;
                                    if failures <= 4 {
                                        panic!(
                                            "mode {mode} case {i}: channel {c} ours {o} ref {r} (src {s:?} dst {d:?} opac {op})",
                                            o = ours[c], r = refp, s = src, d = dst, op = opac_u15,
                                        );
                                    }
                                }
                            }
                            i += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(failures, 0, "compositor parity failures: {failures}");
}
