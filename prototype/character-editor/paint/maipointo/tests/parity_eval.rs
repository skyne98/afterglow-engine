//! Byte-exact parity for `mapping.rs` and `rngdouble.rs` against the
//! vendored C (`mypaint-mapping.c`, `rng-double.c`) via
//! `reference/eval_oracle.c`.

use std::path::PathBuf;
use std::process::Command;

use maipointo::mapping::Mapping;
use maipointo::rngdouble::RngDouble;
use maipointo::settings::{input_index, setting_index, INPUT_INFOS, SETTING_INFOS};

fn paint_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn vendor_dir() -> PathBuf {
    paint_dir().join("vendor").join("libmypaint")
}

fn oracle_path() -> PathBuf {
    let exe = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("maipointo-eval-oracle");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("reference")
        .join("eval_oracle.c");
    let stale = std::fs::metadata(&exe)
        .and_then(|m| m.modified())
        .map(|exe_t| std::fs::metadata(&src).and_then(|s| s.modified()).map(|s| s > exe_t))
        .unwrap_or(Ok(true))
        .unwrap_or(true);
    if stale {
        let status = Command::new("cc")
            .arg("-O2")
            .arg("-ffp-contract=off")
            .arg("-fno-tree-vectorize")
            .arg("-fno-tree-slp-vectorize")
            .arg("-std=c11")
            .arg("-I")
            .arg(paint_dir())
            .arg("-I")
            .arg(vendor_dir())
            .arg(&src)
            .arg(vendor_dir().join("mypaint-mapping.c"))
            .arg(vendor_dir().join("rng-double.c"))
            .arg(vendor_dir().join("helpers.c"))
            .arg(vendor_dir().join("mypaint-tiled-surface.c"))
            .arg(vendor_dir().join("brushmodes.c"))
            .arg(vendor_dir().join("mypaint.c"))
            .arg(vendor_dir().join("mypaint-brush-settings.c"))
            .arg(vendor_dir().join("mypaint-rectangle.c"))
            .arg(vendor_dir().join("mypaint-matrix.c"))
            .arg(vendor_dir().join("mypaint-symmetry.c"))
            .arg(vendor_dir().join("mypaint-surface.c"))
            .arg(vendor_dir().join("operationqueue.c"))
            .arg(vendor_dir().join("tilemap.c"))
            .arg(vendor_dir().join("fifo.c"))
            .arg("-lm")
            .arg("-o")
            .arg(&exe)
            .status()
            .expect("cc for eval oracle");
        assert!(status.success(), "eval-oracle build failed");
    }
    exe
}

// ---- opcode stream encoder (shared with reference/eval_oracle.c) ----

enum Op {
    MappingNew(usize),
    SetBase(f32),
    SetN { input: usize, n: usize },
    SetPoint { input: usize, index: usize, x: f32, y: f32 },
    /// One f32 per input; emits one f32 result.
    Calc(Vec<f32>),
    RngNew(i64),
    RngNext(u32),
}

fn encode(ops: &[Op]) -> Vec<u8> {
    let mut v = Vec::new();
    let mut w = |bytes: &[u8]| v.extend_from_slice(bytes);
    for op in ops {
        match op {
            Op::MappingNew(inputs) => {
                w(&[1, *inputs as u8]);
            }
            Op::SetBase(base) => {
                w(&[2]);
                w(&base.to_le_bytes());
            }
            Op::SetN { input, n } => {
                w(&[3, *input as u8, *n as u8]);
            }
            Op::SetPoint { input, index, x, y } => {
                w(&[4, *input as u8, *index as u8]);
                w(&x.to_le_bytes());
                w(&y.to_le_bytes());
            }
            Op::Calc(data) => {
                w(&[5]);
                for d in data {
                    w(&d.to_le_bytes());
                }
            }
            Op::RngNew(seed) => {
                w(&[6]);
                w(&seed.to_le_bytes());
            }
            Op::RngNext(count) => {
                w(&[7]);
                w(&count.to_le_bytes());
            }
        }
    }
    v
}

