//! The demo's paint surface -- port of `web-surface.c` + the tiled-surface
//! machinery it embeds (`begin_atomic`/`end_atomic` dirty rectangles,
//! per-dab bounding boxes, the symmetry passes of `draw_dab`, and the fixed
//! op queue). Sparse tiles in an open-addressed hash; first-write capture
//! feeds the history system; fixed-capacity storage with deterministic
//! overflow.

use crate::mask::render_dab_mask;
use crate::surface::{DrawDabOp, NULL_DAB_OP, Surface};
use crate::symmetry::{
    Rectangle, SymmetryData, rectangle_expand_to_include_point, update_symmetry_state,
};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::Ordering;

const TILE: usize = 64;
const TILE_PX: usize = TILE * TILE * 4;
const MASK_LEN: usize = TILE * TILE + 2 * TILE;
const WEB_SURFACE_MIN_HASH_SIZE: usize = 8192;
const MAX_DIRTY_RECTS: usize = 32;
const OP_QUEUE_CAP: usize = 16384;
/// Initial shared RGBA16 tile limit. One block is 128 MiB of tile pixels.
pub(crate) const INITIAL_RESIDENT_TILES: usize = 4096;

pub(crate) struct TileBudget {
    used: Cell<usize>,
    limit: Cell<usize>,
    maximum: usize,
}

impl TileBudget {
    pub(crate) fn new(initial: usize, maximum: usize) -> Self {
        let maximum = maximum.max(1);
        Self {
            used: Cell::new(0),
            limit: Cell::new(initial.clamp(1, maximum)),
            maximum,
        }
    }

    pub(crate) fn used(&self) -> usize {
        self.used.get()
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit.get()
    }

    pub(crate) fn maximum(&self) -> usize {
        self.maximum
    }

    pub(crate) fn set_limit(&self, limit: usize) -> bool {
        if limit < self.used.get() || limit > self.maximum {
            return false;
        }
        self.limit.set(limit);
        true
    }
}

// ---- Tile-parallel blend jobs (shared with the pool workers) ----
//
// `end_atomic_prepare` partitions the queued ops into one blend job per
// dirty tile and fills this shared table. The host (or this instance)
// blends via `process_job`; each tile is owned by exactly one job.
pub const MAX_JOBS: usize = 4096;
pub const JOB_WORKERS: usize = 16;
const JOB_OPS_OFF: usize = 0;
const JOB_OP_COUNT: usize = 1;
const JOB_TILE_ADDR: usize = 2;
const JOB_TX: usize = 3;
const JOB_TY: usize = 4;
const JOB_WORDS: usize = 5;

static JOB_TABLE: [std::sync::atomic::AtomicUsize; MAX_JOBS * JOB_WORDS] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; MAX_JOBS * JOB_WORDS];
static JOB_OP_PROGRESS: [std::sync::atomic::AtomicUsize; MAX_JOBS] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; MAX_JOBS];
static JOB_ROW_PROGRESS: [std::sync::atomic::AtomicUsize; MAX_JOBS] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; MAX_JOBS];
#[cfg(test)]
pub(crate) static JOB_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// Single writer during `end_atomic_prepare` (the batch owner); pool
/// workers read slices after claiming, so no two readers mutate.
/// A plain-data arena shared across the module instances (the paint worker
/// and the tile-pool workers) through the common linear memory.
/// `UnsafeCell` is always `!Sync`, so the shared arenas need this wrapper.
struct SharedArena<T: Copy>(std::cell::UnsafeCell<T>);
unsafe impl<T: Copy> Sync for SharedArena<T> {}

static OP_ARENA: SharedArena<[DrawDabOp; OP_QUEUE_CAP]> =
    SharedArena(std::cell::UnsafeCell::new([NULL_DAB_OP; OP_QUEUE_CAP]));
static WORKER_MASKS: SharedArena<[u16; JOB_WORKERS * MASK_LEN]> =
    SharedArena(std::cell::UnsafeCell::new([0; JOB_WORKERS * MASK_LEN]));
