//! The demo paint application -- port of `main.c`'s state machine: layers,
//! groups, history, background, display EOTF, mip rendering, and the
//! `_paint_*` surface the TS worker drives. Zero C, zero emscripten.

use crate::brush::Brush;
use crate::compositor::{layer_blend_over, BlendMode};
use crate::web_surface::WebSurface;
use crate::symmetry::Rectangle;

pub const WEB_MAX_LAYERS: usize = 8;
pub const WEB_MAX_GROUPS: usize = 4;
pub const WEB_HISTORY_RECORDS: usize = 40;
pub const WEB_MIP_MAX_SOURCES: usize = 16;
pub const DISPLAY_LUT_VALUES: usize = 32769;
pub const DISPLAY_LUT_NOISE: usize = 256;
pub const WEB_STROKE_DAB_BUDGET: i32 = 128;

/// `WEB_REF_NONE` / group refs (`WEB_REF_GROUP`).
pub const WEB_REF_NONE: i32 = -1000000;

#[inline]
pub fn web_ref_group(group_id: usize) -> i32 {
    -(group_id as i32) - 1
}

#[inline]
pub fn web_ref_is_group(r: i32) -> bool {
    r < 0 && r != WEB_REF_NONE
}

#[inline]
pub fn web_ref_group_id(r: i32) -> usize {
    (-(r) - 1) as usize
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct TilePos {
    pub tx: i32,
    pub ty: i32,
}

/// One undo record's tile entry.
#[derive(Clone)]
struct HistoryEntry {
    tx: i32,
    ty: i32,
    layer: usize,
    before: Vec<u16>,
    after: Vec<u16>,
}

struct HistoryRecord {
    layer: usize,
    entries: std::ops::Range<usize>,
}

/// Pending first-write snapshots captured during a stroke.
struct PendingCapture {
    pos: TilePos,
    layer: usize,
    before: Vec<u16>,
}

/// The paint application state (one instance per worker).
pub struct PaintApp {
    pub width: i32,
    pub height: i32,
    tile_bytes: usize,

    pub layers: Vec<WebSurface>,
    pub layer_visible: [bool; WEB_MAX_LAYERS],
    pub layer_opacity: [f32; WEB_MAX_LAYERS],
    pub layer_mode: [BlendMode; WEB_MAX_LAYERS],
    layer_count: usize,
    active_layer: usize,

    background_surface: Option<WebSurface>,
    background_color: [u16; 4],
    background_tile: Vec<u16>,

    // Group tree: layers live in `layer_parent` (group index or -1 root),
    // groups in parallel arrays mirroring the C.
    pub layer_parent: Vec<i32>,
    pub layer_next: Vec<i32>,
    pub layer_previous: Vec<i32>,
    pub group_alive: [bool; WEB_MAX_GROUPS],
    pub group_visible: [bool; WEB_MAX_GROUPS],
    pub group_pass_through: [bool; WEB_MAX_GROUPS],
    pub group_isolated: [bool; WEB_MAX_GROUPS],
    pub group_opacity: [f32; WEB_MAX_GROUPS],
    pub group_mode: [BlendMode; WEB_MAX_GROUPS],
    pub group_parent: Vec<i32>,
    pub group_next: Vec<i32>,
    pub group_previous: Vec<i32>,
    pub group_first_child: Vec<i32>,
    pub group_last_child: Vec<i32>,
    pub group_count: usize,
    root_first_child: i32,
    root_last_child: i32,

    pub brush: Option<Brush>,
    pub pending_captures: Vec<PendingCapture>,
    history_entries: Vec<HistoryEntry>,
    history_records: Vec<HistoryRecord>,
    history_cursor: usize,
    history_active: bool,
    history_active_layer: usize,

    composite_tile: Vec<u16>,
    mip_composite_tile: Vec<u16>,
    mip_source_tiles: Vec<u16>,
    display_tile: Vec<u8>,
    display_lut: Vec<u8>,
    display_lut_ready: bool,
    display_eotf: f32,

    pub error_code: i32,
    pub dirty_roi: Vec<crate::symmetry::Rectangle>,
    /// True between `begin_batch` and `end_batch` (the worker's drain
    /// window): `stroke_to` must not end the atomic then, so all of a
    /// drain's dabs land in one ROI.
    pub batch_open: bool,
}

impl PaintApp {
    pub fn new(width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let tile_bytes = 64 * 64 * 4 * 2;
        let mut app = Self {
            width,
            height,
            tile_bytes,
            layers: Vec::with_capacity(WEB_MAX_LAYERS),
            layer_visible: [false; WEB_MAX_LAYERS],
            layer_opacity: [0.0; WEB_MAX_LAYERS],
            layer_mode: [BlendMode::Pigment; WEB_MAX_LAYERS],
            layer_count: 0,
            active_layer: 0,
            background_surface: None,
            background_color: [0; 4],
            background_tile: vec![0; 64 * 64 * 4],
            group_alive: [false; WEB_MAX_GROUPS],
            group_visible: [false; WEB_MAX_GROUPS],
            group_pass_through: [false; WEB_MAX_GROUPS],
            group_isolated: [true; WEB_MAX_GROUPS],
            group_opacity: [1.0; WEB_MAX_GROUPS],
            group_mode: [BlendMode::Normal; WEB_MAX_GROUPS],
            layer_parent: vec![-2; WEB_MAX_LAYERS],
            layer_next: vec![WEB_REF_NONE; WEB_MAX_LAYERS],
            layer_previous: vec![WEB_REF_NONE; WEB_MAX_LAYERS],
            group_parent: vec![-2; WEB_MAX_GROUPS],
            group_next: vec![WEB_REF_NONE; WEB_MAX_GROUPS],
            group_previous: vec![WEB_REF_NONE; WEB_MAX_GROUPS],
            group_first_child: vec![WEB_REF_NONE; WEB_MAX_GROUPS],
            group_last_child: vec![WEB_REF_NONE; WEB_MAX_GROUPS],
            group_count: 0,
            root_first_child: WEB_REF_NONE,
            root_last_child: WEB_REF_NONE,
            brush: None,
            pending_captures: Vec::new(),
            history_entries: Vec::new(),
            history_records: Vec::new(),
            history_cursor: 0,
            history_active: false,
            history_active_layer: 0,
            composite_tile: vec![0; 64 * 64 * 4],
            mip_composite_tile: vec![0; 64 * 64 * 4],
            mip_source_tiles: vec![0; WEB_MIP_MAX_SOURCES * 64 * 64 * 4],
            display_tile: vec![0; 64 * 64 * 4],
            display_lut: vec![0; DISPLAY_LUT_VALUES * DISPLAY_LUT_NOISE],
            display_lut_ready: false,
            display_eotf: 2.2,
            error_code: 0,
            dirty_roi: Vec::new(),
            batch_open: false,
        };
        let layer0 = WebSurface::new(width, height)?;
        app.layers.push(layer0);
        app.layer_visible[0] = true;
        app.layer_opacity[0] = 1.0;
        app.layer_mode[0] = BlendMode::Pigment;
        app.layer_count = 1;
        app.active_layer = 0;
        app.node_append(0, -1);
        // main.c's init() ends with new_brush().
        let mut brush = Brush::new();
        brush.from_defaults();
        brush.new_stroke();
        app.brush = Some(brush);
        app.rebuild_display_lut();
        app.set_background_color(0xA8 as f32 / 255.0, 0xA4 as f32 / 255.0, 0x98 as f32 / 255.0);
        Some(app)
    }

    pub fn active(&mut self) -> &mut WebSurface {
        &mut self.layers[self.active_layer]
    }

    /// `paint_begin_batch`: open one drain window. Inside it `stroke_to`
    /// only queues ops; `end_batch` drains and publishes the ROI.
    pub fn begin_batch(&mut self) {
        if !self.batch_open {
            self.batch_open = true;
            self.active().begin_atomic();
        }
    }

    pub fn end_batch(&mut self) {
        if self.batch_open {
            self.batch_open = false;
            let (roi, _jobs) = self.active().end_atomic();
            self.dirty_roi = roi;
        }
    }

    /// Batch end with tile-parallel dispatch: run all serial bookkeeping
    /// and publish the job table; the caller drains the jobs through
    /// `paint_claim_job`/`paint_process_tile_job` (inline or via the
    /// tile-pool workers). Returns the job count.
    pub fn end_batch_parallel(&mut self) -> i32 {
        if self.batch_open {
            self.batch_open = false;
            return self.active().end_atomic_prepare() as i32;
        }
        0
    }

    pub fn set_background_color(&mut self, r: f32, g: f32, b: f32) {
        let clamp01 = |v: f32| v.clamp(0.0, 1.0);
        let er = clamp01(r).powf(2.2);
        let eg = clamp01(g).powf(2.2);
        let eb = clamp01(b).powf(2.2);
        self.background_color[0] = (er * 32768.0 + 0.5) as u16;
        self.background_color[1] = (eg * 32768.0 + 0.5) as u16;
        self.background_color[2] = (eb * 32768.0 + 0.5) as u16;
        self.background_color[3] = 32768;
        for pixel in 0..64 * 64 {
            self.background_tile[pixel * 4..pixel * 4 + 4]
                .copy_from_slice(&self.background_color);
        }
    }

    pub fn clear_background(&mut self) {
        self.background_color = [0; 4];
        self.background_tile.fill(0);
    }

    fn rebuild_display_lut(&mut self) {
        let inverse_eotf = 1.0 / self.display_eotf;
        for value in 0..DISPLAY_LUT_VALUES {
            for noise in 0..DISPLAY_LUT_NOISE {
                let encoded = ((value as f32 / 32768.0
                    + noise as f32 / (255.0 * 32768.0))
                    .min(1.0)) as f32;
                self.display_lut[value * DISPLAY_LUT_NOISE + noise] =
                    (encoded.powf(inverse_eotf) * 255.0 + 0.5) as u8;
            }
        }
        self.display_lut_ready = true;
    }

    pub fn set_eotf(&mut self, eotf: f32) {
        if eotf.is_finite() && eotf > 0.0 {
            if self.display_lut_ready && (self.display_eotf - eotf).abs() < 0.0001 {
                return;
            }
            self.display_eotf = eotf;
            self.rebuild_display_lut();
        }
    }

    fn display_noise(pixel: usize, channel: usize) -> u32 {
        let value = (pixel as u32)
            .wrapping_mul(747796405)
            .wrapping_add((channel as u32).wrapping_mul(2891336453))
            .wrapping_add(12345);
        (value ^ (value >> 16)) & 255
    }

    fn render_display_tile(&mut self, source: &[u16]) -> &[u8] {
        let display_tile = &mut self.display_tile;
        for pixel in 0..64 * 64 {
            let alpha = source[pixel * 4 + 3] as u32;
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            if alpha != 0 {
                let round_alpha = alpha / 2;
                r = ((source[pixel * 4] as u32) << 15) + round_alpha;
                g = ((source[pixel * 4 + 1] as u32) << 15) + round_alpha;
                b = ((source[pixel * 4 + 2] as u32) << 15) + round_alpha;
                r /= alpha;
                g /= alpha;
                b /= alpha;
                r = r.min(32768);
                g = g.min(32768);
                b = b.min(32768);
            }
            display_tile[pixel * 4] = self.display_lut
                [(r as usize) * DISPLAY_LUT_NOISE + Self::display_noise(pixel, 0) as usize];
            display_tile[pixel * 4 + 1] = self.display_lut
                [(g as usize) * DISPLAY_LUT_NOISE + Self::display_noise(pixel, 1) as usize];
            display_tile[pixel * 4 + 2] = self.display_lut
                [(b as usize) * DISPLAY_LUT_NOISE + Self::display_noise(pixel, 2) as usize];
            display_tile[pixel * 4 + 3] = ((alpha * 255 + 16384) / 32768) as u8;
        }
        display_tile
    }

    fn render_node(&mut self, r: i32, tx: i32, ty: i32, target: &mut [u16]) {
        if web_ref_is_group(r) {
            let group = web_ref_group_id(r) as usize;
            if group >= WEB_MAX_GROUPS
                || !self.group_alive[group]
                || !self.group_visible[group]
            {
                return;
            }
            let direct = self.group_pass_through[group]
                && !self.group_isolated[group]
                && self.group_mode[group] == BlendMode::Normal;
            if direct {
                let opacity = (self.group_opacity[group].clamp(0.0, 1.0) * 32768.0
                    + 0.5) as u32;
                let base = target.to_vec();
                let mut child = self.group_first_child[group];
                while child != WEB_REF_NONE {
                    let next = self.node_next(child);
                    self.render_node(child, tx, ty, target);
                    child = next;
                }
                if opacity < 32768 {
                    let inverse = 32768 - opacity;
                    for pixel in 0..64 * 64 {
                        for channel in 0..4 {
                            let b = base[pixel * 4 + channel] as u32;
                            let res = target[pixel * 4 + channel] as u32;
                            target[pixel * 4 + channel] =
                                ((b * inverse + res * opacity + 16384) >> 15) as u16;
                        }
                    }
                }
                return;
            }
            let mut content = vec![0u16; self.tile_bytes];
            let mut child = self.group_first_child[group];
            while child != WEB_REF_NONE {
                let next = self.node_next(child);
                self.render_node(child, tx, ty, &mut content);
                child = next;
            }
            let opacity = self.group_opacity[group];
            let mode = self.group_mode[group];
            for pixel in 0..64 * 64 {
                layer_blend_over(
                    &mut target[pixel * 4..pixel * 4 + 4],
                    &content[pixel * 4..pixel * 4 + 4],
                    opacity,
                    mode,
                );
            }
            return;
        }
        let layer = r as usize;
        if layer >= self.layer_count || !self.layer_visible[layer] {
            return;
        }
        let Some(source_tile) = self.layers[layer].get_tile(tx, ty) else {
            return;
        };
        let opacity = self.layer_opacity[layer];
        let mode = self.layer_mode[layer];
        for pixel in 0..64 * 64 {
            layer_blend_over(
                &mut target[pixel * 4..pixel * 4 + 4],
                &source_tile[pixel * 4..pixel * 4 + 4],
                opacity,
                mode,
            );
        }
    }

    /// `paint_render_tile_ptr` -- composite the full stack for one tile.
    pub fn render_tile(&mut self, tx: i32, ty: i32) -> &[u16] {
        let mut composite = self.background_tile.clone();
        let mut child = self.root_first_child;
        while child != WEB_REF_NONE {
            let next = self.node_next(child);
            self.render_node(child, tx, ty, &mut composite);
            child = next;
        }
        self.composite_tile = composite;
        &self.composite_tile
    }

    /// `render_display_tile(paint_render_tile_ptr(...))` combined.
    pub fn render_rgba8_tile(&mut self, tx: i32, ty: i32) -> &[u8] {
        self.render_tile(tx, ty);
        let src = self.composite_tile.clone();
        self.render_display_tile(&src);
        &self.display_tile
    }

    pub fn render_layer_rgba8_tile(&mut self, layer_id: usize, tx: i32, ty: i32) -> Option<&[u8]> {
        if layer_id >= self.layer_count {
            return None;
        }
        let tile_copy: Vec<u16> = self.layers[layer_id]
            .get_tile(tx, ty)
            .map(|t| t.to_vec())
            .unwrap_or_default();
        if tile_copy.is_empty() {
            self.display_tile.fill(0);
            return Some(&self.display_tile);
        }
        self.render_display_tile(&tile_copy);
        Some(&self.display_tile)
    }

    /// `paint_write_rgba8_tile` -- import path (RGBA8 into the active layer).
    pub fn write_rgba8_tile(&mut self, tx: i32, ty: i32, source: &[u8]) -> bool {
        if source.len() < 64 * 64 * 4 {
            return false;
        }
        let eotf = self.display_eotf;
        let Some(tile) = self.active().get_or_create_tile_mut(tx, ty) else {
            return false;
        };
        for pixel in 0..64 * 64 {
            let alpha =
                ((source[pixel * 4 + 3] as u32 * 32768 + 127) / 255) as u16;
            for channel in 0..3 {
                let encoded = source[pixel * 4 + channel] as f32 / 255.0;
                let linear = (encoded.powf(eotf) * 32768.0 + 0.5) as u32;
                tile[pixel * 4 + channel] = ((linear * alpha as u32 + 16384) >> 15) as u16;
            }
            tile[pixel * 4 + 3] = alpha;
        }
        true
    }

    /// `paint_region_has_paint`.
    pub fn region_has_paint(&self, tx: i32, ty: i32, level: i32) -> bool {
        if level <= 0 {
            return self.layers[..self.layer_count]
                .iter()
                .enumerate()
                .filter(|(i, _)| self.layer_visible[*i])
                .any(|(_, l)| l.has_tile(tx, ty));
        }
        let level = level.min(2);
        let scale = 1 << level;
        for layer in &self.layers[..self.layer_count] {
            if !layer.is_visible() {
                continue;
            }
            for sy in 0..scale {
                for sx in 0..scale {
                    if layer.has_tile(tx * scale + sx, ty * scale + sy) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `paint_render_rgba8_mip_tile_ptr` -- box-downsampled composite.
    pub fn render_rgba8_mip_tile(&mut self, tx: i32, ty: i32, level: i32) -> &[u8] {
        if level <= 0 {
            let _ = self.render_rgba8_tile(tx, ty);
            return &self.display_tile;
        }
        let level = level.min(2);
        let scale = 1usize << level;
        let has_paint = self
            .layers
            .iter()
            .any(|l| l.used_tile_count() > 0);
        if !has_paint {
            for pixel in 0..64 * 64 {
                self.mip_composite_tile[pixel * 4..pixel * 4 + 4]
                    .copy_from_slice(&self.background_color);
            }
            let t = self.mip_composite_tile.clone();
            self.render_display_tile(&t);
            return &self.display_tile;
        }
        let mut region_has_paint = false;
        'outer: for layer in &self.layers[..self.layer_count] {
            if !layer.is_visible() {
                continue;
            }
            for sy in 0..scale {
                for sx in 0..scale {
                    if layer.has_tile(tx * scale as i32 + sx as i32, ty * scale as i32 + sy as i32) {
                        region_has_paint = true;
                        break 'outer;
                    }
                }
            }
        }
        if !region_has_paint {
            for pixel in 0..64 * 64 {
                self.mip_composite_tile[pixel * 4..pixel * 4 + 4]
                    .copy_from_slice(&self.background_color);
            }
            let t = self.mip_composite_tile.clone();
            self.render_display_tile(&t);
            return &self.display_tile;
        }
        // Snapshot the 4 (or 16) source tiles, then box-average.
        let tile_px = 64 * 64 * 4;
        for sy in 0..scale {
            for sx in 0..scale {
                let src_tx = tx * scale as i32 + sx as i32;
                let src_ty = ty * scale as i32 + sy as i32;
                let rendered = self.render_tile(src_tx, src_ty).to_vec();
                let dst = &mut self.mip_source_tiles
                    [(sy * scale + sx) * tile_px..(sy * scale + sx + 1) * tile_px];
                dst.copy_from_slice(&rendered);
            }
        }
        for pixel_y in 0..64usize {
            for pixel_x in 0..64usize {
                let mut sum = [0u64; 4];
                for sample_y in 0..scale {
                    let source_y = pixel_y * scale + sample_y;
                    let source_tile_y = source_y / 64;
                    let local_y = source_y % 64;
                    for sample_x in 0..scale {
                        let source_x = pixel_x * scale + sample_x;
                        let source_tile_x = source_x / 64;
                        let local_x = source_x % 64;
                        let idx = source_tile_y * scale + source_tile_x;
                        let source = &self.mip_source_tiles[idx * tile_px..(idx + 1) * tile_px];
                        let offset = (local_y * 64 + local_x) * 4;
                        for channel in 0..4 {
                            sum[channel] += source[offset + channel] as u64;
                        }
                    }
                }
                let sample_count = (scale * scale) as u64;
                for channel in 0..4 {
                    self.mip_composite_tile
                        [(pixel_y * 64 + pixel_x) * 4 + channel] =
                        ((sum[channel] + sample_count / 2) / sample_count) as u16;
                }
            }
        }
        let t = self.mip_composite_tile.clone();
        self.render_display_tile(&t);
        &self.display_tile
    }

}

impl PaintApp {
    // ---- layer/group tree (main.c node_* helpers) ----

    #[inline]
    fn node_parent(&self, r: i32) -> i32 {
        if web_ref_is_group(r) {
            self.group_parent[web_ref_group_id(r) as usize]
        } else if r >= 0 && (r as usize) < WEB_MAX_LAYERS {
            self.layer_parent[r as usize]
        } else {
            -2
        }
    }

    #[inline]
    fn node_next(&self, r: i32) -> i32 {
        if web_ref_is_group(r) {
            self.group_next[web_ref_group_id(r) as usize]
        } else if r >= 0 && (r as usize) < WEB_MAX_LAYERS {
            self.layer_next[r as usize]
        } else {
            WEB_REF_NONE
        }
    }

    #[inline]
    fn node_previous(&self, r: i32) -> i32 {
        if web_ref_is_group(r) {
            self.group_previous[web_ref_group_id(r) as usize]
        } else if r >= 0 && (r as usize) < WEB_MAX_LAYERS {
            self.layer_previous[r as usize]
        } else {
            WEB_REF_NONE
        }
    }

    #[inline]
    fn node_set_parent(&mut self, r: i32, parent: i32) {
        if web_ref_is_group(r) {
            self.group_parent[web_ref_group_id(r) as usize] = parent;
        } else if r >= 0 && (r as usize) < WEB_MAX_LAYERS {
            self.layer_parent[r as usize] = parent;
        }
    }

    #[inline]
    fn node_set_next(&mut self, r: i32, next: i32) {
        if web_ref_is_group(r) {
            self.group_next[web_ref_group_id(r) as usize] = next;
        } else if r >= 0 && (r as usize) < WEB_MAX_LAYERS {
            self.layer_next[r as usize] = next;
        }
    }

    #[inline]
    fn node_set_previous(&mut self, r: i32, previous: i32) {
        if web_ref_is_group(r) {
            self.group_previous[web_ref_group_id(r) as usize] = previous;
        } else if r >= 0 && (r as usize) < WEB_MAX_LAYERS {
            self.layer_previous[r as usize] = previous;
        }
    }

    #[inline]
    fn node_first(&self, parent: i32) -> i32 {
        if parent < 0 { self.root_first_child } else { self.group_first_child[parent as usize] }
    }

    #[inline]
    fn node_last(&self, parent: i32) -> i32 {
        if parent < 0 { self.root_last_child } else { self.group_last_child[parent as usize] }
    }

    #[inline]
    fn node_set_first(&mut self, parent: i32, r: i32) {
        if parent < 0 { self.root_first_child = r; } else { self.group_first_child[parent as usize] = r; }
    }

    #[inline]
    fn node_set_last(&mut self, parent: i32, r: i32) {
        if parent < 0 { self.root_last_child = r; } else { self.group_last_child[parent as usize] = r; }
    }

    fn node_remove(&mut self, r: i32) {
        let parent = self.node_parent(r);
        if parent < -1 { return; }
        let previous = self.node_previous(r);
        let next = self.node_next(r);
        if previous == WEB_REF_NONE { self.node_set_first(parent, next); }
        else { self.node_set_next(previous, next); }
        if next == WEB_REF_NONE { self.node_set_last(parent, previous); }
        else { self.node_set_previous(next, previous); }
        self.node_set_parent(r, -2);
        self.node_set_previous(r, WEB_REF_NONE);
        self.node_set_next(r, WEB_REF_NONE);
    }

    fn node_append(&mut self, r: i32, parent: i32) {
        let last = self.node_last(parent);
        self.node_set_parent(r, parent);
        self.node_set_previous(r, last);
        self.node_set_next(r, WEB_REF_NONE);
        if last == WEB_REF_NONE { self.node_set_first(parent, r); }
        else { self.node_set_next(last, r); }
        self.node_set_last(parent, r);
    }

    fn node_insert_before(&mut self, r: i32, before: i32) {
        let parent = self.node_parent(before);
        let previous = self.node_previous(before);
        self.node_set_parent(r, parent);
        self.node_set_previous(r, previous);
        self.node_set_next(r, before);
        self.node_set_previous(before, r);
        if previous == WEB_REF_NONE { self.node_set_first(parent, r); }
        else { self.node_set_next(previous, r); }
    }

    fn group_contains(&self, ancestor: usize, candidate: i32) -> bool {
        let mut current = candidate;
        while current >= 0 && (current as usize) < WEB_MAX_GROUPS {
            if current as usize == ancestor { return true; }
            current = self.group_parent[current as usize];
        }
        false
    }

    fn history_reset_all(&mut self) {
        self.history_entries.clear();
        self.history_records.clear();
        self.history_cursor = 0;
        self.history_active = false;
    }

    pub fn layer_count(&self) -> usize { self.layer_count }
    pub fn active_layer(&self) -> usize { self.active_layer }
}
impl PaintApp {
    /// `begin_stroke` -- history begin + brush reset + zero-pressure warm-up.
    pub fn begin_stroke(&mut self, x: f32, y: f32, xtilt: f32, ytilt: f32,
        viewzoom: f32, viewrotation: f32, barrel_rotation: f32,
    ) {
        self.history_begin();
        let Some(brush) = self.brush.as_mut() else { return };
        self.history_active_layer = self.active_layer;
        self.pending_captures.clear();
        let layer = self.active_layer;
        self.layers[layer].set_capture_enabled(true);
        self.layers[layer].begin_atomic();
        brush.request_reset();
        brush.new_stroke();
        brush.stroke_to(&mut self.layers[layer], x, y, 0.0, xtilt, ytilt,
            10.0, viewzoom, viewrotation, barrel_rotation, false);
        let (roi, _jobs) = self.layers[layer].end_atomic();
        self.dirty_roi = roi;
        let captured = self.layers[layer].take_captured();
        for (tx, ty, before) in captured {
            self.pending_captures.push(PendingCapture {
                pos: TilePos { tx, ty },
                layer: self.active_layer,
                before,
            });
        }
    }

    /// `stroke_to` -- one motion sample through the budgeted driver.
    pub fn stroke_to(&mut self, x: f32, y: f32, pressure: f32, xtilt: f32,
        ytilt: f32, dtime: f64, viewzoom: f32, viewrotation: f32,
        barrel_rotation: f32, linear: bool,
    ) -> i32 {
        let Some(brush) = self.brush.as_mut() else { return -1 };
        let layer = self.active_layer;
        let open = self.batch_open;
        if !open {
            self.layers[layer].begin_atomic();
        }
        let result = brush.stroke_to(&mut self.layers[layer], x, y, pressure,
            xtilt, ytilt, dtime, viewzoom, viewrotation, barrel_rotation,
            linear);
        if !open {
            let (roi, _jobs) = self.layers[layer].end_atomic();
            self.dirty_roi = roi;
        }
        self.absorb_captures();
        if result { 2 } else { 1 }
    }

    fn absorb_captures(&mut self) {
        let layer = self.active_layer;
        let captured = self.layers[layer].take_captured();
        for (tx, ty, before) in captured {
            self.pending_captures.push(PendingCapture {
                pos: TilePos { tx, ty },
                layer,
                before,
            });
        }
    }

    pub fn history_begin(&mut self) {
        if self.history_active {
            self.history_commit();
            if self.history_active { return; }
        }
        if self.history_cursor < self.history_records.len() {
            self.history_records.truncate(self.history_cursor);
        }
        while self.history_records.len() >= WEB_HISTORY_RECORDS {
            self.history_records.remove(0);
        }
        self.history_active = true;
        self.history_active_layer = self.active_layer;
    }

    pub fn history_commit(&mut self) {
        if !self.history_active { return; }
        if self.history_active_layer != self.active_layer {
            self.history_active = false;
            return;
        }
        let layer = self.history_active_layer;
        // The stroke capture stays enabled for the whole stroke; close it
        // here and absorb any tiles written after the last stroke_to.
        self.layers[layer].set_capture_enabled(false);
        let captured = self.layers[layer].take_captured();
        for (tx, ty, before) in captured {
            self.pending_captures.push(PendingCapture {
                pos: TilePos { tx, ty },
                layer,
                before,
            });
        }
        let mut record: Vec<HistoryEntry> = Vec::new();
        for capture in &self.pending_captures {
            let after = self.layers[layer]
                .get_tile(capture.pos.tx, capture.pos.ty)
                .map(|t| t.to_vec())
                .unwrap_or_else(|| vec![0u16; capture.before.len()]);
            record.push(HistoryEntry {
                tx: capture.pos.tx,
                ty: capture.pos.ty,
                layer,
                before: capture.before.clone(),
                after,
            });
        }
        if !record.is_empty() {
            self.history_records.push(HistoryRecord {
                layer,
                entries: self.history_entries.len()
                    ..self.history_entries.len() + record.len(),
            });
            self.history_entries.extend(record);
            self.history_cursor = self.history_records.len();
        }
        self.pending_captures.clear();
        self.history_active = false;
    }

    pub fn history_undo(&mut self) -> bool {
        if self.history_cursor == 0 { return false; }
        self.history_cursor -= 1;
        let (start, end, layer) = {
            let record = &self.history_records[self.history_cursor];
            (record.entries.start, record.entries.end, record.layer)
        };
        let entries: Vec<HistoryEntry> = self.history_entries[start..end].to_vec();
        self.active_layer = layer;
        for e in entries {
            let Some(tile) = self.layers[layer].get_or_create_tile_mut(e.tx, e.ty)
            else {
                self.error_code = 1;
                return false;
            };
            tile.copy_from_slice(&e.before);
        }
        true
    }

    pub fn history_redo(&mut self) -> bool {
        if self.history_cursor >= self.history_records.len() { return false; }
        let (start, end, layer) = {
            let record = &self.history_records[self.history_cursor];
            (record.entries.start, record.entries.end, record.layer)
        };
        let entries: Vec<HistoryEntry> = self.history_entries[start..end].to_vec();
        self.active_layer = layer;
        for e in entries {
            let Some(tile) = self.layers[layer].get_or_create_tile_mut(e.tx, e.ty)
            else {
                self.error_code = 1;
                return false;
            };
            tile.copy_from_slice(&e.after);
        }
        self.history_cursor += 1;
        true
    }

    pub fn history_can_undo(&self) -> bool { self.history_cursor > 0 }
    pub fn history_can_redo(&self) -> bool { self.history_cursor < self.history_records.len() }

    pub fn clear(&mut self) {
        self.active().clear_tiles();
    }

    pub fn pick_color(&mut self, x: f32, y: f32, radius: f32, paint: f32) -> [f32; 4] {
        self.active().get_color(x, y, radius, paint)
    }

    pub fn set_symmetry(&mut self, active: bool, cx: f32, cy: f32, angle: f32,
        kind: i32, lines: i32,
    ) {
        self.active().set_symmetry(active, cx, cy, angle, kind, lines);
    }

    pub fn create_layer(&mut self) -> i32 {
        if self.layer_count >= WEB_MAX_LAYERS { return -1; }
        let (w, h) = (self.width, self.height);
        let Some(surface) = WebSurface::new(w, h) else { return -1 };
        let id = self.layer_count;
        self.layers.push(surface);
        self.layer_visible[id] = true;
        self.layer_opacity[id] = 1.0;
        self.layer_mode[id] = BlendMode::Pigment;
        self.layer_count += 1;
        self.node_append(id as i32, -1);
        self.active_layer = id;
        id as i32
    }

    pub fn delete_layer(&mut self, layer_id: usize) -> bool {
        if self.layer_count <= 1 || layer_id >= self.layer_count { return false; }
        self.node_remove(layer_id as i32);
        self.layers.remove(layer_id);
        for i in layer_id..self.layer_count - 1 {
            self.layer_visible[i] = self.layer_visible[i + 1];
            self.layer_opacity[i] = self.layer_opacity[i + 1];
            self.layer_mode[i] = self.layer_mode[i + 1];
        }
        self.layer_count -= 1;
        let removed = layer_id as i32;
        let remap = |r: i32| -> i32 {
            if r == WEB_REF_NONE || r == removed { WEB_REF_NONE }
            else if r > removed { r - 1 } else { r }
        };
        for i in 0..WEB_MAX_GROUPS {
            self.group_first_child[i] = remap(self.group_first_child[i]);
            self.group_last_child[i] = remap(self.group_last_child[i]);
            self.group_next[i] = remap(self.group_next[i]);
            self.group_previous[i] = remap(self.group_previous[i]);
        }
        self.root_first_child = remap(self.root_first_child);
        self.root_last_child = remap(self.root_last_child);
        for i in 0..self.layer_count {
            self.layer_next[i] = remap(self.layer_next[i]);
            self.layer_previous[i] = remap(self.layer_previous[i]);
        }
        if self.active_layer == layer_id {
            self.active_layer = if layer_id < self.layer_count { layer_id } else { self.layer_count - 1 };
        } else if self.active_layer > layer_id {
            self.active_layer -= 1;
        }
        self.history_reset_all();
        true
    }

    pub fn set_layer_visible(&mut self, layer_id: usize, visible: bool) {
        if layer_id < self.layer_count { self.layer_visible[layer_id] = visible; }
    }

    pub fn set_layer_opacity(&mut self, layer_id: usize, opacity: f32) {
        if layer_id < self.layer_count { self.layer_opacity[layer_id] = opacity.clamp(0.0, 1.0); }
    }

    pub fn set_layer_mode(&mut self, layer_id: usize, mode: BlendMode) {
        if layer_id < self.layer_count { self.layer_mode[layer_id] = mode; }
    }

    pub fn set_layer_group(&mut self, layer_id: usize, group_id: i32) -> bool {
        if layer_id >= self.layer_count { return false; }
        if group_id >= 0 && (group_id as usize >= WEB_MAX_GROUPS || !self.group_alive[group_id as usize]) {
            return false;
        }
        self.node_remove(layer_id as i32);
        self.node_append(layer_id as i32, group_id);
        self.history_reset_all();
        true
    }

    fn move_node(&mut self, r: i32, direction: i32) -> bool {
        let neighbor = if direction < 0 { self.node_previous(r) } else { self.node_next(r) };
        if neighbor == WEB_REF_NONE { return false; }
        let parent = self.node_parent(r);
        let after = if direction < 0 { neighbor } else { self.node_next(neighbor) };
        self.node_remove(r);
        if direction < 0 { self.node_insert_before(r, neighbor); }
        else if after == WEB_REF_NONE { self.node_append(r, parent); }
        else { self.node_insert_before(r, after); }
        true
    }

    pub fn move_layer(&mut self, layer_id: usize, direction: i32) -> bool {
        if layer_id >= self.layer_count || (direction != -1 && direction != 1) { return false; }
        self.move_node(layer_id as i32, direction)
    }

    pub fn create_group(&mut self) -> i32 {
        let Some(group) = self.group_alive.iter().position(|a| !a) else { return -1; };
        self.group_alive[group] = true;
        self.group_visible[group] = true;
        self.group_pass_through[group] = false;
        self.group_isolated[group] = true;
        self.group_opacity[group] = 1.0;
        self.group_mode[group] = BlendMode::Normal;
        self.group_first_child[group] = WEB_REF_NONE;
        self.group_last_child[group] = WEB_REF_NONE;
        self.group_count = self.group_count.max(group + 1);
        self.node_append(web_ref_group(group), -1);
        group as i32
    }

    pub fn delete_group(&mut self, group_id: usize) -> bool {
        if group_id >= WEB_MAX_GROUPS || !self.group_alive[group_id] { return false; }
        let parent = self.group_parent[group_id];
        let mut child = self.group_first_child[group_id];
        while child != WEB_REF_NONE {
            let next = self.node_next(child);
            self.node_remove(child);
            self.node_append(child, parent);
            child = next;
        }
        self.node_remove(web_ref_group(group_id));
        self.group_alive[group_id] = false;
        self.group_visible[group_id] = false;
        self.group_first_child[group_id] = WEB_REF_NONE;
        self.group_last_child[group_id] = WEB_REF_NONE;
        self.group_parent[group_id] = -2;
        while self.group_count > 0 && !self.group_alive[self.group_count - 1] { self.group_count -= 1; }
        self.history_reset_all();
        true
    }

    pub fn set_group_parent(&mut self, group_id: usize, parent_id: i32) -> bool {
        if group_id >= WEB_MAX_GROUPS || !self.group_alive[group_id] { return false; }
        if parent_id >= 0 && (parent_id as usize >= WEB_MAX_GROUPS || !self.group_alive[parent_id as usize]) {
            return false;
        }
        if parent_id == group_id as i32 || (parent_id >= 0 && self.group_contains(group_id, parent_id)) {
            return false;
        }
        self.node_remove(web_ref_group(group_id));
        self.node_append(web_ref_group(group_id), parent_id);
        self.history_reset_all();
        true
    }

    pub fn move_group(&mut self, group_id: usize, direction: i32) -> bool {
        if group_id >= WEB_MAX_GROUPS || !self.group_alive[group_id]
            || (direction != -1 && direction != 1) { return false; }
        self.move_node(web_ref_group(group_id), direction)
    }
}
impl PaintApp {
    pub fn dirty_roi_len(&self) -> usize {
        self.dirty_roi.len()
    }

    pub fn set_dirty_roi(&mut self, roi: Vec<crate::symmetry::Rectangle>) {
        self.dirty_roi = roi;
    }

    pub fn clear_dirty_roi(&mut self) {
        self.dirty_roi.clear();
    }

    pub fn set_active_layer(&mut self, id: usize) -> i32 {
        if id >= self.layer_count {
            return 0;
        }
        self.active_layer = id;
        1
    }

    pub fn group_count(&self) -> usize {
        self.group_count
    }
}
