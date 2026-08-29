//! Port of `mypaint-tiled-surface.c` (`draw_dab_internal`, `get_color`,
//! `process_op`) plus `mypaint-fixed-tiled-surface.c` (fixed in-memory tile
//! buffer with a discarded "null tile" for out-of-range requests).
//! Symmetry is inactive (matching the default C state).

use crate::brushmodes::*;
use crate::helpers::clamp;
use crate::mask::{render_dab_mask, TILE_SIZE};
use crate::random::RandomSource;

/// A queued dab operation (`OperationDataDrawDab`).
#[derive(Clone, Copy, Debug)]
pub struct DrawDabOp {
    x: f32,
    y: f32,
    radius: f32,
    aspect_ratio: f32,
    angle: f32,
    opaque: f32,
    hardness: f32,
    softness: f32,
    lock_alpha: f32,
    colorize: f32,
    posterize: f32,
    posterize_num: u16,
    paint: f32,
    normal: f32,
    color_r: u16,
    color_g: u16,
    color_b: u16,
    color_a: f32,
}

/// C `ROUND(x)` = `(int)((x)+0.5)`.
#[inline]
fn round_i(x: f32) -> i32 {
    (x + 0.5) as i32
}

/// `MyPaintFixedTiledSurface`: W×H pixels, 64×64 RGBA fix15 tiles, the tile
/// buffer pre-filled with `0xFFFF` u16s (the C `memset(buffer, 255)` quirk).
pub struct FixedTiledSurface {
    width: i32,
    height: i32,
    tiles_width: i32,
    tiles_height: i32,
    tiles: Vec<u16>,
    ops: Vec<(i32, i32, DrawDabOp)>,
    mask: Vec<u16>,
    scratch: Vec<f32>,
    random: Box<dyn RandomSource>,
}

impl FixedTiledSurface {
    pub fn new(width: i32, height: i32) -> Self {
        assert!(width > 0 && height > 0);
        let tiles_width = ((width as f32) / TILE_SIZE as f32).ceil() as i32;
        let tiles_height = ((height as f32) / TILE_SIZE as f32).ceil() as i32;
        let words = (tiles_width * tiles_height) as usize * TILE_SIZE * TILE_SIZE * 4;
        Self {
            width,
            height,
            tiles_width,
            tiles_height,
            tiles: vec![0xFFFF; words],
            ops: Vec::new(),
            mask: Vec::new(),
            scratch: Vec::new(),
            random: Box::new(crate::random::PortableRand::default()),
        }
    }