static WORKER_SCRATCH: SharedArena<[f32; JOB_WORKERS * MASK_LEN]> =
    SharedArena(std::cell::UnsafeCell::new([0.0; JOB_WORKERS * MASK_LEN]));

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
    storage_dirty: Vec<bool>,

    null_tile: Vec<u16>,
    mask: Vec<u16>,
    scratch: Vec<f32>,
    /// Reused copy-out buffer for `get_color` tile reads (no per-call
    /// allocation: the smudge path calls this for every dab).
    smudge_tile: Vec<u16>,
    /// Reused matching-operation indexes for sparse smudge samples.
    smudge_op_indices: Vec<usize>,
    /// Persistent pixel-sampling stream for `get_color` (one stream across
    /// tiles and calls, like the C `rand()` stream).
    random: Box<dyn crate::random::RandomSource>,
    captured_bytes: usize,
    capture_byte_limit: usize,

    ops: Vec<QueuedOp>,
    op_len: usize,
    jobs_used: usize,
    pending_roi: Vec<Rectangle>,
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
    tile_budget: Rc<TileBudget>,
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
    fn document_tile_count(&self) -> usize {
        (self.tiles_width * self.tiles_height) as usize
    }

    pub fn new(width: i32, height: i32) -> Option<Self> {
        Self::new_with_budget(
            width,
            height,
            Rc::new(TileBudget::new(
                INITIAL_RESIDENT_TILES,
                INITIAL_RESIDENT_TILES,
            )),
        )
    }

    pub(crate) fn new_with_budget(
        width: i32,
        height: i32,
        tile_budget: Rc<TileBudget>,
    ) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let tiles_width = (width + TILE as i32 - 1) / TILE as i32;
        let tiles_height = (height + TILE as i32 - 1) / TILE as i32;
        let tile_capacity = ((tiles_width * tiles_height) as usize)
            .min(tile_budget.limit())
            .min(INITIAL_RESIDENT_TILES);
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
            storage_dirty: vec![false; tile_capacity],
            null_tile: vec![0; TILE_PX],
            mask: vec![0; MASK_LEN],
            scratch: vec![0.0; MASK_LEN],
            smudge_tile: Vec::with_capacity(TILE_PX),
            smudge_op_indices: Vec::with_capacity(OP_QUEUE_CAP),
            random: Box::new(crate::random::PortableRand::default()),
            captured_bytes: 0,
            capture_byte_limit: crate::app::HISTORY_BYTE_BUDGET / 2,
            ops: vec![
                QueuedOp {
                    tx: 0,
                    ty: 0,
                    op: NULL_DAB_OP
                };
                OP_QUEUE_CAP
            ],
            op_len: 0,
            jobs_used: 0,
            pending_roi: Vec::new(),
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
            tile_budget,
        })
    }

    fn grow_metadata(&mut self) -> bool {
        let target = self.document_tile_count().min(self.tile_budget.limit());
        if self.display_dirty.len() < target {
            let add = target - self.display_dirty.len();
            if self.tiles.try_reserve_exact(add).is_err()
                || self.tile_tx.try_reserve_exact(add).is_err()
                || self.tile_ty.try_reserve_exact(add).is_err()
                || self.display_dirty_slots.try_reserve_exact(add).is_err()
                || self.display_dirty.try_reserve_exact(add).is_err()
                || self.storage_dirty.try_reserve_exact(add).is_err()
                || self.capture_marks.try_reserve_exact(add).is_err()
            {
                return false;
            }
            self.display_dirty.resize(target, false);
            self.storage_dirty.resize(target, false);
            self.capture_marks.resize(target, 0);
        }

        let mut hash_size = self.hash_size;
        while hash_size < target * 2 {
            hash_size <<= 1;
        }
        if hash_size == self.hash_size {
            return true;
        }
        let hash_add = hash_size - self.hash_size;
        if self.slot_used.try_reserve_exact(hash_add).is_err()
            || self.slot_tx.try_reserve_exact(hash_add).is_err()
            || self.slot_ty.try_reserve_exact(hash_add).is_err()
            || self.slot_index.try_reserve_exact(hash_add).is_err()
        {
            return false;
        }
        self.slot_used.resize(hash_size, false);
        self.slot_tx.resize(hash_size, 0);
        self.slot_ty.resize(hash_size, 0);
        self.slot_index.resize(hash_size, 0);
        self.hash_size = hash_size;
        self.rebuild_hash();
        true
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
    pub fn queue_failed(&self) -> u32 {
        self.op_failed
    }

    pub fn take_queue_error(&mut self) -> bool {
        let failed = self.op_failed != 0;
        self.op_failed = 0;
        failed
    }

    pub fn take_capacity_error(&mut self) -> bool {
        let f = self.capacity_failed;
        self.capacity_failed = false;
        f
    }

    #[inline]
    fn find_hash_slot(&self, tx: i32, ty: i32) -> Option<usize> {
        let start = tile_hash(tx, ty) & (self.hash_size as u32 - 1);
        for probe in 0..self.hash_size as u32 {
            let slot = ((start + probe) & (self.hash_size as u32 - 1)) as usize;
            if !self.slot_used[slot] {
                return None;
            }
            if self.slot_tx[slot] == tx && self.slot_ty[slot] == ty {
                return Some(slot);
            }
        }
        None
    }

    fn find_tile_slot(&self, tx: i32, ty: i32) -> Option<usize> {
        self.find_hash_slot(tx, ty)
            .map(|slot| self.slot_index[slot])
    }

    fn create_tile_slot(&mut self, tx: i32, ty: i32) -> Option<usize> {
        if let Some(index) = self.find_tile_slot(tx, ty) {
            return Some(index);
        }
        if self.tile_budget.used() >= self.tile_budget.limit() || !self.grow_metadata() {
            self.capacity_failed = true;
            return None;
        }
        let start = tile_hash(tx, ty) & (self.hash_size as u32 - 1);
        for probe in 0..self.hash_size as u32 {
            let slot = ((start + probe) & (self.hash_size as u32 - 1)) as usize;
            if self.slot_used[slot] {
                continue;
            }
            let index = self.tiles.len();
            let tile = match Box::try_new([0u16; TILE_PX]) {
                Ok(tile) => tile,
                Err(_) => {
                    self.capacity_failed = true;
                    return None;
                }
            };
            self.tiles.push(Some(tile));
            self.tile_budget.used.set(self.tile_budget.used() + 1);
            self.tile_tx.push(tx);
            self.tile_ty.push(ty);
            self.storage_dirty[index] = true;
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
        self.storage_dirty[slot] = true;
        Some(self.tiles[slot].as_mut()?.as_mut())
    }

    /// `web_surface_has_tile`.
    pub fn has_tile(&self, tx: i32, ty: i32) -> bool {
        self.find_tile_slot(tx, ty).is_some()
    }

    /// Return whether the current stroke already captured this resident tile.
    pub fn tile_is_captured(&self, tx: i32, ty: i32) -> bool {
        if !self.capture_enabled {
            return false;
        }
        self.find_hash_slot(tx, ty)
            .map(|slot| self.capture_marks[self.slot_index[slot]] == self.capture_generation)
            .unwrap_or(false)
    }

    /// Remove one resident tile after the worker has saved its bytes.
    pub fn remove_tile(&mut self, tx: i32, ty: i32) -> bool {
        let Some(hash_slot) = self.find_hash_slot(tx, ty) else {
            return false;
        };
        let index = self.slot_index[hash_slot];
        let last = self.tiles.len() - 1;
        self.tiles.swap_remove(index);
        self.tile_tx.swap_remove(index);
        self.tile_ty.swap_remove(index);
        if index != last {
            self.display_dirty[index] = self.display_dirty[last];
            self.storage_dirty[index] = self.storage_dirty[last];
            self.capture_marks[index] = self.capture_marks[last];
        }
        let mut write = 0;
        for read in 0..self.display_dirty_slots.len() {
            let mut dirty_slot = self.display_dirty_slots[read];
            if dirty_slot == index {
                continue;
            }
            if dirty_slot == last {
                dirty_slot = index;
            }
            self.display_dirty_slots[write] = dirty_slot;
            write += 1;
        }
        self.display_dirty_slots.truncate(write);
        self.display_dirty[last] = false;
        self.storage_dirty[last] = false;
        self.capture_marks[last] = 0;
        self.slot_used[hash_slot] = false;
        self.tile_budget
            .used
            .set(self.tile_budget.used().saturating_sub(1));
        self.rebuild_hash();
        true
    }

    fn rebuild_hash(&mut self) {
        self.slot_used.fill(false);
        for (index, (&tx, &ty)) in self.tile_tx.iter().zip(&self.tile_ty).enumerate() {
            let start = tile_hash(tx, ty) & (self.hash_size as u32 - 1);
            for probe in 0..self.hash_size as u32 {
                let slot = ((start + probe) & (self.hash_size as u32 - 1)) as usize;
                if !self.slot_used[slot] {
                    self.slot_used[slot] = true;
                    self.slot_tx[slot] = tx;
                    self.slot_ty[slot] = ty;
                    self.slot_index[slot] = index;
                    break;
                }
            }
        }
    }

    /// Write one raw RGBA16 tile without changing display state.
    pub fn write_rgba16_tile(&mut self, tx: i32, ty: i32, source: &[u16]) -> bool {
        if source.len() < TILE_PX {
            return false;
        }
        let Some(slot) = self
            .find_tile_slot(tx, ty)
            .or_else(|| self.create_tile_slot(tx, ty))
        else {
            return false;
        };
        self.tiles[slot]
            .as_mut()
            .unwrap()
            .copy_from_slice(&source[..TILE_PX]);
        self.storage_dirty[slot] = false;
        true
    }

    pub fn write_rgba16_tile_modified(&mut self, tx: i32, ty: i32, source: &[u16]) -> bool {
        if !self.write_rgba16_tile(tx, ty, source) {
            return false;
        }
        let slot = self.find_tile_slot(tx, ty).unwrap();
        self.storage_dirty[slot] = true;
        true
    }

    pub fn set_external_history(&mut self, enabled: bool) {
        self.capture_byte_limit = if enabled {
            usize::MAX
        } else {
            crate::app::HISTORY_BYTE_BUDGET / 2
        };
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
            self.captured_bytes = 0;
            self.capture_overflow = false;
        }
        self.capture_enabled = enabled;
    }

    pub fn take_captured(&mut self) -> Vec<(i32, i32, Vec<u16>)> {
        self.captured_bytes = 0;
        std::mem::take(&mut self.captured)
    }

    pub fn take_capture_error(&mut self) -> bool {
        let failed = self.capture_overflow;
        self.capture_overflow = false;
        failed
    }

    /// Snapshot a tile's before-state on first write of the stroke.
    fn capture_first_write(&mut self, slot: usize, tx: i32, ty: i32) -> bool {
        if !self.capture_enabled {
            return true;
        }
        if self.capture_overflow {
            return false;
        }
        if self.capture_marks[slot] == self.capture_generation {
            return true; // already captured this stroke
        }
        let words = self.tiles[slot].as_ref().unwrap().len();
        let bytes = words * std::mem::size_of::<u16>();
        if self.captured_bytes.saturating_add(bytes) > self.capture_byte_limit {
            self.capture_overflow = true;
            return false;
        }
        if self.captured.try_reserve(1).is_err() {
            self.capture_overflow = true;
            return false;
        }
        let mut before = Vec::new();
        if before.try_reserve_exact(words).is_err() {
            self.capture_overflow = true;
            return false;
        }
        before.extend_from_slice(&self.tiles[slot].as_ref().unwrap()[..]);
        self.capture_marks[slot] = self.capture_generation;
        self.captured_bytes += bytes;
        self.captured.push((tx, ty, before));
        true
    }

    fn mark_display_dirty(&mut self, slot: usize) {
        if self.display_dirty[slot] {
            return;
        }
        self.display_dirty[slot] = true;
        self.display_dirty_slots.push(slot);
    }

    /// `begin_atomic` (updates the symmetry state + clears dirty bboxes).
    /// Mirrors the C `prepare_bounding_boxes`: zero the previously dirtied
    /// bboxes first (expand treats width 0 as empty), then reset the count.
    pub fn begin_atomic(&mut self) {
        update_symmetry_state(&mut self.symmetry);
        for bbox in &mut self.bboxes[..self.num_bboxes_dirtied.min(MAX_DIRTY_RECTS)] {
            *bbox = Rectangle::default();
        }
        self.num_bboxes_dirtied = 0;
        self.jobs_used = 0;
        self.atomic_active = true;
    }

    /// `end_atomic` -- drain the op queue per dirty tile, then merge the
    /// per-dab bounding boxes into a fresh roi (`mypaint_tiled_surface_end_atomic`).
    pub fn end_atomic(&mut self) -> (Vec<Rectangle>, u32) {
        let mut total = 0;
        loop {
            let count = self.end_atomic_prepare();
            for index in 0..count {
                process_job(index as i32, 0);
            }
            total += count;
            if self.op_len == 0 || count == 0 {
                break;
            }
        }
        (std::mem::take(&mut self.pending_roi), total)
    }

    /// The C tail of `mypaint_tiled_surface_end_atomic`: copy the dirtied
    /// bounding boxes into the roi. `num_bboxes_dirtied` is capped at
    /// MAX_DIRTY_RECTS, so the C's merge-adjacent-slots distribution can
    /// never be needed here.
    fn merge_dirty_bboxes(&mut self) -> Vec<Rectangle> {
        self.bboxes[..self.num_bboxes_dirtied.min(MAX_DIRTY_RECTS)].to_vec()
    }

    /// Serial half of the batch end: partition the op queue per dirty tile,
    /// run all bookkeeping (slot creation, first-write capture, display
    /// dirty, bbox ROI), and fill the shared job table with one blend job
    /// per dirty tile. The blend then runs via `process_job` on this
    /// instance or via `paint_process_tile_job` on any module instance
    /// sharing the same linear memory (the tile-pool workers).
    pub fn end_atomic_prepare(&mut self) -> u32 {
        loop {
            self.jobs_used = 0;
            let dirty = self.partition_ops();
            let count = dirty.len().min(MAX_JOBS);
            let arena = unsafe { &mut *OP_ARENA.0.get() };
            let mut ops_base = 0usize;
            for (tx, ty) in dirty.iter().take(count) {
                ops_base = self.prepare_tile(*tx, *ty, arena, ops_base);
            }
            self.pending_roi = self.merge_dirty_bboxes();
            if self.jobs_used > 0 || self.op_len == 0 {
                return self.jobs_used as u32;
            }
        }
    }

    pub fn has_pending_ops(&self) -> bool {
        self.op_len > 0
    }

    fn partition_ops(&mut self) -> Vec<(i32, i32)> {
        let mut dirty: Vec<(i32, i32)> = Vec::new();
        for r in 0..self.op_len {
            let e = self.ops[r];
            if !dirty.contains(&(e.tx, e.ty)) {
                dirty.push((e.tx, e.ty));
            }
        }
        dirty
    }

    /// Bookkeeping + job fill for one dirty tile. Drains the tile's ops
    /// from the queue in FIFO order (the serial, deterministic half).
    /// Returns the ops arena offset after this tile's batch.
    fn prepare_tile(
        &mut self,
        tx: i32,
        ty: i32,
        arena: &mut [DrawDabOp],
        ops_base: usize,
    ) -> usize {
        // Copy matching ops directly into the fixed arena while compacting
        // the remaining queue. This preserves FIFO order without a Vec per
        // tile.
        let mut op_count = 0usize;
        let mut w = 0usize;
        for r in 0..self.op_len {
            let e = self.ops[r];
            if e.tx == tx && e.ty == ty {
                arena[ops_base + op_count] = e.op;
                op_count += 1;
            } else {
                self.ops[w] = e;
                w += 1;
            }
        }
        self.op_len = w;
        if op_count == 0 {
            return ops_base;
        }
        let Some(slot) = self
            .find_tile_slot(tx, ty)
            .or_else(|| self.create_tile_slot(tx, ty))
        else {
            return ops_base; // out of capacity: ops land in the (zeroed) null tile
        };
        if !self.capture_first_write(slot, tx, ty) {
            return ops_base;
        }
        self.mark_display_dirty(slot);
        self.storage_dirty[slot] = true;

        let job = self.jobs_used;
        self.jobs_used += 1;
        let tile_addr = self.tiles[slot].as_mut().unwrap().as_mut_ptr() as usize;
        let words = &JOB_TABLE[job * JOB_WORDS..(job + 1) * JOB_WORDS];
        words[JOB_OPS_OFF].store(ops_base, Ordering::Relaxed);
        words[JOB_OP_COUNT].store(op_count, Ordering::Relaxed);
        words[JOB_TILE_ADDR].store(tile_addr, Ordering::Relaxed);
        words[JOB_TX].store(tx as usize, Ordering::Relaxed);
        words[JOB_TY].store(ty as usize, Ordering::Relaxed);
        JOB_OP_PROGRESS[job].store(0, Ordering::Relaxed);
        JOB_ROW_PROGRESS[job].store(0, Ordering::Release);
        ops_base + op_count
    }

    fn process_tile(&mut self, tx: i32, ty: i32) {
        // Drain matching ops in FIFO order and compact the rest in place.
        // The tile is created only when the first matching op occurs.
        let mut slot = self.find_tile_slot(tx, ty);
        let mut prepared = false;
        let mut w = 0usize;
        for r in 0..self.op_len {
            let e = self.ops[r];
            if e.tx == tx && e.ty == ty {
                if !prepared {
                    if let Some(index) = slot {
                        if self.capture_first_write(index, tx, ty) {
                            self.mark_display_dirty(index);
                            self.storage_dirty[index] = true;
                        } else {
                            slot = None;
                        }
                    } else {
                        slot = self.create_tile_slot(tx, ty);
                        if let Some(index) = slot {
                            if self.capture_first_write(index, tx, ty) {
                                self.mark_display_dirty(index);
                                self.storage_dirty[index] = true;
                            } else {
                                slot = None;
                            }
                        }
                    }
                    prepared = true;
                }
                if let Some(index) = slot {
                    let tile = &mut self.tiles[index].as_mut().unwrap()[..];
                    crate::surface::process_op(
                        tile,
                        &mut self.mask[..],
                        tx,
                        ty,
                        &e.op,
                        &mut self.scratch[..],
                    );
                }
            } else {
                self.ops[w] = e;
                w += 1;
            }
        }
        self.op_len = w;
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
        // Fixed-capacity divergence from the C: the C queues every touched
        // tile and its tile request returns NULL outside the canvas. Here
        // tiles are allocated on demand, so ops outside the document grid
        // are dropped instead of allocating tiles that can never display
        // (they would only eat the resident-tile budget and the history).
        let mut queued_any = false;
        for ty in ty1.max(0)..=ty2.min(self.tiles_height - 1) {
            for tx in tx1.max(0)..=tx2.min(self.tiles_width - 1) {
                if self.op_len < OP_QUEUE_CAP {
                    self.ops[self.op_len] = QueuedOp { tx, ty, op };
                    self.op_len += 1;
                    queued_any = true;
                } else {
                    self.op_failed += 1;
                }
            }
        }
        queued_any
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
            x,
            y,
            radius,
            color_r,
            color_g,
            color_b,
            opaque,
            hardness,
            softness,
            color_a,
            aspect_ratio,
            angle,
            lock_alpha,
            colorize,
            posterize,
            posterize_num,
            paint,
            0,
        );
        // The C `draw_dab` tail: how many bboxes this call dirtied (1 for
        // the normal pass; the symmetry passes raise it below).
        self.num_bboxes_dirtied = if surface_modified { 1 } else { 0 };
        let symm = self.symmetry.state_current;
        if !(surface_modified && self.symmetry.active && self.symmetry.num_symmetry_matrices > 0) {
            return surface_modified;
        }
        let num_bboxes = self.bboxes.len();
        let rot_angle = 360.0 / symm.num_lines as f32;
        match symm.kind {
            0 => {
                let (xo, yo) = self.symmetry.matrices[0].point(x, y);
                self.draw_dab_internal(
                    xo,
                    yo,
                    radius,
                    color_r,
                    color_g,
                    color_b,
                    opaque,
                    hardness,
                    softness,
                    color_a,
                    aspect_ratio,
                    -2.0 * (90.0 + symm.angle) - angle,
                    lock_alpha,
                    colorize,
                    posterize,
                    posterize_num,
                    paint,
                    1,
                );
            }
            1 => {
                let (xo, yo) = self.symmetry.matrices[0].point(x, y);
                self.draw_dab_internal(
                    xo,
                    yo,
                    radius,
                    color_r,
                    color_g,
                    color_b,
                    opaque,
                    hardness,
                    softness,
                    color_a,
                    aspect_ratio,
                    -2.0 * symm.angle - angle,
                    lock_alpha,
                    colorize,
                    posterize,
                    posterize_num,
                    paint,
                    1,
                );
            }
            2 => {
                let (xo, yo) = self.symmetry.matrices[0].point(x, y);
                self.draw_dab_internal(
                    xo,
                    yo,
                    radius,
                    color_r,
                    color_g,
                    color_b,
                    opaque,
                    hardness,
                    softness,
                    color_a,
                    aspect_ratio,
                    -2.0 * symm.angle - angle,
                    lock_alpha,
                    colorize,
                    posterize,
                    posterize_num,
                    paint,
                    1,
                );
                let (xo, yo) = self.symmetry.matrices[1].point(x, y);
                self.draw_dab_internal(
                    xo,
                    yo,
                    radius,
                    color_r,
                    color_g,
                    color_b,
                    opaque,
                    hardness,
                    softness,
                    color_a,
                    aspect_ratio,
                    angle,
                    lock_alpha,
                    colorize,
                    posterize,
                    posterize_num,
                    paint,
                    2,
                );
                let (xo, yo) = self.symmetry.matrices[2].point(x, y);
                self.draw_dab_internal(
                    xo,
                    yo,
                    radius,
                    color_r,
                    color_g,
                    color_b,
                    opaque,
                    hardness,
                    softness,
                    color_a,
                    aspect_ratio,
                    -2.0 * symm.angle - angle,
                    lock_alpha,
                    colorize,
                    posterize,
                    posterize_num,
                    paint,
                    3,
                );
            }
            4 | 3 => {
                let snowflake = symm.kind == 4;
                if snowflake {
                    let offset = (num_bboxes / 2).min(symm.num_lines as usize);
                    let dabs_per_bbox = 1f32.max(symm.num_lines as f32 * 2.0 / num_bboxes as f32);
                    let base_idx = (symm.num_lines - 1) as usize;
                    let base_angle = -2.0 * symm.angle - angle;
                    for dab_count in 0..symm.num_lines as usize {
                        let bbox_idx = offset
                            + ((dab_count as f32 / dabs_per_bbox).round() as usize)
                                .min(num_bboxes - 1);
                        let (xo, yo) = self.symmetry.matrices[base_idx + dab_count].point(x, y);
                        self.draw_dab_internal(
                            xo,
                            yo,
                            radius,
                            color_r,
                            color_g,
                            color_b,
                            opaque,
                            hardness,
                            softness,
                            color_a,
                            aspect_ratio,
                            base_angle - dab_count as f32 * rot_angle,
                            lock_alpha,
                            colorize,
                            posterize,
                            posterize_num,
                            paint,
                            bbox_idx,
                        );
                    }
                }
                let dabs_per_bbox = 1f32.max(
                    (symm.num_lines * if snowflake { 2 } else { 1 }) as f32 / num_bboxes as f32,
                );
                for dab_count in 1..symm.num_lines as usize {
                    let bbox_index =
                        ((dab_count as f32 / dabs_per_bbox).round() as usize).min(num_bboxes - 1);
                    let (xo, yo) = self.symmetry.matrices[dab_count - 1].point(x, y);
                    self.draw_dab_internal(
                        xo,
                        yo,
                        radius,
                        color_r,
                        color_g,
                        color_b,
                        opaque,
                        hardness,
                        softness,
                        color_a,
                        aspect_ratio,
                        angle - dab_count as f32 * rot_angle,
                        lock_alpha,
                        colorize,
                        posterize,
                        posterize_num,
                        paint,
                        bbox_index,
                    );
                }
            }
            _ => {}
        }
        // Symmetry pass ran: upgrade the count (upstream values: 2 for a
        // single mirror axis, 4 for both, `num_lines` rotational,
        // `num_lines*2` snowflake).
        let num_bboxes_used = match symm.kind {
            0 | 1 => 2,
            2 => 4,
            3 => symm.num_lines as usize,
            4 => (symm.num_lines as usize).saturating_mul(2),
            _ => 1,
        };
        self.num_bboxes_dirtied = num_bboxes_used.min(MAX_DIRTY_RECTS);
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
        let sample_interval: u16 = if radius <= 2.0 {
            1
        } else {
            (radius * 7.0) as u16
        };
        let random_sample_rate = 1.0 / (7.0 * radius);

        let mut tile_copy = std::mem::take(&mut self.smudge_tile);
        for ty in ty1..=ty2 {
            for tx in tx1..=tx2 {
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
                if paint < 0.0 {
                    // The legacy path reads each covered pixel, so retain its
                    // eager tile drain. Spectral sampling reads few pixels and
                    // evaluates queued operations only at those positions.
                    self.process_tile(tx, ty);
                    const ZERO_TILE: [u16; TILE_PX] = [0; TILE_PX];
                    tile_copy.clear();
                    match self.get_tile(tx, ty) {
                        Some(t) => tile_copy.extend_from_slice(t),
                        None => tile_copy.extend_from_slice(&ZERO_TILE),
                    }
                    crate::brushmodes::get_color_accumulate(
                        &self.mask[..],
                        &tile_copy,
                        &mut sums,
                        paint,
                        sample_interval,
                        random_sample_rate,
                        self.random.as_mut(),
                    );
                    continue;
                }

                const ZERO_TILE: [u16; TILE_PX] = [0; TILE_PX];
                let slot = self.find_tile_slot(tx, ty);
                let base = slot
                    .and_then(|index| self.tiles[index].as_ref().map(|tile| &tile[..]))
                    .unwrap_or(&ZERO_TILE);
                self.smudge_op_indices.clear();
                for (index, entry) in self.ops[..self.op_len].iter().enumerate() {
                    if entry.tx == tx && entry.ty == ty {
                        self.smudge_op_indices.push(index);
                    }
                }
                let ops = &self.ops[..self.op_len];
                let op_indices = &self.smudge_op_indices;
                crate::brushmodes::get_color_accumulate_sampled(
                    &self.mask[..],
                    &mut sums,
                    paint,
                    sample_interval,
                    random_sample_rate,
                    self.random.as_mut(),
                    |pi| {
                        let p = pi * 4;
                        let mut pixel = [base[p], base[p + 1], base[p + 2], base[p + 3]];
                        for &index in op_indices {
                            crate::surface::process_op_pixel(
                                &mut pixel,
                                tx,
                                ty,
                                pi,
                                &ops[index].op,
                            );
                        }
                        pixel
                    },
                );
            }
        }
        self.smudge_tile = tile_copy;
        // The C normalization (mypaint-tiled-surface.c get_color tail):
        // legacy sampling divides by the mask weight; the spectral path's
        // sum is already a 0..1 reflectance and must not be divided.
        let weight = sums.weight.max(f32::MIN_POSITIVE);
        let sum_a = sums.a / weight;
        let color_a = sum_a.clamp(0.0, 1.0);
        if sum_a > 0.0 {
            let demul = if paint < 0.0 { sum_a } else { 1.0 };
            [
                (sums.r / demul).clamp(0.0, 1.0),
                (sums.g / demul).clamp(0.0, 1.0),
                (sums.b / demul).clamp(0.0, 1.0),
                color_a,
            ]
        } else {
            [0.0, 1.0, 0.0, color_a]
        }
    }

    pub fn display_dirty_count(&self) -> usize {
        self.display_dirty_slots.len()
    }

    pub fn display_dirty_info(&self, index: usize) -> Option<TilePos> {
        let slot = *self.display_dirty_slots.get(index)?;
        Some(TilePos {
            tx: self.tile_tx[slot],
            ty: self.tile_ty[slot],
        })
    }

    pub fn clear_display_dirty(&mut self) {
        for &slot in &self.display_dirty_slots {
            self.display_dirty[slot] = false;
        }
        self.display_dirty_slots.clear();
    }

    /// `web_surface_clear`.
    pub fn clear_tiles(&mut self) {
        self.tile_budget
            .used
            .set(self.tile_budget.used().saturating_sub(self.tiles.len()));
        self.tiles.clear();
        self.tile_tx.clear();
        self.tile_ty.clear();
        self.slot_used.fill(false);
        self.display_dirty.fill(false);
        self.display_dirty_slots.clear();
        self.storage_dirty.fill(false);
        self.capture_marks.fill(0);
        self.null_tile.fill(0);
    }

    /// `web_surface_set_symmetry`.
    pub fn set_symmetry(
        &mut self,
        active: bool,
        center_x: f32,
        center_y: f32,
        angle: f32,
        symmetry_type: i32,
        lines: i32,
    ) {
        crate::symmetry::symmetry_set_pending(
            &mut self.symmetry,
            active,
            center_x,
            center_y,
            angle,
            symmetry_type,
            lines,
        );
    }

    pub fn used_tile_info(&self, index: usize) -> Option<TilePos> {
        Some(TilePos {
            tx: *self.tile_tx.get(index)?,
            ty: *self.tile_ty.get(index)?,
        })
    }

    pub fn used_tile(&self, index: usize) -> Option<&[u16; TILE_PX]> {
        self.tiles.get(index).and_then(|t| t.as_ref()).map(|b| &**b)
    }

    pub fn used_tile_is_storage_dirty(&self, index: usize) -> bool {
        self.storage_dirty.get(index).copied().unwrap_or(false)
    }
}

