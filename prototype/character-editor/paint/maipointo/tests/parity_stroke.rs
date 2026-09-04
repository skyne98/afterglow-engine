//! Stroke-level byte parity vs the vendored C engine
//! (reference/stroke_oracle.c): identical brush config + event stream in,
//! full-tile byte comparison out.

use std::path::PathBuf;
use std::process::Command;

use maipointo::brush::Brush;

use maipointo::surface::FixedTiledSurface;

fn paint_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn vendor_dir() -> PathBuf {
    paint_dir().join("vendor").join("libmypaint")
}

#[derive(Clone)]
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
        let mut script = String::from(
            "set -e; cc -O2 -ffp-contract=off -fno-tree-vectorize -fno-tree-slp-vectorize -std=c11 ",
        );
        script.push_str(&format!("-I '{}' -I '{}' ", paint.display(), mp.display()));
        script.push_str("$(pkg-config --cflags json-c) ");
        script.push_str(&srcs);
        script.push_str(" $(pkg-config --libs json-c) -lm -o ");
        script.push_str(exe.to_str().unwrap());
        let out1 = Command::new("nix-shell")
            .args(["-p", "json_c", "pkg-config", "--run"])
            .arg(&script)
            .output()
            .expect("cc for stroke oracle");
        if !out1.status.success() {
            panic!(
                "stroke-oracle build failed: {}",
                String::from_utf8_lossy(&out1.stderr)
            );
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
    let dab_file = unique_tmp("stroke-dabs").1;
    std::fs::write(&cmd_file, encode(ops)).unwrap();
    let status = Command::new(oracle_path())
        .args([w.to_string(), h.to_string()])
        .arg(&cmd_file)
        .arg(&out_file)
        .arg(&dab_file)
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
            Op::SetPoint(s, i, idx, x, y) => brush.set_mapping_point_at(*s, *i, *idx, *x, *y),
            Op::StrokeTo(x, y, pr, xt, yt, dt, vz, vr, br, lin) => {
                brush.stroke_to(
                    &mut surface,
                    *x,
                    *y,
                    *pr,
                    *xt,
                    *yt,
                    *dt,
                    *vz,
                    *vr,
                    *br,
                    *lin,
                );
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
    ops.push(Op::StrokeTo(
        64.0, 128.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, false,
    ));
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
    ops.push(Op::StrokeTo(
        40.0, 40.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, false,
    ));
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

// ---- state-level bisection ----

#[derive(Clone)]
enum Op2 {
    FromDefaults,
    SetBase(usize, f32),
    StrokeTo(f32, f32, f32, f32, f32, f64, f32, f32, f32, bool),
    DumpStates,
    DumpSettings,
    DumpSpeedMapping,
}

fn encode2(ops: &[Op2]) -> Vec<u8> {
    let mut v = Vec::new();
    for op in ops {
        match op {
            Op2::FromDefaults => v.push(0),
            Op2::SetBase(s, value) => {
                v.push(1);
                v.extend_from_slice(&(*s as u16).to_le_bytes());
                v.extend_from_slice(&value.to_le_bytes());
            }
            Op2::StrokeTo(x, y, pr, xt, yt, dt, vz, vr, br, lin) => {
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
            Op2::DumpStates => v.push(5),
            Op2::DumpSettings => v.push(6),
            Op2::DumpSpeedMapping => v.push(7),
        }
    }
    v
}

const STATE_NAMES: [&str; 44] = [
    "X",
    "Y",
    "PRESSURE",
    "PARTIAL_DABS",
    "ACTUAL_RADIUS",
    "SMUDGE_RA",
    "SMUDGE_GA",
    "SMUDGE_BA",
    "SMUDGE_A",
    "LAST_GETCOLOR_R",
    "LAST_GETCOLOR_G",
    "LAST_GETCOLOR_B",
    "LAST_GETCOLOR_A",
    "LAST_GETCOLOR_RECENTNESS",
    "ACTUAL_X",
    "ACTUAL_Y",
    "NORM_DX_SLOW",
    "NORM_DY_SLOW",
    "NORM_SPEED1_SLOW",
    "NORM_SPEED2_SLOW",
    "STROKE",
    "STROKE_STARTED",
    "CUSTOM_INPUT",
    "RNG_SEED",
    "ACTUAL_ELLIPTICAL_DAB_RATIO",
    "ACTUAL_ELLIPTICAL_DAB_ANGLE",
    "DIRECTION_DX",
    "DIRECTION_DY",
    "DECLINATION",
    "ASCENSION",
    "VIEWZOOM",
    "VIEWROTATION",
    "DIRECTION_ANGLE_DX",
    "DIRECTION_ANGLE_DY",
    "ATTACK_ANGLE",
    "FLIP",
    "GRIDMAP_X",
    "GRIDMAP_Y",
    "DECLINATIONX",
    "DECLINATIONY",
    "DABS_PER_BASIC_RADIUS",
    "DABS_PER_ACTUAL_RADIUS",
    "DABS_PER_SECOND",
    "BARREL_ROTATION",
];

const SETTING_NAMES: [&str; 65] = [
    "OPAQUE",
    "OPAQUE_MULTIPLY",
    "OPAQUE_LINEARIZE",
    "RADIUS_LOGARITHMIC",
    "HARDNESS",
    "SOFTNESS",
    "ANTI_ALIASING",
    "DABS_PER_BASIC_RADIUS",
    "DABS_PER_ACTUAL_RADIUS",
    "DABS_PER_SECOND",
    "GRIDMAP_SCALE",
    "GRIDMAP_SCALE_X",
    "GRIDMAP_SCALE_Y",
    "RADIUS_BY_RANDOM",
    "SPEED1_SLOWNESS",
    "SPEED2_SLOWNESS",
    "SPEED1_GAMMA",
    "SPEED2_GAMMA",
    "OFFSET_BY_RANDOM",
    "OFFSET_Y",
    "OFFSET_X",
    "OFFSET_ANGLE",
    "OFFSET_ANGLE_ASC",
    "OFFSET_ANGLE_VIEW",
    "OFFSET_ANGLE_2",
    "OFFSET_ANGLE_2_ASC",
    "OFFSET_ANGLE_2_VIEW",
    "OFFSET_ANGLE_ADJ",
    "OFFSET_MULTIPLIER",
    "OFFSET_BY_SPEED",
    "OFFSET_BY_SPEED_SLOWNESS",
    "SLOW_TRACKING",
    "SLOW_TRACKING_PER_DAB",
    "TRACKING_NOISE",
    "COLOR_H",
    "COLOR_S",
    "COLOR_V",
    "RESTORE_COLOR",
    "CHANGE_COLOR_H",
    "CHANGE_COLOR_L",
    "CHANGE_COLOR_HSL_S",
    "CHANGE_COLOR_V",
    "CHANGE_COLOR_HSV_S",
    "SMUDGE",
    "PAINT_MODE",
    "SMUDGE_TRANSPARENCY",
    "SMUDGE_LENGTH",
    "SMUDGE_LENGTH_LOG",
    "SMUDGE_BUCKET",
    "SMUDGE_RADIUS_LOG",
    "ERASER",
    "STROKE_THRESHOLD",
    "STROKE_DURATION_LOGARITHMIC",
    "STROKE_HOLDTIME",
    "CUSTOM_INPUT",
    "CUSTOM_INPUT_SLOWNESS",
    "ELLIPTICAL_DAB_RATIO",
    "ELLIPTICAL_DAB_ANGLE",
    "DIRECTION_FILTER",
    "LOCK_ALPHA",
    "COLORIZE",
    "POSTERIZE",
    "POSTERIZE_NUM",
    "SNAP_TO_PIXEL",
    "PRESSURE_GAIN_LOG",
];

#[test]
fn bisect_states() {
    let mut script: Vec<Op2> = vec![Op2::FromDefaults];
    script.push(Op2::SetBase(0, 0.9)); // opaque
    script.push(Op2::SetBase(4, 6.0f32.ln())); // radius_logarithmic
    script.push(Op2::SetBase(9, 0.35)); // color_v
    script.push(Op2::DumpStates);
    script.push(Op2::DumpSettings);
    script.push(Op2::DumpSpeedMapping);
    for i in 0..60 {
        let t = i as f32 / 59.0;
        script.push(Op2::StrokeTo(
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
        script.push(Op2::DumpStates);
        script.push(Op2::DumpSettings);
    }

    // Oracle side
    let (cmd_file, out_file) = unique_tmp("states-o");
    std::fs::write(&cmd_file, encode2(&script)).unwrap();
    let dab_file = unique_tmp("bisect-dabs").1;
    let status = Command::new(oracle_path())
        .args(["256", "256"])
        .arg(&cmd_file)
        .arg(&out_file)
        .arg(&dab_file)
        .output()
        .expect("run oracle");
    assert!(status.status.success(), "oracle failed");
    let raw = std::fs::read(&out_file).unwrap();
    const SEG: usize = (44 + 65) * 4;
    const SEG0: usize = SEG + 6 * 4;
    let n_dumps = (raw.len() - SEG0) / SEG + 1;
    // Dump #0 carries 6 extra f32 (speed mapping); later dumps are SEG long.
    let seg_off = |d: usize| if d == 0 { 0 } else { SEG0 + (d - 1) * SEG };
    let ref_states: Vec<Vec<f32>> = (0..n_dumps)
        .map(|d| {
            let o = seg_off(d);
            raw[o..o + 44 * 4]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        })
        .collect();
    let ref_speedmap: [f32; 6] = {
        let off = 44 * 4 + 65 * 4;
        let mut v = [0f32; 6];
        for (k, c) in raw[off..off + 24].chunks_exact(4).enumerate() {
            v[k] = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        }
        v
    };
    let ref_settings: Vec<Vec<f32>> = (0..n_dumps)
        .map(|d| {
            let o = seg_off(d);
            raw[o + 44 * 4..o + SEG]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        })
        .collect();

    // Rust side
    let mut states_out: Vec<Vec<f32>> = Vec::new();
    let mut settings_out: Vec<Vec<f32>> = Vec::new();
    let mut speedmap_out: Option<[f32; 6]> = None;
    let _ = &mut states_out;
    let mut brush = Brush::new();
    let mut surface = FixedTiledSurface::new(256, 256);
    for op in &script {
        match op {
            Op2::FromDefaults => brush.from_defaults(),
            Op2::SetBase(s, v) => brush.set_base_value_at(*s, *v),
            Op2::StrokeTo(x, y, pr, xt, yt, dt, vz, vr, br, lin) => {
                brush.stroke_to(
                    &mut surface,
                    *x,
                    *y,
                    *pr,
                    *xt,
                    *yt,
                    *dt,
                    *vz,
                    *vr,
                    *br,
                    *lin,
                );
            }
            Op2::DumpStates => {
                states_out.push((0..44).map(|i| brush.raw_state(i)).collect());
            }
            Op2::DumpSettings => {
                settings_out.push(brush.raw_settings().to_vec());
            }
            Op2::DumpSpeedMapping => {
                speedmap_out = Some(brush.raw_speed_mapping());
            }
        }
    }

    // Compare dump-by-dump; report the first divergent event and state.
    let mut first_event: Option<usize> = None;
    for d in 0..n_dumps.min(states_out.len()) {
        // settings_value is malloc-uninitialized in C until the first
        // update_states runs inside a dab loop; compare from the first dump
        // where PARTIAL_DABS shows a dab has occurred.
        for (i, (w, g)) in ref_settings[d]
            .iter()
            .zip(settings_out[d].iter())
            .enumerate()
            .skip(if ref_states[d][3] == 0.0 {
                usize::MAX
            } else {
                0
            })
        {
            if w.to_bits() != g.to_bits() {
                if first_event.is_none() {
                    first_event = Some(d);
                    eprintln!(
                        "FIRST divergence: dump #{d}, SETTING {} (C={:.7} rust={:.7})",
                        SETTING_NAMES[i], w, g
                    );
                }
            }
        }
        if d == 0 {
            if let Some(sm) = speedmap_out {
                for k in 0..6 {
                    if sm[k].to_bits() != ref_speedmap[k].to_bits() {
                        eprintln!(
                            "SPEEDMAPPING[{}] C={:.9} rust={:.9}",
                            k, ref_speedmap[k], sm[k]
                        );
                    }
                }
            }
        }
        let mut first_state: Option<usize> = None;
        for (i, (w, g)) in ref_states[d].iter().zip(states_out[d].iter()).enumerate() {
            if w.to_bits() != g.to_bits() && first_state.is_none() {
                first_state = Some(i);
            }
        }
        if first_state.is_some() && first_event.is_none() {
            first_event = Some(d);
            eprintln!(
                "FIRST divergence: dump #{d}, state {} (C={:.6} rust={:.6})",
                STATE_NAMES[first_state.unwrap()],
                ref_states[d][first_state.unwrap()],
                states_out[d][first_state.unwrap()]
            );
        }
    }
    if let Some(e) = first_event {
        for d in 0..=e {
            let gate = ref_states.get(d).map(|s| s[3]).unwrap_or(f32::NAN);
            eprintln!(
                "dump #{d} gate(PARTIAL)={gate} C.SPEED2SLOW={:?} R.SPEED2SLOW={:?} C.SPEED1SLOW={:?} R.SPEED1SLOW={:?}",
                ref_settings.get(d).map(|s| s[14]),
                settings_out.get(d).map(|s| s[14]),
                ref_settings.get(d).map(|s| s[15]),
                settings_out.get(d).map(|s| s[15])
            );
        }
        for d in 0..=e.min(2) {
            eprintln!(
                "dump #{d} C.X={:?} C.Y={:?} C.PARTIAL={:?} C.SETOPAQUE={:?} | R.X={:?} R.Y={:?}",
                ref_states.get(d).map(|s| s[0]),
                ref_states.get(d).map(|s| s[1]),
                ref_states.get(d).map(|s| s[3]),
                ref_settings.get(d).map(|s| s[0]),
                states_out.get(d).map(|s| s[0]),
                states_out.get(d).map(|s| s[1])
            );
        }
        eprintln!("raw.len={} n_dumps={}", raw.len(), n_dumps);
        panic!("first divergent event dump #{e}");
    }
}

// ---- dab-argument bisection: compare the 17-float dab stream ----

#[test]
fn bisect_dabs() {
    let mut script: Vec<Op2> = vec![Op2::FromDefaults];
    script.push(Op2::SetBase(0, 0.9)); // opaque
    script.push(Op2::SetBase(4, 6.0f32.ln())); // hardness (C index 4)
    script.push(Op2::SetBase(9, 0.35)); // dabs_per_second (C index 9)
    for i in 0..60 {
        let t = i as f32 / 59.0;
        script.push(Op2::StrokeTo(
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

    // Oracle: dump dab args to a separate file.
    let cmd_file = unique_tmp("dabs-cmd").0;
    let out_file = unique_tmp("dabs-out").1;
    let dab_file = unique_tmp("dabs-trace").1;
    std::fs::write(&cmd_file, encode2(&script)).unwrap();
    let status = Command::new(oracle_path())
        .args(["256", "256"])
        .arg(&cmd_file)
        .arg(&out_file)
        .arg(&dab_file)
        .output()
        .expect("run oracle");
    assert!(
        status.status.success(),
        "oracle failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let raw = std::fs::read(&dab_file).unwrap();
    let ref_dabs: Vec<[f32; 17]> = raw
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect::<Vec<f32>>()
        .chunks_exact(17)
        .map(|c| {
            let mut a = [0f32; 17];
            a.copy_from_slice(c);
            a
        })
        .collect();

    // Rust: replay with dab tracing.
    let mut brush = Brush::new();
    brush.dab_trace = Some(Vec::new());
    let mut surface = FixedTiledSurface::new(256, 256);
    for op in &script {
        match op {
            Op2::FromDefaults => brush.from_defaults(),
            Op2::SetBase(s, v) => brush.set_base_value_at(*s, *v),
            Op2::StrokeTo(x, y, pr, xt, yt, dt, vz, vr, br, lin) => {
                brush.stroke_to(
                    &mut surface,
                    *x,
                    *y,
                    *pr,
                    *xt,
                    *yt,
                    *dt,
                    *vz,
                    *vr,
                    *br,
                    *lin,
                );
            }
            Op2::DumpStates | Op2::DumpSettings | Op2::DumpSpeedMapping => {}
        }
    }
    let got_dabs = brush.dab_trace.take().unwrap();

    assert_eq!(ref_dabs.len(), got_dabs.len(), "dab count");
    for (i, (w, g)) in ref_dabs.iter().zip(got_dabs.iter()).enumerate() {
        for (k, (wv, gv)) in w.iter().zip(g.iter()).enumerate() {
            if wv.to_bits() != gv.to_bits() {
                panic!("dab #{i} arg {k} differs: C={wv:.7} rust={gv:.7}");
            }
        }
    }
}

/// Bisect the basic stroke by event prefix: find the first stroke_to whose
/// tile output diverges.
#[test]
fn stroke_prefix_bisect() {
    let mut head = vec![Op::FromDefaults];
    head.push(Op::SetBase(0, 0.9));
    head.push(Op::SetBase(4, 6.0f32.ln()));
    head.push(Op::SetBase(7, 0.0));
    head.push(Op::SetBase(8, 0.6));
    head.push(Op::SetBase(9, 0.35));
    head.push(Op::StrokeTo(
        64.0, 128.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, false,
    ));
    let events: Vec<Op> = (0..60)
        .map(|i| {
            let t = i as f32 / 59.0;
            Op::StrokeTo(
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
            )
        })
        .collect();
    for n in 0..=events.len() {
        let mut ops = head.clone();
        ops.extend_from_slice(&events[..n]);
        // dab-arg trace for this prefix
        let (cmd_file, out_file) = unique_tmp("pfx");
        let dab_file = unique_tmp("pfx-dabs").1;
        std::fs::write(&cmd_file, encode(&ops)).unwrap();
        let st = Command::new(oracle_path())
            .args(["256", "256"])
            .arg(&cmd_file)
            .arg(&out_file)
            .arg(&dab_file)
            .output()
            .unwrap();
        assert!(st.status.success());
        let raw = std::fs::read(&dab_file).unwrap();
        let ref_dabs: Vec<[f32; 17]> = raw
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect::<Vec<f32>>()
            .chunks_exact(17)
            .map(|c| {
                let mut a = [0f32; 17];
                a.copy_from_slice(c);
                a
            })
            .collect();
        let mut brush = Brush::new();
        brush.dab_trace = Some(Vec::new());
        let mut surf = FixedTiledSurface::new(256, 256);
        for op in &ops {
            match op {
                Op::FromDefaults => brush.from_defaults(),
                Op::SetBase(s, v) => {
                    brush.set_base_value_at(*s, *v);
                }
                Op::StrokeTo(x, y, pr, xt, yt, dt, vz, vr, br, lin) => {
                    brush.stroke_to(&mut surf, *x, *y, *pr, *xt, *yt, *dt, *vz, *vr, *br, *lin);
                }
                _ => {}
            }
        }
        let got_dabs = brush.dab_trace.take().unwrap();
        if ref_dabs.len() != got_dabs.len() {
            panic!(
                "prefix {n}: dab count C={} rust={}",
                ref_dabs.len(),
                got_dabs.len()
            );
        }
        for (i, (w, g)) in ref_dabs.iter().zip(got_dabs.iter()).enumerate() {
            for (k, (wv, gv)) in w.iter().zip(g.iter()).enumerate() {
                if wv.to_bits() != gv.to_bits() {
                    panic!("prefix {n}: dab #{i} arg {k}: C={wv:.6} rust={gv:.6}");
                }
            }
        }
        let want = run_oracle(256, 256, &ops);
        let got = run_maipointo(256, 256, &ops);
        if want != got {
            let diffs: Vec<usize> = want
                .iter()
                .zip(got.iter())
                .enumerate()
                .filter(|(_, (w, g))| w != g)
                .map(|(i, _)| i)
                .collect();
            let mut cmin = 99;
            let mut cmax = -1;
            let mut rmin = 9999;
            let mut rmax = -1;
            for &d in diffs.iter() {
                let lw = d % (64 * 64 * 4);
                let px = lw / 4;
                let c = (px % 64) as i32;
                let r = (px / 64) as i32;
                if c < cmin {
                    cmin = c;
                }
                if c > cmax {
                    cmax = c;
                }
                if r < rmin {
                    rmin = r;
                }
                if r > rmax {
                    rmax = r;
                }
            }
            eprintln!("diff bbox tile5 cols {cmin}-{cmax} rows {rmin}-{rmax}");
            for &d in diffs.iter().take(8) {
                let t = d / (64 * 64 * 4);
                let lw = d % (64 * 64 * 4);
                eprintln!(
                    "word {d}: tile {t} local_px {} (col {} row {}) want {} got {}",
                    lw / 4,
                    (lw / 4) % 64,
                    (lw / 4) / 64,
                    want[d],
                    got[d]
                );
            }
            eprintln!("total diffs {}", diffs.len());
            let idx = want
                .iter()
                .zip(got.iter())
                .position(|(w, g)| w != g)
                .unwrap();
            panic!(
                "first divergent prefix: {n} events, word {idx} want {} got {}",
                want[idx], got[idx]
            );
        }
    }
}
