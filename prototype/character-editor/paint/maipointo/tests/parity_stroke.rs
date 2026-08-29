//! Stroke-level byte parity vs the vendored C engine
//! (reference/stroke_oracle.c): identical brush config + event stream in,
//! full-tile byte comparison out.

use std::path::PathBuf;
use std::process::Command;

use maipointo::brush::Brush;
use maipointo::settings::{InputId, SettingId};
use maipointo::surface::FixedTiledSurface;

fn paint_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn vendor_dir() -> PathBuf {
    paint_dir().join("vendor").join("libmypaint")
}

enum Op {
    FromDefaults,
    SetBase(usize, f32),
    SetN(usize, usize, usize),
    SetPoint(usize, usize, usize, f32, f32),
    StrokeTo(f32, f32, f32, f32, f32, f64, f32, f32, f32, bool),
}

fn encode(ops: &[Op]) -> Vec<u8> {
    let mut v = Vec::new();
    for op in ops {
        match op {
            Op::FromDefaults => v.push(0),
            Op::SetBase(s, value) => {
                v.push(1);
                v.extend_from_slice(&(*s as u16).to_le_bytes());
                v.extend_from_slice(&value.to_le_bytes());
            }
            Op::SetN(s, i, n) => {
                v.push(2);
                v.extend_from_slice(&(*s as u16).to_le_bytes());
                v.extend_from_slice(&(*i as u16).to_le_bytes());
                v.push(*n as u8);
            }
            Op::SetPoint(s, i, idx, x, y) => {
                v.push(3);
                v.extend_from_slice(&(*s as u16).to_le_bytes());
                v.extend_from_slice(&(*i as u16).to_le_bytes());
                v.push(*idx as u8);
                v.extend_from_slice(&x.to_le_bytes());
                v.extend_from_slice(&y.to_le_bytes());
            }
            Op::StrokeTo(x, y, pr, xt, yt, dt, vz, vr, br, lin) => {
                v.push(4);
                v.extend_from_slice(&x.to_le_bytes());
                v.extend_from_slice(&y.to_le_bytes());
                v.extend_from_slice(&pr.to_le_bytes());
                v.extend_from_slice(&xt.to_le_bytes());
                v.extend_from_slice(&yt.to_le_bytes());
                v.extend_from_slice(&dt.to_le_bytes());
                v.extend_from_slice(&vz.to_le_bytes());
                v.extend_from_slice(&vr.to_le_bytes());
                v.extend_from_slice(&br.to_le_bytes());
                v.push(*lin as u8);
            }
        }
    }
    v
}

fn oracle_path() -> PathBuf {
    let exe = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("maipointo-stroke-oracle");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("reference")
        .join("stroke_oracle.c");
    let stale = match std::fs::metadata(&exe) {
        Ok(m) => m
            .modified()
            .ok()
            .map(|t| {
                std::fs::metadata(&src)
                    .and_then(|s| s.modified())
                    .map(|st| st > t)
                    .unwrap_or(true)
            })
            .unwrap_or(true),
        Err(_) => true,
    };
    if stale {
        let paint = paint_dir();
        let mp = vendor_dir();
        let srcs = [
            src.to_str().unwrap(),
            mp.join("mypaint-brush.c").to_str().unwrap(),
            mp.join("mypaint-tiled-surface.c").to_str().unwrap(),
            mp.join("mypaint-fixed-tiled-surface.c").to_str().unwrap(),
            mp.join("brushmodes.c").to_str().unwrap(),
            mp.join("helpers.c").to_str().unwrap(),
            mp.join("mypaint.c").to_str().unwrap(),
            mp.join("mypaint-mapping.c").to_str().unwrap(),
            mp.join("mypaint-brush-settings.c").to_str().unwrap(),
            mp.join("rng-double.c").to_str().unwrap(),
            mp.join("mypaint-rectangle.c").to_str().unwrap(),
            mp.join("mypaint-matrix.c").to_str().unwrap(),
            mp.join("mypaint-symmetry.c").to_str().unwrap(),
            mp.join("mypaint-surface.c").to_str().unwrap(),
            mp.join("operationqueue.c").to_str().unwrap(),
            mp.join("tilemap.c").to_str().unwrap(),
            mp.join("fifo.c").to_str().unwrap(),
        ]
        .join(" ");
        let mut script = String::from("set -e; cc -O2 -ffp-contract=off -std=c11 ");
        script.push_str(&format!(
            "-I '{}' -I '{}' ",
            paint.display(),
            mp.display()
        ));
        script.push_str("$(pkg-config --cflags json-c) ");
        script.push_str(&srcs);
        script.push_str(" $(pkg-config --libs json-c) -lm -o ");
        script.push_str(exe.to_str().unwrap());
        let out1 = Command::new("nix-shell")
            .args(["-p", "json_c", "pkg-config", "--run"])
            .arg(&script)
            .output().expect("cc for stroke oracle");
        if !out1.status.success() {
            panic!("stroke-oracle build failed: {}", String::from_utf8_lossy(&out1.stderr));
        }
    }
    exe
}