impl Drop for WebSurface {
    fn drop(&mut self) {
        self.tile_budget
            .used
            .set(self.tile_budget.used().saturating_sub(self.tiles.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_atomic_reports_dirty_roi() {
        let _guard = JOB_TEST_LOCK.lock().unwrap();
        let mut surface = WebSurface::new(4096, 4096).unwrap();
        surface.begin_atomic();
        assert!(surface.draw_dab(
            100.0, 100.0, 10.0, 1.0, 0.0, 0.0, 1.0, 0.8, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            0.0,
        ));
        let (roi, _jobs) = surface.end_atomic();
        assert_eq!(roi.len(), 1);
        let r = &roi[0];
        assert_eq!(r.x, 89);
        assert_eq!(r.y, 89);
        assert_eq!(r.width, 23);
        assert_eq!(r.height, 23);
        // A second atomic must not merge with the stale bbox.
        surface.begin_atomic();
        assert!(surface.draw_dab(
            300.0, 300.0, 5.0, 1.0, 0.0, 0.0, 1.0, 0.8, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            0.0,
        ));
        let (roi, _jobs) = surface.end_atomic();
        assert_eq!(roi.len(), 1);
        assert_eq!(roi[0].x, 294);
    }

    #[test]
    fn spectral_sampling_matches_an_eager_tile_drain() {
        let _guard = JOB_TEST_LOCK.lock().unwrap();
        fn queue(surface: &mut WebSurface) {
            surface.begin_atomic();
            for i in 0..6 {
                assert!(surface.draw_dab(
                    54.25 + i as f32 * 3.5,
                    62.75 - i as f32 * 1.25,
                    22.0,
                    0.73,
                    0.21,
                    0.84,
                    0.82,
                    0.61,
                    0.12,
                    0.74,
                    1.4,
                    17.0,
                    0.0,
                    0.0,
                    0.0,
                    4.0,
                    1.0,
                ));
            }
        }

        let mut eager = WebSurface::new(256, 256).unwrap();
        let mut sparse = WebSurface::new(256, 256).unwrap();
        queue(&mut eager);
        queue(&mut sparse);
        let _ = eager.end_atomic();

        let eager_color = eager.get_color(62.5, 58.0, 24.0, 1.0);
        let sparse_color = sparse.get_color(62.5, 58.0, 24.0, 1.0);
        assert_eq!(
            eager_color.map(f32::to_bits),
            sparse_color.map(f32::to_bits),
        );
        assert!(
            sparse.op_len > 0,
            "sampling must not drain queued operations"
        );

        let _ = sparse.end_atomic();
        assert_eq!(eager.tile_tx, sparse.tile_tx);
        assert_eq!(eager.tile_ty, sparse.tile_ty);
        for (a, b) in eager.tiles.iter().zip(&sparse.tiles) {
            assert_eq!(a.as_deref(), b.as_deref());
        }
    }

    #[test]
    fn resumable_jobs_match_full_jobs() {
        let _guard = JOB_TEST_LOCK.lock().unwrap();
        fn queue(surface: &mut WebSurface) {
            surface.begin_atomic();
            for i in 0..4 {
                assert!(surface.draw_dab(
                    58.0 + i as f32 * 2.0,
                    57.0 + i as f32,
                    24.0,
                    0.8,
                    0.3,
                    0.6,
                    0.9,
                    0.65,
                    0.1,
                    0.8,
                    1.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    4.0,
                    1.0,
                ));
            }
        }

        let mut full = WebSurface::new(256, 256).unwrap();
        let mut resumed = WebSurface::new(256, 256).unwrap();
        queue(&mut full);
        queue(&mut resumed);
        let _ = full.end_atomic();
        let jobs = resumed.end_atomic_prepare();
        for job in 0..jobs {
            while !process_job_work(job as i32, 0, 7) {}
        }
        assert_eq!(full.tile_tx, resumed.tile_tx);
        assert_eq!(full.tile_ty, resumed.tile_ty);
        for (a, b) in full.tiles.iter().zip(&resumed.tiles) {
            assert_eq!(a.as_deref(), b.as_deref());
        }
    }

    #[test]
    fn failed_tile_allocation_publishes_no_job() {
        let _guard = JOB_TEST_LOCK.lock().unwrap();
        let budget = Rc::new(TileBudget::new(1, 1));
        budget.used.set(1);
        let mut surface = WebSurface::new_with_budget(128, 128, budget).unwrap();
        surface.begin_atomic();
        assert!(surface.draw_dab(
            32.0, 32.0, 10.0, 1.0, 0.0, 0.0, 1.0, 0.8, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
        ));
        assert_eq!(surface.end_atomic_prepare(), 0);
    }

    #[test]
    fn off_canvas_dabs_allocate_no_tiles() {
        let _guard = JOB_TEST_LOCK.lock().unwrap();
        let mut surface = WebSurface::new(4096, 4096).unwrap();
        surface.begin_atomic();
        // Fully outside (left/above the canvas).
        assert!(!surface.draw_dab(
            -100.0, -100.0, 10.0, 1.0, 0.0, 0.0, 1.0, 0.8, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            0.0,
        ));
        // Straddling the top-left corner: queues only the in-grid part.
        assert!(surface.draw_dab(
            4.0, 4.0, 30.0, 1.0, 0.0, 0.0, 1.0, 0.8, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
        ));
        let (_roi, jobs) = surface.end_atomic();
        assert_eq!(jobs, 1); // only tile (0,0)
        assert_eq!(surface.used_tile_count(), 1);
        assert!(surface.has_tile(0, 0));
        assert!(!surface.has_tile(-1, -1));
        assert_eq!(surface.queue_failed(), 0);
    }

    #[test]
    fn resident_tile_budget_is_shared_and_released() {
        let budget = Rc::new(TileBudget::new(2, 2));
        let mut first = WebSurface::new_with_budget(64, 64, Rc::clone(&budget)).unwrap();
        let mut second = WebSurface::new_with_budget(64, 64, Rc::clone(&budget)).unwrap();
        assert!(first.get_or_create_tile_mut(0, 0).is_some());
        assert!(second.get_or_create_tile_mut(0, 0).is_some());
        assert_eq!(budget.used(), 2);
        first.clear_tiles();
        assert_eq!(budget.used(), 1);
        drop(second);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn resident_tile_budget_rejects_before_allocation() {
        let budget = Rc::new(TileBudget::new(
            INITIAL_RESIDENT_TILES,
            INITIAL_RESIDENT_TILES,
        ));
        budget.used.set(INITIAL_RESIDENT_TILES);
        let mut surface = WebSurface::new_with_budget(16384, 16384, budget).unwrap();
        assert_eq!(surface.tiles.capacity(), INITIAL_RESIDENT_TILES);
        assert_eq!(surface.display_dirty.len(), INITIAL_RESIDENT_TILES);
        assert!(surface.get_or_create_tile_mut(0, 0).is_none());
        assert!(surface.take_capacity_error());
    }

    #[test]
    fn resident_tile_limit_and_storage_state_grow_on_demand() {
        let budget = Rc::new(TileBudget::new(1, 3));
        let mut surface = WebSurface::new_with_budget(256, 256, Rc::clone(&budget)).unwrap();
        assert!(surface.get_or_create_tile_mut(0, 0).is_some());
        assert!(surface.used_tile_is_storage_dirty(0));
        assert!(surface.get_or_create_tile_mut(1, 0).is_none());
        assert!(budget.set_limit(3));
        assert!(surface.write_rgba16_tile(1, 0, &[9; TILE_PX]));
        assert!(!surface.used_tile_is_storage_dirty(1));
        assert!(surface.write_rgba16_tile_modified(1, 0, &[8; TILE_PX]));
        assert!(surface.used_tile_is_storage_dirty(1));
        assert!(surface.get_or_create_tile_mut(1, 0).is_some());
        assert!(surface.used_tile_is_storage_dirty(1));
        assert!(surface.get_or_create_tile_mut(2, 0).is_some());
        assert_eq!(budget.used(), 3);
        assert_eq!(surface.display_dirty.len(), 3);
    }

    #[test]
    fn resident_tile_remove_rebuilds_hash_and_releases_budget() {
        let budget = Rc::new(TileBudget::new(INITIAL_RESIDENT_TILES, 8192));
        let mut surface = WebSurface::new_with_budget(4096, 4096, Rc::clone(&budget)).unwrap();
        *surface
            .get_or_create_tile_mut(1, 2)
            .unwrap()
            .first_mut()
            .unwrap() = 77;
        *surface
            .get_or_create_tile_mut(3, 4)
            .unwrap()
            .first_mut()
            .unwrap() = 88;
        assert_eq!(budget.used(), 2);
        surface.mark_display_dirty(0);
        surface.mark_display_dirty(1);
        assert!(surface.remove_tile(1, 2));
        assert!(!surface.has_tile(1, 2));
        assert_eq!(surface.get_tile(3, 4).unwrap()[0], 88);
        assert_eq!(surface.display_dirty_count(), 1);
        assert_eq!(
            surface.display_dirty_info(0),
            Some(TilePos { tx: 3, ty: 4 })
        );
        assert_eq!(budget.used(), 1);
        assert!(surface.write_rgba16_tile(1, 2, &[99; TILE_PX]));
        assert_eq!(surface.get_tile(1, 2).unwrap()[0], 99);
        assert_eq!(budget.used(), 2);
        assert!(!surface.used_tile_is_storage_dirty(1));
    }

    #[test]
    fn captured_tiles_are_identified_for_paging_protection() {
        let mut surface = WebSurface::new(4096, 4096).unwrap();
        assert!(!surface.tile_is_captured(1, 2));
        surface.set_capture_enabled(true);
        surface.get_or_create_tile_mut(1, 2).unwrap();
        surface.capture_first_write(0, 1, 2);
        assert!(surface.tile_is_captured(1, 2));
        assert!(!surface.tile_is_captured(3, 4));
    }

    #[test]
    fn failed_history_capture_does_not_change_pixels() {
        let mut surface = WebSurface::new(64, 64).unwrap();
        surface.capture_byte_limit = 0;
        surface.set_capture_enabled(true);
        surface.begin_atomic();
        assert!(surface.draw_dab(
            32.0, 32.0, 10.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
        ));
        surface.end_atomic();
        assert!(
            surface
                .get_tile(0, 0)
                .unwrap()
                .iter()
                .all(|value| *value == 0)
        );
        assert!(surface.take_capture_error());
    }

    #[test]
    fn external_history_has_no_internal_byte_limit() {
        let mut surface = WebSurface::new(4096, 4096).unwrap();
        surface.set_external_history(true);
        surface.set_capture_enabled(true);
        surface.get_or_create_tile_mut(1, 2).unwrap();
        surface.captured_bytes = crate::app::HISTORY_BYTE_BUDGET / 2;
        surface.capture_first_write(0, 1, 2);
        assert_eq!(surface.captured.len(), 1);
        assert!(!surface.capture_overflow);
    }

    #[test]
    fn job_groups_leave_no_operation_for_the_next_stroke() {
        let _guard = JOB_TEST_LOCK.lock().unwrap();
        let budget = Rc::new(TileBudget::new(1, 1));
        let width = ((MAX_JOBS + 1) * TILE) as i32;
        let mut surface = WebSurface::new_with_budget(width, 64, budget).unwrap();
        surface.begin_atomic();
        surface.op_len = MAX_JOBS + 1;
        let mut op = NULL_DAB_OP;
        op.radius = 1.0;
        op.aspect_ratio = 1.0;
        op.opaque = 1.0;
        op.hardness = 1.0;
        op.normal = 1.0;
        for index in 0..surface.op_len {
            surface.ops[index] = QueuedOp {
                tx: index as i32,
                ty: 0,
                op,
            };
        }
        let first = surface.end_atomic_prepare();
        assert_eq!(first, 1);
        process_job(0, 0);
        assert_eq!(surface.end_atomic_prepare(), 0);
        assert!(!surface.has_pending_ops());
    }

    #[test]
    fn operation_queue_reports_overflow() {
        let mut surface = WebSurface::new(4096, 4096).unwrap();
        for _ in 0..2_000 {
            surface.draw_dab(
                100.0, 100.0, 60.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0,
                1.0, 0.0,
            );
        }
        assert!(surface.queue_failed() > 0);
        assert!(surface.take_queue_error());
        assert_eq!(surface.queue_failed(), 0);
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
            x,
            y,
            radius,
            color_r,
            color_g,
            color_b,
            opaque,
            hardness,
            softness,
            color_a,
            aspect_ratio,
            angle,
            lock_alpha,
            colorize,
            posterize,
            posterize_num,
            paint,
        )
    }

    fn get_color(&mut self, x: f32, y: f32, radius: f32, paint: f32) -> [f32; 4] {
        self.get_color(x, y, radius, paint)
    }
}

/// The op arena address (the pool workers deserialize received ops here;
/// `paint_get_job_info` publishes offsets into it).
pub fn op_arena_ptr() -> usize {
    OP_ARENA.0.get() as usize
}

/// One job's fields for the host's serialization path.
pub fn job_info(job_index: i32, out: &mut [usize]) {
    let j = job_index.max(0) as usize;
    if j >= MAX_JOBS || out.len() < 5 {
        return;
    }
    let words = &JOB_TABLE[j * JOB_WORDS..(j + 1) * JOB_WORDS];
    out[0] = words[JOB_OPS_OFF].load(Ordering::Acquire);
    out[1] = words[JOB_OP_COUNT].load(Ordering::Acquire);
    out[2] = words[JOB_TILE_ADDR].load(Ordering::Acquire);
    out[3] = words[JOB_TX].load(Ordering::Acquire);
    out[4] = words[JOB_TY].load(Ordering::Acquire);
}

/// Blend one prepared job. Callable from any module instance sharing the
/// linear memory: reads the job table + op arena statics and the tile, and
/// touches only this worker's mask/scratch arenas. One tile is owned by
/// exactly one job, so no two workers write the same tile.
pub fn process_job(job_index: i32, worker_id: usize) {
    let j = job_index.max(0) as usize;
    if j >= MAX_JOBS {
        return;
    }
    let words = &JOB_TABLE[j * JOB_WORDS..(j + 1) * JOB_WORDS];
    let ops_off = words[JOB_OPS_OFF].load(Ordering::Acquire) as usize;
    let op_count = words[JOB_OP_COUNT].load(Ordering::Relaxed) as usize;
    let tile_addr = words[JOB_TILE_ADDR].load(Ordering::Relaxed) as usize;
    let tx = words[JOB_TX].load(Ordering::Relaxed) as i32;
    let ty = words[JOB_TY].load(Ordering::Relaxed) as i32;
    if op_count == 0 || tile_addr == 0 {
        return;
    }
    let arena = unsafe { &*OP_ARENA.0.get() };
    let ops = &arena[ops_off..ops_off + op_count];
    let tile = unsafe { std::slice::from_raw_parts_mut(tile_addr as *mut u16, TILE_PX) };
    let masks = unsafe { &mut *WORKER_MASKS.0.get() };
    let scratches = unsafe { &mut *WORKER_SCRATCH.0.get() };
    let wi = (worker_id.min(JOB_WORKERS - 1)) * MASK_LEN;
    let (m, _) = masks.split_at_mut(wi + MASK_LEN);
    let (s2, _) = scratches.split_at_mut(wi + MASK_LEN);
    let mask = &mut m[wi..];
    let scratch = &mut s2[wi..];
    for op in ops {
        crate::surface::process_op(tile, mask, tx, ty, op, scratch);
    }
    JOB_OP_PROGRESS[j].store(op_count, Ordering::Relaxed);
    JOB_ROW_PROGRESS[j].store(0, Ordering::Release);
}

/// Blend at most `row_budget` operation rows and retain the job position.
/// Returns true when the job is complete.
pub fn process_job_work(job_index: i32, worker_id: usize, row_budget: usize) -> bool {
    let j = job_index.max(0) as usize;
    if j >= MAX_JOBS {
        return true;
    }
    let words = &JOB_TABLE[j * JOB_WORDS..(j + 1) * JOB_WORDS];
    let ops_off = words[JOB_OPS_OFF].load(Ordering::Acquire);
    let op_count = words[JOB_OP_COUNT].load(Ordering::Relaxed);
    let tile_addr = words[JOB_TILE_ADDR].load(Ordering::Relaxed);
    let tx = words[JOB_TX].load(Ordering::Relaxed) as i32;
    let ty = words[JOB_TY].load(Ordering::Relaxed) as i32;
    if op_count == 0 || tile_addr == 0 {
        return true;
    }

    let arena = unsafe { &*OP_ARENA.0.get() };
    let tile = unsafe { std::slice::from_raw_parts_mut(tile_addr as *mut u16, TILE_PX) };
    let masks = unsafe { &mut *WORKER_MASKS.0.get() };
    let scratches = unsafe { &mut *WORKER_SCRATCH.0.get() };
    let wi = worker_id.min(JOB_WORKERS - 1) * MASK_LEN;
    let mask = &mut masks[wi..wi + MASK_LEN];
    let scratch = &mut scratches[wi..wi + MASK_LEN];
    let mut op_index = JOB_OP_PROGRESS[j].load(Ordering::Acquire);
    let mut row = JOB_ROW_PROGRESS[j].load(Ordering::Relaxed);
    let mut budget = row_budget.max(1);

    while op_index < op_count && budget > 0 {
        let rows = budget.min(TILE - row);
        crate::surface::process_op_rows(
            tile,
            mask,
            tx,
            ty,
            &arena[ops_off + op_index],
            scratch,
            row,
            row + rows,
        );
        row += rows;
        budget -= rows;
        if row == TILE {
            op_index += 1;
            row = 0;
        }
    }
    JOB_OP_PROGRESS[j].store(op_index, Ordering::Relaxed);
    JOB_ROW_PROGRESS[j].store(row, Ordering::Release);
    op_index == op_count
}
