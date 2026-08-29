//! Bit-exact parity tests against the vendored libmypaint C code.
//!
//! The test builds `reference/oracle.c` with `cc` (no FMA contraction), feeds
//! identical binary dab commands to the oracle and to maipointo, and requires
//! the 64×64 fix15 tile bytes — and the `get_color` float sums — to compare
//! exactly, bit for bit.

use std::path::PathBuf;
use std::process::Command;

use maipointo::brushmodes::*;
use maipointo::mask::{render_dab_mask, TILE_SIZE};
use maipointo::Tile;

/// One 64-byte dab command record, shared with reference/oracle.c.
#[derive(Clone, Copy, Debug)]
struct DabCmd {
    mode: u8,
    x: f32,
    y: f32,
    radius: f32,
    hardness: f32,
    softness: f32,
    aspect: f32,
    angle: f32,
    r: u16,
    g: u16,
    b: u16,
    a: u16,
    opacity: u16,
    posterize_num: u16,
    paint: f32,
    interval: u16,
    rand_rate: f32,
    seed: u32,
}

const MODE_NORMAL: u8 = 0;
const MODE_NORMAL_ERASER: u8 = 1;
const MODE_LOCK_ALPHA: u8 = 2;
const MODE_POSTERIZE: u8 = 3;
const MODE_COLORIZE: u8 = 4;
const MODE_GET_COLOR_LEGACY: u8 = 5;
const MODE_GET_COLOR_ACCUM: u8 = 6;

impl DabCmd {
    fn encode(&self) -> [u8; 64] {
        let mut v = [0u8; 64];
        v[0] = self.mode;
        v[4..8].copy_from_slice(&self.x.to_le_bytes());
        v[8..12].copy_from_slice(&self.y.to_le_bytes());
        v[12..16].copy_from_slice(&self.radius.to_le_bytes());
        v[16..20].copy_from_slice(&self.hardness.to_le_bytes());
        v[20..24].copy_from_slice(&self.softness.to_le_bytes());
        v[24..28].copy_from_slice(&self.aspect.to_le_bytes());
        v[28..32].copy_from_slice(&self.angle.to_le_bytes());
        v[32..34].copy_from_slice(&self.r.to_le_bytes());
        v[34..36].copy_from_slice(&self.g.to_le_bytes());
        v[36..38].copy_from_slice(&self.b.to_le_bytes());
        v[38..40].copy_from_slice(&self.a.to_le_bytes());
        v[40..42].copy_from_slice(&self.opacity.to_le_bytes());
        v[42..44].copy_from_slice(&self.posterize_num.to_le_bytes());
        v[48..52].copy_from_slice(&self.paint.to_le_bytes());
        v[52..54].copy_from_slice(&self.interval.to_le_bytes());
        v[56..60].copy_from_slice(&self.rand_rate.to_le_bytes());
        v[60..64].copy_from_slice(&self.seed.to_le_bytes());
        v
    }
}

/// Tiny deterministic LCG so the fuzz is reproducible.
struct Lcg(u64);

impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn f32_in(&mut self, lo: f32, hi: f32) -> f32 {
        let t = (self.next_u32() & 0xffffff) as f32 / 0x1000000 as f32;
        lo + t * (hi - lo)
    }
    fn u16_in(&mut self, lo: u16, hi: u16) -> u16 {
        lo + (self.next_u32() % ((hi - lo) as u32 + 1)) as u16
    }
}

fn paint_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn vendor_dir() -> PathBuf {
    paint_dir().join("vendor").join("libmypaint")
}

