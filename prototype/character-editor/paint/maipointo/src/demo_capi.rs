//! The demo's `_paint_*` C ABI — the pure-Rust wasm module surface, replacing
//! the Emscripten `main.c` module one-for-one. The TS worker keeps calling
//! the same names (`_init`, `_stroke_to`, `_paint_*`, ...); the loader in
//! the worker wraps these exports and exposes `HEAPU8`/`HEAP32` from the
//! wasm memory.

#![allow(clippy::too_many_arguments)]

use crate::app::{PaintApp, WEB_MAX_GROUPS};
use crate::compositor::BLEND_MODE_COUNT;
use crate::compositor::BlendMode;
use std::cell::RefCell;

pub(crate) fn maipo_load_brush_json(brush: &mut crate::brush::Brush, text: &str) -> bool {
    crate::capi_json::load(brush, text)
}

thread_local! {
    static APP: RefCell<Option<PaintApp>> = const { RefCell::new(None) };
}

fn with_app<R: Default>(f: impl FnOnce(&mut PaintApp) -> R) -> R {
    install_panic_hook();
    APP.with(|slot| {
        let Ok(mut borrow) = slot.try_borrow_mut() else {
            return R::default();
        };
        let app = borrow.get_or_insert_with(|| PaintApp::new(2048, 2048).unwrap());
        // A panic unwinds (wasm exception handling), drops this RefMut and
        // leaves the module usable; the error code reports the failure.
        // Without this, an abort leaks the borrow and bricks every later
        // call (the engine must never trap into a dead instance).
        let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(app))) {
            Ok(r) => r,
            Err(_) => {
                app.error_code = 1;
                R::default()
            }
        };
        let active_layer = app.active_layer();
        let capacity_failed = match app.layers.get_mut(active_layer) {
            Some(surface) => surface.take_capacity_error(),
            None => false,
        };
        if capacity_failed {
            app.error_code = 1;
        }
        result
    })
}

const POOL_SLOTS: usize = 64;
const POOL_SLOT_BYTES: usize = 32768; // largest cooked .myb JSON is ~25 KiB
static POOL: [u8; POOL_SLOTS * POOL_SLOT_BYTES] = [0; POOL_SLOTS * POOL_SLOT_BYTES];
static POOL_FREE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// Bounded scratch pool: the worker mallocs short-lived strings/out-params
/// and frees them immediately, so a fixed 64x4 KiB slot pool with a free
/// bitmask never exhausts. Returns null for oversized or exhausted requests.
#[unsafe(no_mangle)]
pub extern "C" fn _malloc(n: usize) -> *mut u8 {
    use std::sync::atomic::Ordering;
    if n == 0 || n > POOL_SLOT_BYTES {
        return std::ptr::null_mut();
    }
    loop {
        let free = POOL_FREE.load(Ordering::Relaxed);
        if free == 0 {
            return std::ptr::null_mut();
        }
        let slot = free.trailing_zeros() as usize;
        if POOL_FREE
            .compare_exchange_weak(free, free & !(1u64 << slot), Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            return unsafe { POOL.as_ptr().add(slot * POOL_SLOT_BYTES) as *mut u8 };
        }
    }
}

/// Returns the slot to the pool. Out-of-pool pointers are ignored.
#[unsafe(no_mangle)]
pub extern "C" fn _free(ptr: *mut u8, _n: usize) {
    use std::sync::atomic::Ordering;
    let base = POOL.as_ptr() as usize;
    let addr = ptr as usize;
    if addr < base || addr >= base + POOL_SLOTS * POOL_SLOT_BYTES {
        return;
    }
    let slot = (addr - base) / POOL_SLOT_BYTES;
    POOL_FREE.fetch_or(1u64 << slot, Ordering::AcqRel);
}