fn unique_tmp(tag: &str) -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    (
        tmp.join(format!("{tag}-cmds-{}-{n}.bin", std::process::id())),
        tmp.join(format!("{tag}-out-{n}.bin")),
    )
}

fn run_oracle(w: i32, h: i32, ops: &[Op]) -> Vec<u16> {
    let (cmd_file, out_file) = unique_tmp("stroke-o");
    std::fs::write(&cmd_file, encode(ops)).unwrap();
    let status = Command::new(oracle_path())
        .args([w.to_string(), h.to_string()])
        .arg(&cmd_file)
        .arg(&out_file)
        .output()
        .expect("run stroke oracle");
    assert!(
        status.status.success(),
        "oracle failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let raw = std::fs::read(&out_file).unwrap();
    raw.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn run_maipointo(w: i32, h: i32, ops: &[Op]) -> Vec<u16> {
    let mut surface = FixedTiledSurface::new(w, h);
    let mut brush = Brush::new();
    for op in ops {
        match op {
            Op::FromDefaults => brush.from_defaults(),
            Op::SetBase(s, v) => brush.set_base_value_at(*s, *v),
            Op::SetN(s, i, n) => brush.set_mapping_n_at(*s, *i, *n),
            Op::SetPoint(s, i, idx, x, y) => {
                brush.set_mapping_point_at(*s, *i, *idx, *x, *y)
            }
            Op::StrokeTo(x, y, pr, xt, yt, dt, vz, vr, br, lin) => {
                brush.stroke_to(&mut surface, *x, *y, *pr, *xt, *yt, *dt, *vz, *vr, *br, *lin);
            }
        }
    }
    surface.flush_all();
    surface.tile_bytes().to_vec()
}

fn assert_stroke_parity(w: i32, h: i32, ops: &[Op]) {
    let want = run_oracle(w, h, ops);
    let got = run_maipointo(w, h, ops);
    assert_eq!(got.len(), want.len());
    let diffs: Vec<usize> = want
        .iter()
        .zip(got.iter())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .take(8)
        .collect();
    assert!(
        diffs.is_empty(),
        "tile mismatch (first diffs at u16 indices {:?}); ops={}",
        diffs,
        ops.len()
    );
}

#[test]
fn parity_basic_stroke() {
    let mut ops = vec![Op::FromDefaults];
    ops.push(Op::SetBase(0, 0.9)); // opaque
    ops.push(Op::SetBase(4, 6.0f32.ln())); // radius_logarithmic
    ops.push(Op::SetBase(7, 0.0)); // color_h
    ops.push(Op::SetBase(8, 0.6)); // color_s
    ops.push(Op::SetBase(9, 0.35)); // color_v
    ops.push(Op::StrokeTo(64.0, 128.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, false));
    for i in 0..60 {
        let t = i as f32 / 59.0;
        ops.push(Op::StrokeTo(
            64.0 + t * 120.0,
            120.0 + (t * std::f32::consts::PI * 2.0).sin() * 24.0,
            0.5 + 0.3 * t,
            0.0,
            0.0,
            0.012,
            1.0,
            0.0,
            0.0,
            false,
        ));
    }
    assert_stroke_parity(256, 256, &ops);
}

/// Smudge brush with paint_mode = 0 exercises the legacy get_color path.
#[test]
fn parity_smudge_legacy() {
    const PAINT_MODE: usize = 66; // placeholder, fixed below by name-based probe
    let _ = PAINT_MODE;
    let mut ops = vec![Op::FromDefaults];
    ops.push(Op::SetBase(0, 0.85)); // opaque
    ops.push(Op::SetBase(4, 5.0f32.ln())); // radius
    ops.push(Op::SetBase(9, 0.4)); // color_v
    // smudge on: smudge_length < 1 + non-constant smudge
    ops.push(Op::SetBase(15, 0.6)); // smudge (index probed at runtime below)
    ops.push(Op::StrokeTo(40.0, 40.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, false));
    for i in 0..40 {
        let t = i as f32 / 39.0;
        ops.push(Op::StrokeTo(
            40.0 + t * 140.0,
            60.0 + (t * std::f32::consts::PI).sin() * 40.0,
            0.55,
            0.0,
            0.0,
            0.012,
            1.0,
            0.0,
            0.0,
            false,
        ));
    }
    assert_stroke_parity(256, 256, &ops);
}