/// Build (or reuse) the C oracle.
fn oracle_path() -> PathBuf {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let exe = tmp.join("maipointo-oracle");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("reference").join("oracle.c");
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
        let cc = Command::new("cc")
            .arg("-O2")
            .arg("-ffp-contract=off")
            .arg("-std=c11")
            .arg("-I")
            .arg(paint_dir())
            .arg("-I")
            .arg(vendor_dir())
            .arg(&src)
            .arg(vendor_dir().join("brushmodes.c"))
            .arg(vendor_dir().join("mypaint-tiled-surface.c"))
            .arg(vendor_dir().join("helpers.c"))
            .arg(vendor_dir().join("mypaint.c"))
            .arg(vendor_dir().join("mypaint-mapping.c"))
            .arg(vendor_dir().join("mypaint-brush-settings.c"))
            .arg(vendor_dir().join("rng-double.c"))
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
            .expect("failed to run cc for the parity oracle");
        assert!(cc.success(), "oracle build failed");
    }
    exe
}

/// Run one side (oracle or maipointo) over the commands and return the
/// 64×64×4 u16 tile bytes plus five f64 sums.
/// Unique temp files per call — cargo runs tests in parallel.
fn scratch_paths(tag: &str) -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    (
        tmp.join(format!("parity-cmds-{tag}-{}-{n}.bin", std::process::id())),
        tmp.join(format!("parity-out-{tag}-{n}.bin", )),
    )
}

fn run_oracle(cmds: &[DabCmd]) -> (Vec<u16>, [f64; 5]) {
    let (cmd_file, out_file) = scratch_paths("o");
    let mut bytes = Vec::with_capacity(cmds.len() * 64);
    for c in cmds {
        bytes.extend_from_slice(&c.encode());
    }
    std::fs::write(&cmd_file, &bytes).unwrap();
    let status = Command::new(oracle_path())
        .arg(&cmd_file)
        .arg(&out_file)
        .output()
        .expect("failed to run the parity oracle");
    assert!(
        status.status.success(),
        "oracle failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let raw = std::fs::read(&out_file).unwrap();
    let tile_words = TILE_SIZE * TILE_SIZE * 4;
    let mut tile = Vec::with_capacity(tile_words);
    for i in 0..tile_words {
        tile.push(u16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]));
    }
    let sums_raw: [u8; 40] = raw[tile_words * 2..tile_words * 2 + 40]
        .try_into()
        .unwrap();
    let mut sums = [0f64; 5];
    for (i, s) in sums.iter_mut().enumerate() {
        *s = f64::from_le_bytes(sums_raw[i * 8..i * 8 + 8].try_into().unwrap());
    }
    (tile, sums)
}

/// Apply the same command list through maipointo.
fn run_maipointo(cmds: &[DabCmd]) -> (Vec<u16>, [f64; 5]) {
    let mut tile = Tile::new();
    let mut scratch = Vec::new();
    let mut mask = Vec::new();
    let mut sums = ColorSums::default();
    for c in cmds {
        render_dab_mask(
            &mut mask,
            c.x,
            c.y,
            c.radius,
            c.hardness,
            c.softness,
            c.aspect,
            c.angle,
            &mut scratch,
        );
        match c.mode {
            MODE_NORMAL => draw_dab_normal(&mask, &mut tile.data, c.r, c.g, c.b, c.opacity),
            MODE_NORMAL_ERASER => draw_dab_normal_and_eraser(
                &mask, &mut tile.data, c.r, c.g, c.b, c.a, c.opacity,
            ),
            MODE_LOCK_ALPHA => draw_dab_lock_alpha(&mask, &mut tile.data, c.r, c.g, c.b, c.opacity),
            MODE_POSTERIZE => {
                draw_dab_posterize(&mask, &mut tile.data, c.opacity, c.posterize_num)
            }
            MODE_COLORIZE => draw_dab_colorize(&mask, &mut tile.data, c.r, c.g, c.b, c.opacity),
            MODE_GET_COLOR_LEGACY => get_color_legacy(&mask, &tile.data, &mut sums),
            MODE_GET_COLOR_ACCUM => get_color_accumulate(
                &mask,
                &tile.data,
                &mut sums,
                c.paint,
                c.interval,
                c.rand_rate,
                || 0, // unused at rate 0 / interval 1
                2147483647,
            ),
            _ => unreachable!(),
        }
    }
    (
        tile.data.clone(),
        [sums.weight as f64, sums.r as f64, sums.g as f64, sums.b as f64, sums.a as f64],
    )
}

