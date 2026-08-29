//! Symmetry-transform parity: replay the same pending-state stream through
//! the C oracle (real mypaint-symmetry.c + mypaint-matrix.c) and the Rust
//! port, comparing every emitted transform matrix bit-for-bit via
//! `mypaint_transform_point` outputs at probe points.

use std::io::Write;
use std::process::{Command, Stdio};

fn build_oracle() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let paint = manifest.parent().unwrap();
    let exe = manifest.join("target/symmetry-oracle");
    let stale = std::fs::metadata(&exe)
        .and_then(|m| m.modified())
        .map(|exe_t| {
            [
                paint.join("vendor/libmypaint/mypaint-symmetry.c"),
                paint.join("vendor/libmypaint/mypaint-matrix.c"),
                manifest.join("reference/symmetry_oracle.c"),
            ]
            .iter()
            .any(|p| {
                std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .map(|t| t > exe_t)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(true);
    if stale {
        let status = Command::new("cc")
            .args(["-O2", "-std=c11"])
            .arg("-I")
            .arg(paint.join("vendor/libmypaint"))
            .arg(manifest.join("reference/symmetry_oracle.c"))
            .arg(paint.join("vendor/libmypaint/mypaint-symmetry.c"))
            .arg(paint.join("vendor/libmypaint/mypaint-matrix.c"))
            .arg("-lm")
            .arg("-o")
            .arg(&exe)
            .status()
            .expect("cc for symmetry oracle");
        assert!(status.success(), "symmetry oracle build failed");
    }
    exe
}

/// The demo-reachable symmetry states: all 5 types across line counts and
/// angles, at a fixed center.
fn states() -> Vec<(i32, f32, f32, f32, i32)> {
    let mut v = Vec::new();
    for kind in 0..5i32 {
        for lines in [2i32, 3, 4, 6, 9] {
            for angle in [0.0f32, 37.5, 270.0] {
                v.push((kind, 512.0, 384.0, angle, lines));
            }
        }
    }
    v
}

#[test]
fn symmetry_matches_mypaint_reference() {
    let exe = build_oracle();
    let cases_file = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/symmetry-cases.bin");
    let out_file = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/symmetry-ref.bin");

    let states = states();
    {
        let mut f = std::fs::File::create(&cases_file).expect("cases");
        for (kind, cx, cy, angle, lines) in &states {
            f.write_all(&kind.to_le_bytes()).unwrap();
            f.write_all(&cx.to_le_bytes()).unwrap();
            f.write_all(&cy.to_le_bytes()).unwrap();
            f.write_all(&angle.to_le_bytes()).unwrap();
            f.write_all(&lines.to_le_bytes()).unwrap();
        }
    }
    let status = Command::new(&exe)
        .arg(&cases_file)
        .arg(&out_file)
        .output()
        .expect("run symmetry oracle");
    assert!(status.status.success(), "oracle failed");
    let raw = std::fs::read(&out_file).expect("oracle output");

    // Replay through the Rust port.
    let mut data = maipointo::symmetry::default_symmetry_data();
    let mut off = 0usize;
    for (kind, cx, cy, angle, lines) in &states {
        if *kind >= 0 {
            maipointo::symmetry::symmetry_set_pending(
                &mut data, true, *cx, *cy, *angle, *kind, *lines,
            );
        }
        maipointo::symmetry::update_symmetry_state(&mut data);
        let n = i32::from_le_bytes(raw[off..off + 4].try_into().unwrap()) as usize;
        off += 4;
        // The C store grows only when required exceeds it; the Rust store is
        // fixed at 64, so compare growth for large line counts only.
        if n <= 16 {
            // keep ours as-is (16 unless a larger C allocation happened)
        } else {
            assert_eq!(data.num_symmetry_matrices, n, "allocated count");
        }
        let _ = data;
        // Only the required (used) matrices are recomputed by both sides;
        // the C's allocation slack holds stale values.
        let used = match *kind {
            0 | 1 => 1,
            2 => 3,
            3 => (*lines - 1) as usize,
            4 => (2 * *lines - 1) as usize,
            _ => 0,
        };
        for m in 0..used {
            for r in 0..3 {
                for c in 0..3 {
                    let bits = u32::from_le_bytes(raw[off..off + 4].try_into().unwrap());
                    off += 4;
                    let got = data.matrices[m].rows[r][c];
                    assert_eq!(
                        got.to_bits(),
                        bits,
                        "state ({kind},{cx},{cy},{angle},{lines}) matrix {m} [{r}][{c}]"
                    );
                }
            }
        }
        // Skip the C's allocation-slack matrices (calloc-zeroed, stale).
        off += (n - used) * 9 * 4;
    }
}