/// # Safety
/// Called once by the worker before anything else.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init(width: i32, height: i32) -> i32 {
    APP.with(|slot| {
        *slot.borrow_mut() = PaintApp::new(width, height);
    });
    with_app(|app| {
        let _ = app;
        1
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn new_brush() {
    with_app(|app| {
        app.brush = Some(crate::brush::Brush::new());
        app.brush.as_mut().unwrap().from_defaults();
        app.brush.as_mut().unwrap().new_stroke();
    });
}

/// # Safety
/// `json` must be a NUL-terminated UTF-8 string inside the wasm memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn load_brush(json: *const i8) -> i32 {
    if json.is_null() {
        return 0;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(json) }
        .to_string_lossy()
        .into_owned();
    with_app(|app| {
        // Same as new_brush(), inlined: calling new_brush() here would
        // re-enter with_app while the RefCell is already borrowed.
        let mut brush = crate::brush::Brush::new();
        brush.from_defaults();
        brush.new_stroke();
        app.brush = Some(brush);
        let Some(brush) = app.brush.as_mut() else { return 0 };
        let loaded = maipo_load_brush_json(brush, &text);
        if loaded {
            brush.new_stroke();
        }
        loaded as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn reset_brush() {
    with_app(|app| {
        if let Some(brush) = app.brush.as_mut() {
            brush.request_reset();
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn begin_stroke(x: f32, y: f32, xtilt: f32, ytilt: f32,
    viewzoom: f32, viewrotation: f32, barrel_rotation: f32,
) {
    with_app(|app| app.begin_stroke(x, y, xtilt, ytilt, viewzoom, viewrotation, barrel_rotation));
}

#[unsafe(no_mangle)]
pub extern "C" fn stroke_to(x: f32, y: f32, pressure: f32, xtilt: f32, ytilt: f32,
    dtime: f64, viewzoom: f32, viewrotation: f32, barrel_rotation: f32,
    linear: i32,
) -> i32 {
    with_app(|app| {
        app.stroke_to(
            x, y, pressure, xtilt, ytilt, dtime, viewzoom, viewrotation,
            barrel_rotation, linear != 0,
        )
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn set_brush_base_value(cname: *const i8, value: f64) {
    if cname.is_null() {
        return;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(cname) }
        .to_string_lossy()
        .into_owned();
    with_app(|app| {
        if let Some(brush) = app.brush.as_mut() {
            if let Some(i) = crate::settings::setting_index_runtime(&name) {
                brush.set_base_value_at(i, value as f32);
            }
        }
    });
}

/// # Safety
/// `cname` must be a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_brush_base_value(cname: *const i8) -> f32 {
    if cname.is_null() {
        return 0.0;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(cname) }
        .to_string_lossy()
        .into_owned();
    with_app(|app| {
        match crate::settings::setting_index_runtime(&name) {
            Some(i) => {
                let Some(brush) = app.brush.as_ref() else { return 0.0 };
                crate::settings::SettingId::from_index(i)
                    .map(|id| brush.get_base_value(id))
                    .unwrap_or(0.0)
            }
            None => 0.0,
        }
    })
}

/// # Safety
/// `setting_name`/`input_name` must be NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_brush_mapping_n(setting_name: *const i8,
    input_name: *const i8, number_of_mapping_points: i32,
) {
    if setting_name.is_null() || input_name.is_null() || number_of_mapping_points < 0 {
        return;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(setting_name) }.to_string_lossy().into_owned();
    let i = unsafe { std::ffi::CStr::from_ptr(input_name) }.to_string_lossy().into_owned();
    with_app(|app| {
        if let Some(brush) = app.brush.as_mut() {
            if let (Some(si), Some(ii)) = (
                crate::settings::setting_index_runtime(&s),
                crate::settings::input_index_runtime(&i),
            ) {
                brush.set_mapping_n_at(si, ii, number_of_mapping_points as usize);
            }
        }
    });
}

/// # Safety
/// `setting_name`/`input_name` must be NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_brush_mapping_point(setting_name: *const i8,
    input_name: *const i8, index: i32, x: f32, y: f32,
) {
    if setting_name.is_null() || input_name.is_null() || index < 0 {
        return;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(setting_name) }.to_string_lossy().into_owned();
    let i = unsafe { std::ffi::CStr::from_ptr(input_name) }.to_string_lossy().into_owned();
    with_app(|app| {
        if let Some(brush) = app.brush.as_mut() {
            if let (Some(si), Some(ii)) = (
                crate::settings::setting_index_runtime(&s),
                crate::settings::input_index_runtime(&i),
            ) {
                brush.set_mapping_point_at(si, ii, index as usize, x, y);
            }
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_begin_atomic() {
    with_app(|app| app.active().begin_atomic());
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_end_atomic() -> i32 {
    with_app(|app| {
        let (roi, _jobs) = app.active().end_atomic();
        app.set_dirty_roi(roi);
        app.dirty_roi_len() as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_begin_batch() {
    with_app(|app| app.begin_batch());
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_end_batch() -> i32 {
    with_app(|app| {
        app.end_batch();
        app.dirty_roi_len() as i32
    })
}

/// Prepare the tile-parallel drain: serial bookkeeping + the shared job
/// table. Returns the job count; drain via `paint_claim_job` (any module
/// instance sharing the linear memory) + `paint_process_tile_job`, then
/// `paint_end_batch_finish` for the ROI bookkeeping.
#[unsafe(no_mangle)]
pub extern "C" fn paint_end_batch_parallel() -> i32 {
    with_app(|app| app.end_batch_parallel())
}

/// Claim the next unclaimed blend job (atomic; pool-safe). -1 = drained.
#[unsafe(no_mangle)]
pub extern "C" fn paint_claim_job() -> i32 {
    crate::web_surface::claim_job()
}

/// Mark one claimed job complete.
#[unsafe(no_mangle)]
pub extern "C" fn paint_complete_job() {
    crate::web_surface::complete_job();
}

/// Blend one claimed job. Runs on any module instance sharing the linear
/// memory; `worker_id` selects the private mask/scratch arenas.
/// # Safety
/// `job_index` must be within the published job table (0..count-1) and
/// `worker_id` below JOB_WORKERS; both are enforced by clamping.
#[unsafe(no_mangle)]
pub extern "C" fn paint_process_tile_job(job_index: i32, worker_id: i32) {
    crate::web_surface::process_job(job_index, worker_id.max(0) as usize);
}

/// The published job count of the current batch (the host's wait target).
#[unsafe(no_mangle)]
pub extern "C" fn paint_job_count() -> i32 {
    crate::web_surface::job_count()
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_is_batch_done() -> i32 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_end_batch_finish() -> i32 {
    with_app(|app| app.dirty_roi_len() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_continue_stroke_to() -> i32 {
    // The serial driver completes every stroke inside stroke_to.
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_has_stroke_continuation() -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_width() -> i32 {
    with_app(|app| app.active().width())
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_height() -> i32 {
    with_app(|app| app.active().height())
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_error_code() -> i32 {
    with_app(|app| app.error_code)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_clear_error() {
    with_app(|app| app.error_code = 0);
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_tiles_width() -> i32 {
    with_app(|app| app.active().tiles_width())
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_tiles_height() -> i32 {
    with_app(|app| app.active().tiles_height())
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_used_tile_count() -> i32 {
    with_app(|app| app.active().used_tile_count() as i32)
}

/// # Safety
/// The returned pointer aliases the active layer's tile buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn paint_get_tile_ptr(tx: i32, ty: i32) -> usize {
    with_app(|app| {
        let s = app.active();
        s.get_tile(tx, ty).map(|t| t.as_ptr() as usize).unwrap_or(0)
    })
}

/// # Safety
/// The returned pointer aliases the composite tile buffer (valid until the
/// next render call).
#[unsafe(no_mangle)]
pub extern "C" fn paint_render_tile_ptr(tx: i32, ty: i32) -> usize {
    with_app(|app| {
        let tile = app.render_tile(tx, ty);
        tile.as_ptr() as usize
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_eotf(eotf: f32) {
    with_app(|app| app.set_eotf(eotf));
}

/// # Safety
/// The returned pointer aliases the display tile buffer.
#[unsafe(no_mangle)]
pub extern "C" fn paint_render_rgba8_tile_ptr(tx: i32, ty: i32) -> usize {
    with_app(|app| {
        app.render_rgba8_tile(tx, ty).as_ptr() as usize
    })
}

/// # Safety
/// The returned pointer aliases the display tile buffer.
#[unsafe(no_mangle)]
pub extern "C" fn paint_render_layer_rgba8_tile_ptr(layer_id: i32, tx: i32, ty: i32) -> usize {
    with_app(|app| {
        if layer_id < 0 {
            return 0;
        }
        app.render_layer_rgba8_tile(layer_id as usize, tx, ty)
            .map(|t| t.as_ptr() as usize)
            .unwrap_or(0)
    })
}

/// # Safety
/// `source` must point to `64*64*4` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn paint_write_rgba8_tile(tx: i32, ty: i32, source: *const u8) -> i32 {
    if source.is_null() {
        return 0;
    }
    with_app(|app| {
        let bytes = unsafe { std::slice::from_raw_parts(source, 64 * 64 * 4) };
        app.write_rgba8_tile(tx, ty, bytes) as i32
    })
}

/// # Safety
/// The returned pointer aliases the display tile buffer.
#[unsafe(no_mangle)]
pub extern "C" fn paint_render_rgba8_mip_tile_ptr(tx: i32, ty: i32, level: i32) -> usize {
    with_app(|app| app.render_rgba8_mip_tile(tx, ty, level).as_ptr() as usize)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_region_has_paint(tx: i32, ty: i32, level: i32) -> i32 {
    with_app(|app| app.region_has_paint(tx, ty, level) as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_dirty_count() -> i32 {
    with_app(|app| app.dirty_roi_len() as i32)
}

/// # Safety
/// `out_rect` must point to 4 writable i32s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn paint_get_dirty_rect(index: i32, out_rect: *mut i32) {
    if out_rect.is_null() {
        return;
    }
    with_app(|app| {
        if let Some(r) = app.dirty_roi.get(index as usize) {
            unsafe {
                *out_rect.add(0) = r.x;
                *out_rect.add(1) = r.y;
                *out_rect.add(2) = r.width;
                *out_rect.add(3) = r.height;
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_dirty_tile_count() -> i32 {
    with_app(|app| app.active().display_dirty_count() as i32)
}

/// # Safety
/// `out_tile` must point to 2 writable i32s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn paint_get_dirty_tile_info(index: i32, out_tile: *mut i32) {
    if out_tile.is_null() {
        return;
    }
    with_app(|app| {
        if let Some(pos) = app.active().display_dirty_info(index as usize) {
            unsafe {
                *out_tile.add(0) = pos.tx;
                *out_tile.add(1) = pos.ty;
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_clear_dirty() {
    with_app(|app| {
        app.clear_dirty_roi();
        app.active().clear_display_dirty();
    })
}

/// # Safety
/// Nothing; plain math.
#[unsafe(no_mangle)]
pub extern "C" fn paint_set_background_color(r: f32, g: f32, b: f32) {
    with_app(|app| app.set_background_color(r, g, b));
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_clear_background() {
    with_app(|app| app.clear_background());
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_history_begin() {
    with_app(|app| app.history_begin());
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_history_commit() {
    with_app(|app| app.history_commit());
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_history_undo() -> i32 {
    with_app(|app| app.history_undo() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_history_redo() -> i32 {
    with_app(|app| app.history_redo() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_history_can_undo() -> i32 {
    with_app(|app| app.history_can_undo() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_history_can_redo() -> i32 {
    with_app(|app| app.history_can_redo() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_clear() {
    with_app(|app| app.clear());
}

/// # Safety
/// `out_rgba` must point to 4 writable f32s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn paint_pick_color(x: f32, y: f32, radius: f32,
    paint: f32, out_rgba: *mut f32,
) {
    if out_rgba.is_null() {
        return;
    }
    with_app(|app| {
        let [r, g, b, a] = app.pick_color(x, y, radius, paint);
        unsafe {
            *out_rgba.add(0) = r;
            *out_rgba.add(1) = g;
            *out_rgba.add(2) = b;
            *out_rgba.add(3) = a;
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_symmetry(active: i32, center_x: f32, center_y: f32,
    angle: f32, symmetry_type: i32, lines: i32,
) {
    with_app(|app| {
        app.set_symmetry(active != 0, center_x, center_y, angle, symmetry_type, lines);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_layer_count() -> i32 {
    with_app(|app| app.layer_count() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_active_layer() -> i32 {
    with_app(|app| app.active_layer() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_active_layer(layer_id: i32) -> i32 {
    with_app(|app| {
        if layer_id < 0 || layer_id as usize >= app.layer_count() {
            return 0;
        }
        app.set_active_layer(layer_id as usize)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_create_layer() -> i32 {
    with_app(|app| app.create_layer())
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_delete_layer(layer_id: i32) -> i32 {
    with_app(|app| {
        if layer_id < 0 {
            return 0;
        }
        app.delete_layer(layer_id as usize) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_layer_visible(layer_id: i32) -> i32 {
    with_app(|app| {
        let id = layer_id as usize;
        (id < app.layer_count() && app.layer_visible[id]) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_layer_visible(layer_id: i32, visible: i32) {
    with_app(|app| {
        let id = layer_id as usize;
        if id < app.layer_count() {
            app.layer_visible[id] = visible != 0;
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_layer_opacity(layer_id: i32, opacity: f32) {
    with_app(|app| {
        let id = layer_id as usize;
        if id < app.layer_count() {
            app.layer_opacity[id] = opacity.clamp(0.0, 1.0);
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_layer_opacity(layer_id: i32) -> f32 {
    with_app(|app| {
        let id = layer_id as usize;
        if id < app.layer_count() {
            app.layer_opacity[id]
        } else {
            0.0
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_layer_mode(layer_id: i32) -> i32 {
    with_app(|app| {
        let id = layer_id as usize;
        if id < app.layer_count() {
            app.layer_mode[id] as i32
        } else {
            BlendMode::Normal as i32
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_layer_mode(layer_id: i32, mode: i32) {
    with_app(|app| {
        let id = layer_id as usize;
        if id < app.layer_count() && (0..BLEND_MODE_COUNT as i32).contains(&mode) {
            app.set_layer_mode(id, BlendMode::from_int(mode));
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_layer_group(layer_id: i32) -> i32 {
    with_app(|app| {
        let id = layer_id as usize;
        if id < app.layer_count() {
            app.layer_parent[id]
        } else {
            -1
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_layer_group(layer_id: i32, group_id: i32) -> i32 {
    with_app(|app| {
        if layer_id < 0 {
            return 0;
        }
        app.set_layer_group(layer_id as usize, group_id) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_move_layer(layer_id: i32, direction: i32) -> i32 {
    with_app(|app| {
        if layer_id < 0 {
            return 0;
        }
        app.move_layer(layer_id as usize, direction) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_group_count() -> i32 {
    with_app(|app| app.group_count() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_group_alive(group_id: i32) -> i32 {
    with_app(|app| {
        let id = group_id as usize;
        (id < WEB_MAX_GROUPS && app.group_alive[id]) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_group_parent(group_id: i32) -> i32 {
    with_app(|app| {
        let id = group_id as usize;
        if id < WEB_MAX_GROUPS && app.group_alive[id] {
            app.group_parent[id]
        } else {
            -1
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_create_group() -> i32 {
    with_app(|app| app.create_group())
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_delete_group(group_id: i32) -> i32 {
    with_app(|app| {
        if group_id < 0 {
            return 0;
        }
        app.delete_group(group_id as usize) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_group_parent(group_id: i32, parent_id: i32) -> i32 {
    with_app(|app| {
        if group_id < 0 {
            return 0;
        }
        app.set_group_parent(group_id as usize, parent_id) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_group_visible(group_id: i32) -> i32 {
    with_app(|app| {
        let id = group_id as usize;
        (id < WEB_MAX_GROUPS && app.group_alive[id] && app.group_visible[id]) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_group_visible(group_id: i32, visible: i32) {
    with_app(|app| {
        let id = group_id as usize;
        if id < WEB_MAX_GROUPS && app.group_alive[id] {
            app.group_visible[id] = visible != 0;
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_group_opacity(group_id: i32) -> f32 {
    with_app(|app| {
        let id = group_id as usize;
        if id < WEB_MAX_GROUPS && app.group_alive[id] {
            app.group_opacity[id]
        } else {
            0.0
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_group_opacity(group_id: i32, opacity: f32) {
    with_app(|app| {
        let id = group_id as usize;
        if id < WEB_MAX_GROUPS && app.group_alive[id] {
            app.group_opacity[id] = opacity.clamp(0.0, 1.0);
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_group_mode(group_id: i32) -> i32 {
    with_app(|app| {
        let id = group_id as usize;
        if id < WEB_MAX_GROUPS && app.group_alive[id] {
            app.group_mode[id] as i32
        } else {
            BlendMode::Normal as i32
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_group_mode(group_id: i32, mode: i32) -> i32 {
    with_app(|app| {
        let id = group_id as usize;
        if id < WEB_MAX_GROUPS && app.group_alive[id] {
            app.group_mode[id] = BlendMode::from_int(mode);
            1
        } else {
            0
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_group_pass_through(group_id: i32) -> i32 {
    with_app(|app| {
        let id = group_id as usize;
        (id < WEB_MAX_GROUPS && app.group_alive[id] && app.group_pass_through[id]) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_group_pass_through(group_id: i32, value: i32) {
    with_app(|app| {
        let id = group_id as usize;
        if id < WEB_MAX_GROUPS && app.group_alive[id] {
            app.group_pass_through[id] = value != 0;
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_get_group_isolated(group_id: i32) -> i32 {
    with_app(|app| {
        let id = group_id as usize;
        (id < WEB_MAX_GROUPS && app.group_alive[id] && app.group_isolated[id]) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_set_group_isolated(group_id: i32, value: i32) {
    with_app(|app| {
        let id = group_id as usize;
        if id < WEB_MAX_GROUPS && app.group_alive[id] {
            app.group_isolated[id] = value != 0;
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_move_group(group_id: i32, direction: i32) -> i32 {
    with_app(|app| {
        if group_id < 0 {
            return 0;
        }
        app.move_group(group_id as usize, direction) as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn paint_destroy() {
    APP.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

/// The job-table header address (a fixed wasm static; the same address in
/// every instance sharing the linear memory). The pool workers build their
/// Atomics views here.
#[unsafe(no_mangle)]
pub extern "C" fn paint_header_ptr() -> i32 {
    crate::web_surface::job_header_ptr() as i32
}

/// The completed-job count of the current batch (the host's wait progress).
#[unsafe(no_mangle)]
pub extern "C" fn paint_completed_jobs() -> i32 {
    crate::web_surface::completed_jobs()
}

/// The `DrawDabOp` byte size (the host serializes job ops by raw bytes).
#[unsafe(no_mangle)]
pub extern "C" fn paint_draw_dab_op_size() -> i32 {
    std::mem::size_of::<crate::surface::DrawDabOp>() as i32
}

/// Pool-local tile blend: the tile-pool worker received the ops bytes and
/// the tile bytes in ITS OWN linear memory (each pool worker owns an
/// isolated module instance + memory, so two Rust allocators never share
/// state). Blends the ops into the tile in FIFO order.
///
/// # Safety
/// The pointers must reference the caller's own memory arenas sized
/// `op_count * size_of::<DrawDabOp>()` and 64*64*4*2 bytes respectively.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn paint_blend_tile_ops(
    ops_ptr: *const crate::surface::DrawDabOp,
    op_count: i32,
    tile_ptr: *mut u16,
    tx: i32,
    ty: i32,
) {
    if ops_ptr.is_null() || tile_ptr.is_null() || op_count <= 0 {
        return;
    }
    let ops = std::slice::from_raw_parts(ops_ptr, op_count as usize);
    let tile = std::slice::from_raw_parts_mut(tile_ptr, 64 * 64 * 4);
    let mut mask = [0u16; crate::surface::MASK_LEN];
    let mut scratch = [0.0f32; crate::surface::MASK_LEN];
    for op in ops {
        crate::surface::process_op(tile, &mut mask, tx, ty, op, &mut scratch);
    }
}

/// Job fields for the host's serialization: out receives
/// [ops_off, op_count, tile_addr, tx, ty].
///
/// # Safety
/// `out` must point to 5 usize slots.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn paint_get_job_info(job_index: i32, out: *mut usize) {
    crate::web_surface::job_info(job_index, std::slice::from_raw_parts_mut(out, 5));
}

/// The op-arena address in this instance\x27s memory (the pool workers
/// deserialize the received op bytes here, then blend from it).
#[unsafe(no_mangle)]
pub extern "C" fn paint_ops_arena_ptr() -> i32 {
    crate::web_surface::op_arena_ptr() as i32
}
/// The pool private blended-tile buffer (64x64 rgba16). A plain static:
/// each pool worker has its own module instance and memory.
static POOL_TILE: [u16; 64 * 64 * 4] = [0; 64 * 64 * 4];

#[unsafe(no_mangle)]
pub extern "C" fn paint_pool_tile_ptr() -> i32 {
    POOL_TILE.as_ptr() as i32
}

/// Install a silent panic hook once: the error code is the reporting
/// channel; unwinding restores a usable module state.
fn install_panic_hook() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|_| {}));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_app_access_returns_without_a_panic() {
        let result = with_app(|_| with_app(|_| 7u32));
        assert_eq!(result, 0);
        assert_eq!(with_app(|app| app.error_code), 0);
    }
}