fn assert_exact(cmds: &[DabCmd]) {
    let (want_tile, want_sums) = run_oracle(cmds);
    let (got_tile, got_sums) = run_maipointo(cmds);
    assert_eq!(got_tile.len(), want_tile.len());
    let mut first_diff: Option<(usize, u16, u16)> = None;
    for (i, (w, g)) in want_tile.iter().zip(got_tile.iter()).enumerate() {
        if w != g && first_diff.is_none() {
            first_diff = Some((i, *g, *w));
        }
    }
    assert!(
        first_diff.is_none(),
        "tile mismatch at u16 index {:?} (got, want); cmds={}",
        first_diff,
        cmds.len()
    );
    for i in 0..5 {
        // Oracle stores f64(float_sum); bit-compare the f32 values.
        let w = want_sums[i] as f32;
        let g = got_sums[i] as f32;
        assert_eq!(g.to_bits(), w.to_bits(), "sums[{i}] differs");
    }
}

/// Every blend mode over a fixed, hand-chosen scenario set that hits all the
/// tricky paths: radius < 3 (AA path), hardness == 1.0, softness > 0, dab
/// centers off-tile, extremes of colors.
fn fixed_scenarios() -> Vec<DabCmd> {
    let mut cmds = Vec::new();
    for &radius in &[0.5f32, 1.0, 2.5, 2.99, 3.0, 3.01, 8.0, 40.0] {
        for &hardness in &[0.01f32, 0.5, 0.999, 1.0] {
            cmds.push(DabCmd {
                mode: MODE_NORMAL,
                x: 32.0,
                y: 32.0,
                radius,
                hardness,
                softness: 0.0,
                aspect: 1.0,
                angle: 0.0,
                r: 32768,
                g: 16384,
                b: 8192,
                a: 32768,
                opacity: 32768,
                posterize_num: 2,
                paint: -1.0,
                interval: 1,
                rand_rate: 0.0,
                seed: 0,
            });
        }
    }
    // Off-tile and corner dabs exercise the clamped bounding box.
    for &(x, y) in &[(0.0f32, 0.0), (63.9, 63.9), (-5.0, 70.0), (32.3, 32.7)] {
        for &aspect in &[1.0f32, 3.5] {
            for &angle in &[0.0f32, 37.5, 270.0] {
                cmds.push(DabCmd {
                    mode: MODE_NORMAL_ERASER,
                    x,
                    y,
                    radius: 6.0,
                    hardness: 0.7,
                    softness: 0.0,
                    aspect,
                    angle,
                    r: 20000,
                    g: 20000,
                    b: 20000,
                    a: 19660, // 0.6 smudge
                    opacity: 24576,
                    posterize_num: 2,
                    paint: -1.0,
                    interval: 1,
                    rand_rate: 0.0,
                    seed: 0,
                });
                cmds.push(DabCmd {
                    mode: MODE_LOCK_ALPHA,
                    x,
                    y,
                    radius: 6.0,
                    hardness: 0.7,
                    softness: 0.0,
                    aspect,
                    angle,
                    r: 16384,
                    g: 32768,
                    b: 4096,
                    a: 32768,
                    opacity: 16384,
                    posterize_num: 2,
                    paint: -1.0,
                    interval: 1,
                    rand_rate: 0.0,
                    seed: 0,
                });
                cmds.push(DabCmd {
                    mode: MODE_COLORIZE,
                    x,
                    y,
                    radius: 6.0,
                    hardness: 0.7,
                    softness: 0.0,
                    aspect,
                    angle,
                    r: 24576,
                    g: 4096,
                    b: 8192,
                    a: 32768,
                    opacity: 32768,
                    posterize_num: 2,
                    paint: -1.0,
                    interval: 1,
                    rand_rate: 0.0,
                    seed: 0,
                });
                cmds.push(DabCmd {
                    mode: MODE_POSTERIZE,
                    x,
                    y,
                    radius: 6.0,
                    hardness: 0.7,
                    softness: 0.0,
                    aspect,
                    angle,
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 32768,
                    opacity: 28000,
                    posterize_num: 7,
                    paint: -1.0,
                    interval: 1,
                    rand_rate: 0.0,
                    seed: 0,
                });
            }
        }
    }
    // get_color on the painted result.
    cmds.push(DabCmd {
        mode: MODE_GET_COLOR_LEGACY,
        x: 32.0,
        y: 32.0,
        radius: 12.0,
        hardness: 0.5,
        softness: 0.0,
        aspect: 1.0,
        angle: 0.0,
        r: 0,
        g: 0,
        b: 0,
        a: 0,
        opacity: 0,
        posterize_num: 1,
        paint: -1.0,
        interval: 1,
        rand_rate: 0.0,
        seed: 0,
    });
    cmds.push(DabCmd {
        mode: MODE_GET_COLOR_ACCUM,
        x: 32.0,
        y: 32.0,
        radius: 12.0,
        hardness: 0.5,
        softness: 0.0,
        aspect: 1.0,
        angle: 0.0,
        r: 0,
        g: 0,
        b: 0,
        a: 0,
        opacity: 0,
        posterize_num: 1,
        paint: 0.0,
        interval: 1,
        rand_rate: 0.0,
        seed: 0,
    });
    cmds
}