    /// Construct with an explicit random source (the parity oracle injects
    /// `GlibcRand` to match the native C reference).
    pub fn new_with_random(width: i32, height: i32, random: Box<dyn RandomSource>) -> Self {
        let mut surface = Self::new(width, height);
        surface.random = random;
        surface
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    #[inline]
    fn tile_base(&self, tx: i32, ty: i32) -> Option<usize> {
        if tx < 0 || ty < 0 || tx >= self.tiles_width || ty >= self.tiles_height {
            return None;
        }
        Some((ty * self.tiles_width + tx) as usize * TILE_SIZE * TILE_SIZE * 4)
    }

    /// `process_tile` — apply all queued ops for one tile, FIFO order.
    fn process_tile(&mut self, tx: i32, ty: i32) {
        let mut batch = Vec::new();
        self.ops.retain(|(ttx, tty, op)| {
            if *ttx == tx && *tty == ty {
                batch.push(*op);
                false
            } else {
                true
            }
        });
        if batch.is_empty() {
            return;
        }

        let mut mask = std::mem::take(&mut self.mask);
        let mut scratch = std::mem::take(&mut self.scratch);
        if let Some(base) = self.tile_base(tx, ty) {
            let end = base + TILE_SIZE * TILE_SIZE * 4;
            let (before, rgba) = self.tiles.split_at_mut(end);
            let rgba = &mut rgba[..TILE_SIZE * TILE_SIZE * 4];
            for op in &batch {
                process_op(rgba, &mut mask, tx, ty, op, &mut scratch);
            }
            let _ = before;
        }
        // Out-of-range tiles drew into the discarded null tile.
        self.mask = mask;
        self.scratch = scratch;
    }

    /// `draw_dab_internal` — validate + queue the dab onto every tile it
    /// touches. Returns whether the surface was modified.
    #[allow(clippy::too_many_arguments)]
    fn draw_dab_internal(
        &mut self,
        x: f32,
        y: f32,
        radius: f32,
        color_r: f32,
        color_g: f32,
        color_b: f32,
        opaque: f32,
        hardness: f32,
        softness: f32,
        color_a: f32,
        aspect_ratio: f32,
        angle: f32,
        lock_alpha: f32,
        colorize: f32,
        posterize: f32,
        posterize_num: f32,
        paint: f32,
    ) -> bool {
        let mut op = DrawDabOp {
            x,
            y,
            radius,
            aspect_ratio,
            angle,
            opaque: clamp(opaque, 0.0, 1.0),
            hardness: clamp(hardness, 0.0, 1.0),
            softness: clamp(softness, 0.0, 1.0),
            lock_alpha: clamp(lock_alpha, 0.0, 1.0),
            colorize: clamp(colorize, 0.0, 1.0),
            posterize: clamp(posterize, 0.0, 1.0),
            posterize_num: clamp(round_i(posterize_num * 100.0) as f32, 1.0, 128.0) as u16,
            paint: clamp(paint, 0.0, 1.0),
            normal: 1.0,
            color_r: 0,
            color_g: 0,
            color_b: 0,
            color_a: 0.0,
        };

        if op.radius < 0.1 {
            return false; // don't bother with dabs smaller than 0.1 pixel
        }
        if op.hardness == 0.0 {
            return false; // infinitely small center point, transparent outside
        }
        if op.softness == 1.0 {
            return false;
        }
        if op.opaque == 0.0 {
            return false;
        }

        let color_r = clamp(color_r, 0.0, 1.0);
        let color_g = clamp(color_g, 0.0, 1.0);
        let color_b = clamp(color_b, 0.0, 1.0);
        let color_a = clamp(color_a, 0.0, 1.0);

        op.color_r = (color_r * (1 << 15) as f32) as u16;
        op.color_g = (color_g * (1 << 15) as f32) as u16;
        op.color_b = (color_b * (1 << 15) as f32) as u16;
        op.color_a = color_a;

        // blending mode preparation
        op.normal = 1.0;
        op.normal *= 1.0 - op.lock_alpha;
        op.normal *= 1.0 - op.colorize;
        op.normal *= 1.0 - op.posterize;

        if op.aspect_ratio < 1.0 {
            op.aspect_ratio = 1.0;
        }

        let r_fringe = radius + 1.0;

        let tx1 = ((x - r_fringe).floor() as i32) / TILE_SIZE as i32;
        let tx2 = ((x + r_fringe).floor() as i32) / TILE_SIZE as i32;
        let ty1 = ((y - r_fringe).floor() as i32) / TILE_SIZE as i32;
        let ty2 = ((y + r_fringe).floor() as i32) / TILE_SIZE as i32;

        for ty in ty1..=ty2 {
            for tx in tx1..=tx2 {
                self.ops.push((tx, ty, op));
            }
        }
        true
    }

    /// `get_color` (surface vtable). Serial tile order matches the
    /// non-OpenMP C build; the glibc rand() stream continues across calls.
    fn get_color_internal(&mut self, x: f32, y: f32, radius: f32, paint: f32) -> [f32; 4] {
        let radius = if radius < 1.0 { 1.0 } else { radius };
        let hardness = 0.5f32;
        let softness = 0.5f32;
        let aspect_ratio = 1.0f32;
        let angle = 0.0f32;

        let mut sums = ColorSums::default();

        let r_fringe = radius + 1.0;

        let tx1 = ((x - r_fringe).floor() as i32) / TILE_SIZE as i32;
        let tx2 = ((x + r_fringe).floor() as i32) / TILE_SIZE as i32;
        let ty1 = ((y - r_fringe).floor() as i32) / TILE_SIZE as i32;
        let ty2 = ((y + r_fringe).floor() as i32) / TILE_SIZE as i32;

        let sample_interval: u16 = if radius <= 2.0 { 1 } else { (radius * 7.0) as u16 };
        let random_sample_rate = 1.0 / (7.0 * radius);

        for ty in ty1..=ty2 {
            for tx in tx1..=tx2 {
                self.process_tile(tx, ty);

                static NULL_TILE: [u16; TILE_SIZE * TILE_SIZE * 4] =
                    [0; TILE_SIZE * TILE_SIZE * 4];
                let rgba: &[u16] = match self.tile_base(tx, ty) {
                    Some(base) => {
                        let end = base + TILE_SIZE * TILE_SIZE * 4;
                        &self.tiles[base..end]
                    }
                    None => &NULL_TILE,
                };

                render_dab_mask(
                    &mut self.mask,
                    x - (tx * TILE_SIZE as i32) as f32,
                    y - (ty * TILE_SIZE as i32) as f32,
                    radius,
                    hardness,
                    softness,
                    aspect_ratio,
                    angle,
                    &mut self.scratch,
                );

                get_color_accumulate(
                    &self.mask,
                    rgba,
                    &mut sums,
                    paint,
                    sample_interval,
                    random_sample_rate,
                    self.random.as_mut(),
                );
            }
        }

        debug_assert!(sums.weight > 0.0);
        let sum_a = sums.a / sums.weight;

        let mut sum_r = sums.r;
        let mut sum_g = sums.g;
        let mut sum_b = sums.b;
        if paint < 0.0 {
            sum_r /= sums.weight;
            sum_g /= sums.weight;
            sum_b /= sums.weight;
        }

        let mut out = [0.0f32; 4];
        out[3] = clamp(sum_a, 0.0, 1.0);
        if sum_a > 0.0 {
            let demul = if paint < 0.0 { sum_a } else { 1.0 };
            out[0] = clamp(sum_r / demul, 0.0, 1.0);
            out[1] = clamp(sum_g / demul, 0.0, 1.0);
            out[2] = clamp(sum_b / demul, 0.0, 1.0);
        } else {
            out[0] = 0.0;
            out[1] = 1.0;
            out[2] = 0.0;
        }
        out
    }

    /// Surface-level `draw_dab` (symmetry pass skipped: inactive by default).
    #[allow(clippy::too_many_arguments)]
    pub fn surface_draw_dab(
        &mut self,
        x: f32,
        y: f32,
        radius: f32,
        color_r: f32,
        color_g: f32,
        color_b: f32,
        opaque: f32,
        hardness: f32,
        softness: f32,
        color_a: f32,
        aspect_ratio: f32,
        angle: f32,
        lock_alpha: f32,
        colorize: f32,
        posterize: f32,
        posterize_num: f32,
        paint: f32,
    ) -> bool {
        self.draw_dab_internal(
            x, y, radius, color_r, color_g, color_b, opaque, hardness, softness, color_a,
            aspect_ratio, angle, lock_alpha, colorize, posterize, posterize_num, paint,
        )
    }

    /// Surface-level `get_color`.
    pub fn surface_get_color(&mut self, x: f32, y: f32, radius: f32, paint: f32) -> [f32; 4] {
        self.get_color_internal(x, y, radius, paint)
    }

    /// Drain every queued operation into its tile (end-of-stroke flush).
    pub fn flush_all(&mut self) {
        let mut coords: Vec<(i32, i32)> = Vec::new();
        for (tx, ty, _) in &self.ops {
            if !coords.contains(&(*tx, *ty)) {
                coords.push((*tx, *ty));
            }
        }
        for (tx, ty) in coords {
            self.process_tile(tx, ty);
        }
    }

    /// Raw tile dump (the parity artifact).
    pub fn tile_bytes(&self) -> &[u16] {
        &self.tiles
    }
}

/// `process_op` — render the mask then stamp the dab with each active blend
/// mode (verbatim from `mypaint-tiled-surface.c`).
fn process_op(
    rgba: &mut [u16],
    mask: &mut Vec<u16>,
    tx: i32,
    ty: i32,
    op: &DrawDabOp,
    scratch: &mut Vec<f32>,
) {
    render_dab_mask(
        mask,
        op.x - (tx * TILE_SIZE as i32) as f32,
        op.y - (ty * TILE_SIZE as i32) as f32,
        op.radius,
        op.hardness,
        op.softness,
        op.aspect_ratio,
        op.angle,
        scratch,
    );

    let m = &mask[..];
    if op.paint < 1.0 {
        if op.normal != 0.0 {
            if op.color_a == 1.0 {
                draw_dab_normal(
                    m,
                    rgba,
                    op.color_r,
                    op.color_g,
                    op.color_b,
                    (op.normal * op.opaque * (1.0 - op.paint) * (1 << 15) as f32) as u16,
                );
            } else {
                // normal case for brushes that use smudging (eg. watercolor)
                draw_dab_normal_and_eraser(
                    m,
                    rgba,
                    op.color_r,
                    op.color_g,
                    op.color_b,
                    (op.color_a * (1 << 15) as f32) as u16,
                    (op.normal * op.opaque * (1.0 - op.paint) * (1 << 15) as f32) as u16,
                );
            }
        }

        if op.lock_alpha != 0.0 && op.color_a != 0.0 {
            draw_dab_lock_alpha(
                m,
                rgba,
                op.color_r,
                op.color_g,
                op.color_b,
                (op.lock_alpha
                    * op.opaque
                    * (1.0 - op.colorize)
                    * (1.0 - op.posterize)
                    * (1.0 - op.paint)
                    * (1 << 15) as f32) as u16,
            );
        }
    } else {
        // spectral paint path (paint >= 1.0): the NG Pigment mode
        if op.normal != 0.0 {
            if op.color_a == 1.0 {
                draw_dab_normal_paint(
                    m,
                    rgba,
                    op.color_r,
                    op.color_g,
                    op.color_b,
                    (op.normal * op.opaque * op.paint * (1 << 15) as f32) as u16,
                );
            } else {
                draw_dab_normal_and_eraser_paint(
                    m,
                    rgba,
                    op.color_r,
                    op.color_g,
                    op.color_b,
                    (op.color_a * (1 << 15) as f32) as u16,
                    (op.normal * op.opaque * op.paint * (1 << 15) as f32) as u16,
                );
            }
        }

        if op.lock_alpha != 0.0 && op.color_a != 0.0 {
            draw_dab_lock_alpha_paint(
                m,
                rgba,
                op.color_r,
                op.color_g,
                op.color_b,
                (op.lock_alpha
                    * op.opaque
                    * (1.0 - op.colorize)
                    * (1.0 - op.posterize)
                    * op.paint
                    * (1 << 15) as f32) as u16,
            );
        }
    }

    if op.colorize != 0.0 {
        draw_dab_colorize(
            m,
            rgba,
            op.color_r,
            op.color_g,
            op.color_b,
            (op.colorize * op.opaque * (1 << 15) as f32) as u16,
        );
    }
    if op.posterize != 0.0 {
        draw_dab_posterize(
            m,
            rgba,
            (op.posterize * op.opaque * (1 << 15) as f32) as u16,
            op.posterize_num,
        );
    }
}

/// The `MyPaintSurface` vtable subset the brush engine calls.
pub trait Surface {
    /// `mypaint_surface_draw_dab` — returns whether the surface changed.
    #[allow(clippy::too_many_arguments)]
    fn surface_draw_dab(
        &mut self,
        x: f32,
        y: f32,
        radius: f32,
        color_r: f32,
        color_g: f32,
        color_b: f32,
        opaque: f32,
        hardness: f32,
        softness: f32,
        color_a: f32,
        aspect_ratio: f32,
        angle: f32,
        lock_alpha: f32,
        colorize: f32,
        posterize: f32,
        posterize_num: f32,
        paint: f32,
    ) -> bool;

    /// `mypaint_surface_get_color` — returns [r, g, b, a].
    fn get_color(&mut self, x: f32, y: f32, radius: f32, paint: f32) -> [f32; 4];
}

impl Surface for FixedTiledSurface {
    fn surface_draw_dab(
        &mut self,
        x: f32,
        y: f32,
        radius: f32,
        color_r: f32,
        color_g: f32,
        color_b: f32,
        opaque: f32,
        hardness: f32,
        softness: f32,
        color_a: f32,
        aspect_ratio: f32,
        angle: f32,
        lock_alpha: f32,
        colorize: f32,
        posterize: f32,
        posterize_num: f32,
        paint: f32,
    ) -> bool {
        self.draw_dab_internal(
            x, y, radius, color_r, color_g, color_b, opaque, hardness, softness, color_a,
            aspect_ratio, angle, lock_alpha, colorize, posterize, posterize_num, paint,
        )
    }

    fn get_color(&mut self, x: f32, y: f32, radius: f32, paint: f32) -> [f32; 4] {
        self.get_color_internal(x, y, radius, paint)
    }
}