/// Run the ops through both implementations; panic on any bit difference.
fn assert_exact(ops: &[Op], n_calc_results: usize, n_rng_values: usize) {
    let (cmd_file, out_file) = {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        (
            tmp.join(format!("eval-cmds-{}-{n}.bin", std::process::id())),
            tmp.join(format!("eval-out-{n}.bin")),
        )
    };
    std::fs::write(&cmd_file, encode(ops)).unwrap();
    let status = Command::new(oracle_path())
        .arg(&cmd_file)
        .arg(&out_file)
        .output()
        .expect("run eval oracle");
    assert!(
        status.status.success(),
        "oracle failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let reference = std::fs::read(&out_file).unwrap();

    // Replay through maipointo, emitting results in the same order.
    let mut results: Vec<u8> = Vec::new();
    let mut mapping: Option<Mapping> = None;
    let mut rng: Option<maipointo::rngdouble::RngDouble> = None;
    for op in ops {
        match op {
            Op::MappingNew(inputs) => mapping = Some(Mapping::new(*inputs)),
            Op::SetBase(b) => mapping.as_mut().unwrap().set_base_value(*b),
            Op::SetN { input, n } => mapping.as_mut().unwrap().set_n(*input, *n),
            Op::SetPoint { input, index, x, y } => {
                mapping.as_mut().unwrap().set_point(*input, *index, *x, *y)
            }
            Op::Calc(data) => results.extend_from_slice(
                &mapping.as_ref().unwrap().calculate(data).to_le_bytes(),
            ),
            Op::RngNew(seed) => rng = Some(maipointo::rngdouble::RngDouble::new(*seed)),
            Op::RngNext(count) => {
                for _ in 0..*count {
                    results.extend_from_slice(&rng.as_mut().unwrap().next().to_le_bytes());
                }
            }
        }
    }

    assert_eq!(
        results.len(),
        n_calc_results * 4 + n_rng_values * 8,
        "result count mismatch"
    );
    assert_eq!(
        results.len(),
        reference.len(),
        "oracle produced {} bytes, expected {}",
        reference.len(),
        results.len()
    );
    for i in 0..reference.len() {
        assert_eq!(
            results[i], reference[i],
            "bit difference at byte {i} (calc-result boundary at {})",
            n_calc_results * 4
        );
    }
}

/// Fixed mapping scenarios: constant base, single-curve, multi-curve,
/// degenerate segments, x-before-first-point and x-after-last clamping.
#[test]
fn parity_mapping_fixed() {
    let mut ops = vec![Op::MappingNew(3), Op::SetBase(0.5)];
    // Input 0: three points with a curve; input 1: flat segment; input 2 unused.
    ops.push(Op::SetN { input: 0, n: 4 });
    for (i, (x, y)) in [(0.0, 0.0f32), (0.25, 1.0), (0.5, -1.0), (1.0, 0.5)].iter().enumerate() {
        ops.push(Op::SetPoint { input: 0, index: i, x: *x, y: *y });
    }
    ops.push(Op::SetN { input: 1, n: 2 });
    for (i, (x, y)) in [(0.0, 0.4f32), (1.0, 0.4)].iter().enumerate() {
        ops.push(Op::SetPoint { input: 1, index: i, x: *x, y: *y });
    }
    // Evaluate across and outside the defined ranges.
    for &x0 in &[-1.0f32, 0.0, 0.1, 0.249, 0.25, 0.251, 0.499, 0.5, 0.75, 1.0, 2.0] {
        ops.push(Op::Calc(vec![x0, x0 / 2.0, 0.0]));
    }
    // Degenerate flat segment (y0 == y1) and vertical (x0 == x1).
    ops.push(Op::SetN { input: 2, n: 3 });
    for (i, (x, y)) in [(0.0, 0.1f32), (0.0, 0.2), (1.0, 0.1)].iter().enumerate() {
        ops.push(Op::SetPoint { input: 2, index: i, x: *x, y: *y });
    }
    ops.push(Op::Calc(vec![0.5, 0.5, 0.0]));
    assert_exact(&ops, 12, 0);
}

/// RNG parity: Knuth lagged-Fibonacci with MyPaint's low-quality constants.
/// Multiple seeds; long streams cross the QUALITY-buffer cycle boundary.
#[test]
fn parity_rng_streams() {
    for seed in [0i64, 1, 1000, 999_999, -1, i32::MAX as i64, (1i64 << 40) + 12345] {
        let ops = vec![
            Op::RngNew(seed),
            Op::RngNext(64),  // several cycles + exact sentinel handling
            Op::RngNew(seed), // reseed resets state
            Op::RngNext(64),
            Op::RngNext(3),
        ];
        assert_exact(&ops, 0, 64 * 2 + 3);
    }
}

/// Fuzz: random mappings (points, n, data) and rng ops against the C.
#[test]
fn parity_eval_fuzz() {
    struct Lcg(u64);
    impl Lcg {
        fn next_u32(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
        fn f32_in(&mut self, lo: f32, hi: f32) -> f32 {
            let t = (self.next_u32() & 0xffffff) as f32 / 0x1000000 as f32;
            lo + t * (hi - lo)
        }
    }
    let mut rng = Lcg(0x243F_6A88_85A3_08D3);
    let mut ops = Vec::new();
    let mut calc_count = 0usize;
    let mut rng_count = 0usize;

    ops.push(Op::MappingNew(6));
    ops.push(Op::SetBase(rng.f32_in(-2.0, 2.0)));
    for input in 0..6usize {
        // Some inputs get curves, some stay unused.
        if rng.next_u32() % 3 == 0 {
            continue;
        }
        let n = 2 + (rng.next_u32() % 9) as usize; // 2..=10 points
        ops.push(Op::SetN { input, n });
        let mut x = rng.f32_in(-1.0, 0.0);
        for i in 0..n {
            let y = rng.f32_in(-3.0, 3.0);
            ops.push(Op::SetPoint { input, index: i, x, y });
            x = rng.f32_in(x, x + 2.0);
        }
    }
    for _ in 0..50 {
        let data: Vec<f32> = (0..6).map(|_| rng.f32_in(-2.0, 3.0)).collect();
        ops.push(Op::Calc(data));
        calc_count += 1;
    }
    ops.push(Op::RngNew(1000)); // the brush engine's seed
    ops.push(Op::RngNext(500));
    rng_count += 500;
    ops.push(Op::RngNew(-7));
    ops.push(Op::RngNext(97));
    rng_count += 97;
    assert_exact(&ops, calc_count, rng_count);
}

/// The generated settings tables match upstream ids/order.
#[test]
fn settings_tables_sane() {
    assert_eq!(maipointo::settings::INPUT_INFOS[0].id, "pressure");
    assert_eq!(maipointo::settings::SETTING_INFOS[0].internal_name, "opaque");
    assert_eq!(
        input_index("pressure"),
        Some(0),
        "first input is pressure"
    );
    assert_eq!(setting_index("opaque"), Some(0));
    // Upstream defaults that the brush engine relies on.
    let opaque = &maipointo::settings::SETTING_INFOS[setting_index("opaque").unwrap()];
    assert_eq!(opaque.default.to_bits(), 1.0f32.to_bits());
    assert!(opaque.constant == false);
    let paint = SETTING_INFOS
        .iter()
        .find(|s| s.internal_name == "paint_mode")
        .expect("paint_mode setting");
    // NOTE: this NG version defaults paint_mode to 1.0 (full pigment
    // mixing) — which raises the priority of the deferred spectral port.
    assert_eq!(paint.default.to_bits(), 1.0f32.to_bits());
}