#[test]
fn parity_fixed_scenarios() {
    assert_exact(&fixed_scenarios());
}

#[test]
fn bisect_first_divergence() {
    let cmds = fixed_scenarios();
    for (i, c) in cmds.iter().enumerate() {
        let (want, _) = run_oracle(&[c.clone()]);
        let (got, _) = run_maipointo(&[c.clone()]);
        if want != got {
            let count = want.iter().zip(got.iter()).filter(|(w, g)| w != g).count();
            panic!("FIRST DIVERGENCE at cmd {i}: {c:?} ({count} differing words)");
        }
    }
}
/// Seed printed on failure so it can be reproduced.
#[test]
fn parity_fuzzed_dabs() {
    let mut rng = Lcg(0x9E3779B97F4A7C15);
    let mut cmds = Vec::new();
    for i in 0..400 {
        let mode = match i % 7 {
            0 => MODE_NORMAL,
            1 => MODE_NORMAL_ERASER,
            2 => MODE_LOCK_ALPHA,
            3 => MODE_POSTERIZE,
            4 => MODE_COLORIZE,
            5 => MODE_GET_COLOR_LEGACY,
            _ => MODE_GET_COLOR_ACCUM,
        };
        // hardness must be != 0 (caller contract) and aspect >= 1 is enforced
        // inside; radius > 0 required to avoid div by zero.
        let hardness = rng.f32_in(0.02, 1.0);
        let radius = rng.f32_in(0.5, 40.0);
        let posterize = rng.u16_in(2, 16);
        let cmd = DabCmd {
            mode,
            x: rng.f32_in(-10.0, 74.0),
            y: rng.f32_in(-10.0, 74.0),
            radius,
            hardness,
            softness: 0.0,
            aspect: rng.f32_in(1.0, 8.0),
            angle: rng.f32_in(0.0, 360.0),
            r: rng.u16_in(0, 32768),
            g: rng.u16_in(0, 32768),
            b: rng.u16_in(0, 32768),
            a: rng.u16_in(0, 32768),
            opacity: rng.u16_in(0, 32768),
            posterize_num: posterize,
            // paint < 0 → legacy sampling; 0.0 → non-spectral accumulate.
            paint: if mode == MODE_GET_COLOR_ACCUM { 0.0 } else { -1.0 },
            interval: 1,
            rand_rate: 0.0,
            seed: 0,
        };
        cmds.push(cmd);
    }
    assert_exact(&cmds);
}
