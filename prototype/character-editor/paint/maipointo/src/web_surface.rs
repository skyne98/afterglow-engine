//! The demo's paint surface -- port of `web-surface.c` + the tiled-surface
//! machinery it embeds (`begin_atomic`/`end_atomic` dirty rectangles,
//! per-dab bounding boxes, the symmetry passes of `draw_dab`, and the fixed
//! op queue). Sparse tiles in an open-addressed hash; first-write capture
//! feeds the history system; fixed-capacity storage with deterministic
//! overflow.

use crate::compositor::BlendMode;
use crate::surface::{DrawDabOp, NULL_DAB_OP, Surface};
use crate::mask::render_dab_mask;
use crate::symmetry::{
    rectangle_expand_to_include_point, update_symmetry_state, Rectangle, SymmetryData,
};

const TILE: usize = 64;
const TILE_PX: usize = TILE * TILE * 4;
const MASK_LEN: usize = TILE * TILE + 2 * TILE;
const WEB_SURFACE_MIN_HASH_SIZE: usize = 8192;
const MAX_DIRTY_RECTS: usize = 32;
const OP_QUEUE_CAP: usize = 16384;

/// One dirty-tile position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TilePos {
    pub tx: i32,
    pub ty: i32,
}

/// One queued dab targeted at a tile.
#[derive(Clone, Copy)]
struct QueuedOp {
    tx: i32,
    ty: i32,
    bbox_idx: usize,
    op: DrawDabOp,
}

/// The demo's paint surface: sparse tiles, used-tile tracking, display
/// dirty slots, first-write capture, fixed-capacity op queue.
pub struct WebSurface {
    width: i32,
    height: i32,
    tiles_width: i32,
    tiles_height: i32,
    hash_size: usize,

    slot_used: Vec<bool>,
    slot_tx: Vec<i32>,
    slot_ty: Vec<i32>,
    slot_index: Vec<usize>, // slot -> used-tile index
    tiles: Vec<Option<Box<[u16; TILE_PX]>>>,
    tile_tx: Vec<i32>,
    tile_ty: Vec<i32>,

    display_dirty: Vec<bool>,
    display_dirty_slots: Vec<usize>,

    null_tile: Vec<u16>,
    mask: Vec<u16>,
    scratch: Vec<f32>,

    ops: Vec<QueuedOp>,
    op_len: usize,
    op_failed: u32,

    bboxes: [Rectangle; MAX_DIRTY_RECTS],
    num_bboxes_dirtied: usize,
    atomic_active: bool,

    pub symmetry: SymmetryData,

    // First-write capture (the C write_callback + FixedTileSet path).
    capture_enabled: bool,
    capture_generation: u32,
    capture_marks: Vec<u32>,
    pub captured: Vec<(i32, i32, Vec<u16>)>,
    capture_overflow: bool,

    capacity_failed: bool,
    visible: bool,
}

#[inline]
fn tile_hash(x: i32, y: i32) -> u32 {
    let mut value = (x as u32).wrapping_mul(0x9E37_79B1);
    value ^= (y as u32).wrapping_mul(0x85EB_CA77);
    value ^= value >> 16;
    value = value.wrapping_mul(0xC2B2_AE3D);
    value ^ (value >> 13)
}

impl WebSurface {
    fn tiles_capacity(&self) -> usize {
        (self.tiles_width * self.tiles_height) as usize
    }

    pub fn new(width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let tiles_width = (width + TILE as i32 - 1) / TILE as i32;
        let tiles_height = (height + TILE as i32 - 1) / TILE as i32;
        let tile_capacity = (tiles_width * tiles_height) as usize;
        let mut hash_size = WEB_SURFACE_MIN_HASH_SIZE;
        while hash_size < tile_capacity * 2 {
            hash_size <<= 1;
        }
        Some(Self {
            width,
            height,
            tiles_width,
            tiles_height,
            hash_size,
            slot_used: vec![false; hash_size],
            slot_tx: vec![0; hash_size],
            slot_ty: vec![0; hash_size],
            slot_index: vec![0; hash_size],
            tiles: Vec::with_capacity(tile_capacity),
            tile_tx: Vec::with_capacity(tile_capacity),
            tile_ty: Vec::with_capacity(tile_capacity),
            display_dirty: vec![false; tile_capacity],
            display_dirty_slots: Vec::with_capacity(tile_capacity),
            null_tile: vec![0; TILE_PX],
            mask: vec![0; MASK_LEN],
            scratch: vec![0.0; MASK_LEN],
            ops: vec![QueuedOp { tx: 0, ty: 0, bbox_idx: 0, op: NULL_DAB_OP }; OP_QUEUE_CAP],
            op_len: 0,
            op_failed: 0,
            bboxes: [Rectangle::default(); MAX_DIRTY_RECTS],
            num_bboxes_dirtied: 0,
            atomic_active: false,
            symmetry: crate::symmetry::default_symmetry_data(),
            capture_enabled: false,
            capture_generation: 0,
            capture_marks: vec![0; tile_capacity],
            captured: Vec::new(),
            capture_overflow: false,
            capacity_failed: false,
            visible: true,
        })
    }

    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }
    pub fn tiles_width(&self) -> i32 {
        self.tiles_width
    }
    pub fn tiles_height(&self) -> i32 {
        self.tiles_height
    }
    pub fn used_tile_count(&self) -> usize {
        self.tiles.len()
    }
    pub fn is_visible(&self) -> bool {
        self.visible
    }
    pub fn set_visible(&mut self, v: bool) {
        self.visible = v;
    }
    pub fn queue_failed(&self) -> u32 {
        self.op_failed
    }
    pub fn take_capacity_error(&mut self) -> bool {
        let f = self.capacity_failed;
        self.capacity_failed = false;
        f
    }

    #[inline]
    fn find_tile_slot(&self, tx: i32, ty: i32) -> Option<usize> {
        let start = tile_hash(tx, ty) & (self.hash_size as u32 - 1);
        for probe in 0..self.hash_size as u32 {
            let slot = ((start + probe) & (self.hash_size as u32 - 1)) as usize;
            if !self.slot_used[slot] {
                return None;
            }
            if self.slot_tx[slot] == tx && self.slot_ty[slot] == ty {
                return Some(self.slot_index[slot]);
            }
        }
        None
    }

    fn create_tile_slot(&mut self, tx: i32, ty: i32) -> Option<usize> {
        let start = tile_hash(tx, ty) & (self.hash_size as u32 - 1);
        for probe in 0..self.hash_size as u32 {
            let slot = ((start + probe) & (self.hash_size as u32 - 1)) as usize;
            if self.slot_used[slot] {
                if self.slot_tx[slot] == tx && self.slot_ty[slot] == ty {
                    return Some(self.slot_index[slot]);
                }
                continue;
            }
            if self.tiles.len() >= self.tiles_capacity() {
                self.capacity_failed = true;
                return None;
            }
            let index = self.tiles.len();
            self.tiles.push(Some(Box::new([0u16; TILE_PX])));
            self.tile_tx.push(tx);
            self.tile_ty.push(ty);
            self.slot_used[slot] = true;
            self.slot_tx[slot] = tx;
            self.slot_ty[slot] = ty;
            self.slot_index[slot] = index;
            return Some(index);
        }
        self.capacity_failed = true;
        None
    }

    /// `web_surface_get_tile`.
    pub fn get_tile(&self, tx: i32, ty: i32) -> Option<&[u16; TILE_PX]> {
        let slot = self.find_tile_slot(tx, ty)?;
        Some(self.tiles[slot].as_ref()?.as_ref())
    }

    /// `web_surface_get_or_create_tile`.
    pub fn get_or_create_tile_mut(&mut self, tx: i32, ty: i32) -> Option<&mut [u16; TILE_PX]> {
        let slot = match self.find_tile_slot(tx, ty) {
            Some(s) => s,
            None => self.create_tile_slot(tx, ty)?,
        };
        Some(self.tiles[slot].as_mut()?.as_mut())
    }

    /// `web_surface_has_tile`.
    pub fn has_tile(&self, tx: i32, ty: i32) -> bool {
        self.find_tile_slot(tx, ty).is_some()
    }

    /// First-write capture control (the C write_callback + FixedTileSet).
    pub fn set_capture_enabled(&mut self, enabled: bool) {
        if enabled && !self.capture_enabled {
            self.capture_generation = self.capture_generation.wrapping_add(1);
            if self.capture_generation == 0 {
                self.capture_marks.fill(0);
                self.capture_generation = 1;
            }
            self.captured.clear();
        }
        self.capture_enabled = enabled;
    }

    pub fn take_captured(&mut self) -> Vec<(i32, i32, Vec<u16>)> {
        std::mem::take(&mut self.captured)
    }

    /// Snapshot a tile's before-state on first write of the stroke.
    fn capture_first_write(&mut self, slot: usize, tx: i32, ty: i32) {
        if !self.capture_enabled {
            return;
        }
        if self.capture_marks[slot] == self.capture_generation {
            return; // already captured this stroke
        }
        self.capture_marks[slot] = self.capture_generation;
        let bytes = self.tiles[slot].as_ref().unwrap().to_vec();
        self.captured.push((tx, ty, bytes));
    }

    fn mark_display_dirty(&mut self, slot: usize) {
        if self.display_dirty[slot] {
            return;
        }
        self.display_dirty[slot] = true;
        self.display_dirty_slots.push(slot);
    }

    /// `begin_atomic` (updates the symmetry state + clears dirty bboxes).
    pub fn begin_atomic(&mut self) {
        update_symmetry_state(&mut self.symmetry);
        self.num_bboxes_dirtied = 0;
        self.atomic_active = true;
    }

    /// `end_atomic` -- drain the op queue per dirty tile, then merge the
    /// per-dab bounding boxes into a fresh roi (the C distribution).
    pub fn end_atomic(&mut self) -> Vec<Rectangle> {
        let mut dirty: Vec<(i32, i32)> = Vec::new();
        for r in 0..self.op_len {
            let e = self.ops[r];
            if !dirty.contains(&(e.tx, e.ty)) {
                dirty.push((e.tx, e.ty));
            }
        }
        for (tx, ty) in &dirty {
            self.process_tile(*tx, *ty);
        }

        let num_dirty = self.num_bboxes_dirtied;
        let mut roi = vec![Rectangle::default(); MAX_DIRTY_RECTS];
        if num_dirty > 0 {
            let roi_rects = roi.len();
            let bboxes_per_output = 1f32.max(num_dirty as f32 / roi_rects as f32);
            for i in 0..num_dirty {
                let out_index = if num_dirty > roi_rects {
                    ((i as f32 / bboxes_per_output).round() as usize).min(roi_rects - 1)
                } else {
                    i
                };
                rectangle_expand_to_include_point(
                    &mut roi[out_index],
                    self.bboxes[i].x,
                    self.bboxes[i].y,
                );
            }
            roi.truncate(roi_rects.min(num_dirty));
        }
        self.num_bboxes_dirtied = 0;
        self.atomic_active = false;
        roi
    }

    fn process_tile(&mut self, tx: i32, ty: i32) {
        // Drain matching ops in FIFO order; compact non-matching in place.
        let mut batch: Vec<DrawDabOp> = Vec::new();
        let mut w = 0usize;
        for r in 0..self.op_len {
            let e = self.ops[r];
            if e.tx == tx && e.ty == ty {
                batch.push(e.op);
            } else {
                self.ops[w] = e;
                w += 1;
            }
        }
        self.op_len = w;
        if batch.is_empty() {
            return;
        }
        let Some(slot) = self
            .find_tile_slot(tx, ty)
            .or_else(|| self.create_tile_slot(tx, ty))
        else {
            return; // out of capacity: ops land in the (zeroed) null tile
        };
        self.capture_first_write(slot, tx, ty);
        self.mark_display_dirty(slot);

        let tile = &mut self.tiles[slot].as_mut().unwrap()[..];
        for op in &batch {
            crate::surface::process_op(
                tile,
                &mut self.mask[..],
                tx,
                ty,
                op,
                &mut self.scratch[..],
            );
        }
    }

    /// `draw_dab_internal` -- validate + queue the dab for every touched
    /// tile + update the bbox for `bbox_idx`.
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
        bbox_idx: usize,
    ) -> bool {
        let clamp01 = |v: f32| v.clamp(0.0, 1.0);
        let mut op = DrawDabOp {
            x,
            y,
            radius,
            aspect_ratio,
            angle,
            opaque: clamp01(opaque),
            hardness: clamp01(hardness),
            softness: clamp01(softness),
            lock_alpha: clamp01(lock_alpha),
            colorize: clamp01(colorize),
            posterize: clamp01(posterize),
            posterize_num: ((posterize_num * 100.0 + 0.5) as i32).clamp(1, 128) as u16,
            paint,
            normal: 1.0,
            color_r: 0,
            color_g: 0,
            color_b: 0,
            color_a: 0.0,
        };
        if op.radius < 0.1 || op.hardness == 0.0 || op.softness == 1.0 || op.opaque == 0.0 {
            return false;
        }
        let color_r = clamp01(color_r);
        let color_g = clamp01(color_g);
        let color_b = clamp01(color_b);
        let color_a = clamp01(color_a);
        op.color_r = (color_r * (1 << 15) as f32) as u16;
        op.color_g = (color_g * (1 << 15) as f32) as u16;
        op.color_b = (color_b * (1 << 15) as f32) as u16;
        op.color_a = color_a;
        op.normal = 1.0;
        op.normal *= 1.0 - op.lock_alpha;
        op.normal *= 1.0 - op.colorize;
        op.normal *= 1.0 - op.posterize;
        if op.aspect_ratio < 1.0 {
            op.aspect_ratio = 1.0;
        }

        let r_fringe = radius + 1.0;
        let bbox_idx = bbox_idx.min(self.bboxes.len() - 1);
        let bb_x = (x - r_fringe).floor() as i32;
        let bb_y = (y - r_fringe).floor() as i32;
        let bb_w = (x + r_fringe).floor() as i32 - bb_x + 1;
        let bb_h = (y + r_fringe).floor() as i32 - bb_y + 1;
        let bbox = &mut self.bboxes[bbox_idx];
        rectangle_expand_to_include_point(bbox, bb_x, bb_y);
        rectangle_expand_to_include_point(bbox, bb_x + bb_w - 1, bb_y + bb_h - 1);

        let tx1 = ((x - r_fringe).floor() as i32) / TILE as i32;
        let tx2 = ((x + r_fringe).floor() as i32) / TILE as i32;
        let ty1 = ((y - r_fringe).floor() as i32) / TILE as i32;
        let ty2 = ((y + r_fringe).floor() as i32) / TILE as i32;
        for ty in ty1..=ty2 {
            for tx in tx1..=tx2 {
                if self.op_len < OP_QUEUE_CAP {
                    self.ops[self.op_len] = QueuedOp { tx, ty, bbox_idx, op };
                    self.op_len += 1;
                } else {
                    self.op_failed += 1;
                }
            }
        }
        true
    }

    /// `draw_dab` -- the normal pass plus the symmetry passes (bit-faithful
    /// to mypaint-tiled-surface.c, including the Snowflake fall-through).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_dab(
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
        let surface_modified = self.draw_dab_internal(
            x, y, radius, color_r, color_g, color_b, opaque, hardness, softness,
            color_a, aspect_ratio, angle, lock_alpha, colorize, posterize,
            posterize_num, paint, 0,
        );
        let symm = self.symmetry.state_current;
        if !(surface_modified && self.symmetry.active && self.symmetry.num_symmetry_matrices > 0)
        {
            return surface_modified;
        }
        let num_bboxes = self.bboxes.len();
        let rot_angle = 360.0 / symm.num_lines as f32;
        match symm.kind {
            0 => {
                let (xo, yo) = self.symmetry.matrices[0].point(x, y);
                self.draw_dab_internal(
                    xo, yo, radius, color_r, color_g, color_b, opaque, hardness,
                    softness, color_a, aspect_ratio,
                    -2.0 * (90.0 + symm.angle) - angle,
                    lock_alpha, colorize, posterize, posterize_num, paint, 1,
                );
            }
            1 => {
                let (xo, yo) = self.symmetry.matrices[0].point(x, y);
                self.draw_dab_internal(
                    xo, yo, radius, color_r, color_g, color_b, opaque, hardness,
                    softness, color_a, aspect_ratio, -2.0 * symm.angle - angle,
                    lock_alpha, colorize, posterize, posterize_num, paint, 1,
                );
            }
            2 => {
                let (xo, yo) = self.symmetry.matrices[0].point(x, y);
                self.draw_dab_internal(
                    xo, yo, radius, color_r, color_g, color_b, opaque, hardness,
                    softness, color_a, aspect_ratio, -2.0 * symm.angle - angle,
                    lock_alpha, colorize, posterize, posterize_num, paint, 1,
                );
                let (xo, yo) = self.symmetry.matrices[1].point(x, y);
                self.draw_dab_internal(
                    xo, yo, radius, color_r, color_g, color_b, opaque, hardness,
                    softness, color_a, aspect_ratio, angle, lock_alpha, colorize,
                    posterize, posterize_num, paint, 2,
                );
                let (xo, yo) = self.symmetry.matrices[2].point(x, y);
                self.draw_dab_internal(
                    xo, yo, radius, color_r, color_g, color_b, opaque, hardness,
                    softness, color_a, aspect_ratio, -2.0 * symm.angle - angle,
                    lock_alpha, colorize, posterize, posterize_num, paint, 3,
                );
            }
            4 | 3 => {
                let snowflake = symm.kind == 4;
                let num_lines = symm.num_lines;
                if snowflake {
                    let offset = (num_bboxes / 2).min(symm.num_lines as usize);
                    let dabs_per_bbox =
                        1f32.max(symm.num_lines as f32 * 2.0 / num_bboxes as f32);
                    let base_idx = (symm.num_lines - 1) as usize;
                    let base_angle = -2.0 * symm.angle - angle;
                    for dab_count in 0..symm.num_lines as usize {
                        let bbox_idx = offset
                            + ((dab_count as f32 / dabs_per_bbox).round() as usize)
                                .min(num_bboxes - 1);
                        let (xo, yo) =
                            self.symmetry.matrices[base_idx + dab_count].point(x, y);
                        self.draw_dab_internal(
                            xo, yo, radius, color_r, color_g, color_b, opaque,
                            hardness, softness, color_a, aspect_ratio,
                            base_angle - dab_count as f32 * rot_angle, lock_alpha,
                            colorize, posterize, posterize_num, paint, bbox_idx,
                        );
                    }
                }
                let dabs_per_bbox = 1f32.max(
                    (symm.num_lines * if snowflake { 2 } else { 1 }) as f32
                        / num_bboxes as f32,
                );
                for dab_count in 1..symm.num_lines as usize {
                    let bbox_index = ((dab_count as f32 / dabs_per_bbox).round() as usize)
                        .min(num_bboxes - 1);
                    let (xo, yo) =
                        self.symmetry.matrices[dab_count - 1].point(x, y);
                    self.draw_dab_internal(
                        xo, yo, radius, color_r, color_g, color_b, opaque, hardness,
                        softness, color_a, aspect_ratio,
                        angle - dab_count as f32 * rot_angle, lock_alpha, colorize,
                        posterize, posterize_num, paint, bbox_index,
                    );
                }
            }
            _ => {}
        }
        surface_modified
    }

    /// `get_color` (surface vtable). Flushes queued ops per tile, then
    /// samples -- serial order matches the non-OpenMP C build.
    pub fn get_color(&mut self, x: f32, y: f32, radius: f32, paint: f32) -> [f32; 4] {
        let radius = if radius < 1.0 { 1.0 } else { radius };
        let hardness = 0.5f32;
        let softness = 0.5f32;
        let aspect_ratio = 1.0f32;
        let angle = 0.0f32;
        let mut sums = crate::brushmodes::ColorSums::default();

        let r_fringe = radius + 1.0;
        let tx1 = ((x - r_fringe).floor() as i32) / TILE as i32;
        let tx2 = ((x + r_fringe).floor() as i32) / TILE as i32;
        let ty1 = ((y - r_fringe).floor() as i32) / TILE as i32;
        let ty2 = ((y + r_fringe).floor() as i32) / TILE as i32;
        let sample_interval: u16 = if radius <= 2.0 { 1 } else { (radius * 7.0) as u16 };
        let random_sample_rate = 1.0 / (7.0 * radius);

        for ty in ty1..=ty2 {
            for tx in tx1..=tx2 {
                self.process_tile(tx, ty);
                // Copy the tile out: the mask/scratch borrow self mutably.
                let tile_copy: Vec<u16> = self.get_tile(tx, ty).map(|t| t.to_vec())
                    .unwrap_or_default();
                let rgba: &[u16] = &tile_copy[..];
                render_dab_mask(
                    &mut self.mask[..],
                    x - (tx * TILE as i32) as f32,
                    y - (ty * TILE as i32) as f32,
                    radius,
                    hardness,
                    softness,
                    aspect_ratio,
                    angle,
                    &mut self.scratch[..],
                );
                crate::brushmodes::get_color_accumulate(
                    &self.mask[..],
                    rgba,
                    &mut sums,
                    paint,
                    sample_interval,
                    random_sample_rate,
                    &mut crate::random::PortableRand::default(),
                );
            }
        }
        let weight = sums.weight.max(f32::MIN_POSITIVE);
        [
            sums.r / weight,
            sums.g / weight,
            sums.b / weight,
            sums.a / weight,
        ]
    }

    pub fn display_dirty_count(&self) -> usize {
        self.display_dirty_slots.len()
    }

    pub fn display_dirty_info(&self, index: usize) -> Option<TilePos> {
        let slot = *self.display_dirty_slots.get(index)?;
        Some(TilePos { tx: self.tile_tx[slot], ty: self.tile_ty[slot] })
    }

    pub fn clear_display_dirty(&mut self) {
        for &slot in &self.display_dirty_slots {
            self.display_dirty[slot] = false;
        }
        self.display_dirty_slots.clear();
    }

    /// `web_surface_clear`.
    pub fn clear_tiles(&mut self) {
        self.tiles.clear();
        self.tile_tx.clear();
        self.tile_ty.clear();
        self.slot_used.fill(false);
        self.display_dirty.fill(false);
        self.display_dirty_slots.clear();
        self.null_tile.fill(0);
    }

    /// `web_surface_set_symmetry`.
    pub fn set_symmetry(&mut self, active: bool, center_x: f32, center_y: f32,
        angle: f32, symmetry_type: i32, lines: i32,
    ) {
        crate::symmetry::symmetry_set_pending(
            &mut self.symmetry, active, center_x, center_y, angle,
            symmetry_type, lines,
        );
    }

    pub fn used_tile_info(&self, index: usize) -> Option<TilePos> {
        Some(TilePos { tx: *self.tile_tx.get(index)?, ty: *self.tile_ty.get(index)? })
    }

    pub fn used_tile(&self, index: usize) -> Option<&[u16; TILE_PX]> {
        self.tiles.get(index).and_then(|t| t.as_ref()).map(|b| &**b)
    }
}

impl Surface for WebSurface {
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
    ) -> bool {
        self.draw_dab(
            x, y, radius, color_r, color_g, color_b, opaque, hardness,
            softness, color_a, aspect_ratio, angle, lock_alpha, colorize,
            posterize, posterize_num, paint,
        )
    }

    fn get_color(&mut self, x: f32, y: f32, radius: f32, paint: f32) -> [f32; 4] {
        self.get_color(x, y, radius, paint)
    }
}
