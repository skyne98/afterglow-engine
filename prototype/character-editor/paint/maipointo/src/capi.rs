//! C ABI surface for the demo's Emscripten wasm module (`capi`).
//!
//! Exports mirror the `mypaint_brush_*` public API the demo uses plus the
//! cooperative stroke driver, so `main.c` can select this engine at build
//! time. The C host hands us its `MyPaintSurface*`; dab and get_color calls
//! go straight back through libmypaint's public surface entry points, which
//! are linked into the same module.

#![allow(clippy::missing_safety_doc)]
#![allow(unsafe_op_in_unsafe_fn)]

use crate::brush::cooperative::StrokeState;
use crate::brush::Brush;
use crate::settings::{InputId, SettingId};
use crate::surface::Surface;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

/// Route all Rust allocations through the module's emscripten malloc. The
/// staticlib is linked into an emcc module whose heap is managed by
/// emscripten's allocator; a second wasm allocator (the wasm32-unknown-
/// unknown std default) would double-manage the same linear memory.
struct EmscriptenAlloc;

unsafe impl std::alloc::GlobalAlloc for EmscriptenAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        unsafe extern "C" {
            fn malloc(size: usize) -> *mut u8;
        }
        unsafe { malloc(layout.size()) }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: std::alloc::Layout) {
        unsafe extern "C" {
            fn free(ptr: *mut u8);
        }
        unsafe { free(ptr) }
    }

    #[inline]
    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        _layout: std::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        unsafe extern "C" {
            fn realloc(ptr: *mut u8, size: usize) -> *mut u8;
        }
        unsafe { realloc(ptr, new_size) }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        unsafe extern "C" {
            fn calloc(n: usize, size: usize) -> *mut u8;
        }
        unsafe { calloc(1, layout.size()) }
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: EmscriptenAlloc = EmscriptenAlloc;

/// The host's opaque surface. We only ever pass it back to the C surface
/// entry points.
#[derive(Clone, Copy)]
pub struct CBrushSurface(pub *mut std::ffi::c_void);

unsafe extern "C" {
    fn mypaint_surface_draw_dab(
        surface: *mut std::ffi::c_void,
        x: f32,
        y: f32,
        radius: f32,
        color_r: f32,
        color_g: f32,
        color_b: f32,
        opaque: f32,
        hardness: f32,
        softness: f32,
        alpha_eraser: f32,
        aspect_ratio: f32,
        angle: f32,
        lock_alpha: f32,
        colorize: f32,
        posterize: f32,
        posterize_num: f32,
        paint: f32,
    ) -> c_int;

    fn mypaint_surface_get_color(
        surface: *mut std::ffi::c_void,
        x: f32,
        y: f32,
        radius: f32,
        color_r: *mut f32,
        color_g: *mut f32,
        color_b: *mut f32,
        color_a: *mut f32,
        paint: f32,
    );
}

impl Surface for CBrushSurface {
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
        unsafe {
            mypaint_surface_draw_dab(
                self.0,
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
            ) != 0
        }
    }

    fn get_color(&mut self, x: f32, y: f32, radius: f32, paint: f32) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        unsafe {
            mypaint_surface_get_color(
                self.0, x, y, radius,
                &mut out[0], &mut out[1], &mut out[2], &mut out[3],
                paint,
            );
        }
        out
    }
}

/// The cooperative wrapper in the C demo keeps one global continuation.
static mut STROKE: Option<StrokeState> = None;

fn stroke_state() -> *mut StrokeState {
    unsafe {
        let head = std::ptr::addr_of_mut!(STROKE);
        if (*head).is_none() {
            *head = Some(StrokeState::default());
        }
        (*head).as_mut().unwrap_unchecked()
    }
}

fn stroke_cancel() {
    unsafe {
        let head = std::ptr::addr_of_mut!(STROKE);
        if let Some(s) = (*head).as_mut() {
            s.cancel();
        }
    }
}

fn stroke_pending() -> bool {
    unsafe {
        let head = std::ptr::addr_of!(STROKE);
        (*head).as_ref().is_some_and(|s| s.pending())
    }
}

fn cname_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok() }
}

/// `mypaint_brush_new` / `mypaint_brush_new_with_buckets`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_new(num_smudge_buckets: c_int) -> *mut Brush {
    Box::into_raw(Box::new(Brush::new_with_buckets(num_smudge_buckets)))
}

/// `mypaint_brush_unref` (refcounted in C; single-owner here).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_free(brush: *mut Brush) {
    if !brush.is_null() {
        drop(unsafe { Box::from_raw(brush) });
    }
}

/// `mypaint_brush_from_defaults`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_from_defaults(brush: *mut Brush) {
    if !brush.is_null() {
        (*brush).from_defaults();
    }
}

/// `.myb` v3 JSON loader — mirrors `update_brush_from_json_object`:
/// version must be 3, each known setting applies base_value + inputs, and
/// the call succeeds when at least one setting applied.
fn maipo_brush_load_json(brush: &mut Brush, text: &str) -> bool {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    if root.get("version").and_then(|v| v.as_i64()) != Some(3) {
        return false;
    }
    let Some(settings) = root.get("settings") else {
        return false;
    };
    let Some(settings) = settings.as_object() else {
        return false;
    };
    let mut updated_any = false;
    for (setting_name, setting_obj) in settings {
        let Some(setting_index) = crate::settings::setting_index_runtime(setting_name) else {
            continue; // unknown setting: warn-free skip, keep scanning
        };
        let Some(base_value) = setting_obj.get("base_value").and_then(|v| v.as_f64()) else {
            continue;
        };
        brush.set_base_value_at(setting_index, base_value as f32);
        updated_any = true;
        let Some(inputs) = setting_obj.get("inputs").and_then(|v| v.as_object()) else {
            continue;
        };
        for (input_name, input_obj) in inputs {
            let Some(input_index) = crate::settings::input_index_runtime(input_name) else {
                continue;
            };
            let Some(points) = input_obj.as_array() else {
                continue;
            };
            brush.set_mapping_n_at(setting_index, input_index, points.len());
            for (i, point) in points.iter().enumerate() {
                let Some(point) = point.as_array() else { continue };
                if point.len() < 2 {
                    continue;
                }
                let x = point[0].as_f64().unwrap_or(0.0) as f32;
                let y = point[1].as_f64().unwrap_or(0.0) as f32;
                brush.set_mapping_point_at(setting_index, input_index, i, x, y);
            }
        }
    }
    updated_any
}

/// `mypaint_brush_from_string` — `.myb` v3 JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_from_string(
    brush: *mut Brush,
    json: *const c_char,
) -> c_int {
    if brush.is_null() {
        return 0;
    }
    let Some(text) = cname_to_str(json) else {
        return 0;
    };
    maipo_brush_load_json(&mut *brush, text) as c_int
}

/// `mypaint_brush_new_stroke`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_new_stroke(brush: *mut Brush) {
    if !brush.is_null() {
        (*brush).new_stroke();
    }
}

/// `mypaint_brush_reset` (queues a reset for the next stroke_to).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_reset(brush: *mut Brush) {
    if !brush.is_null() {
        (*brush).request_reset();
    }
}

/// `mypaint_brush_setting_from_cname` — -1 when unknown.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_setting_from_cname(cname: *const c_char) -> c_int {
    match cname_to_str(cname).and_then(crate::settings::setting_index_runtime) {
        Some(i) => i as c_int,
        None => -1,
    }
}

/// `mypaint_brush_input_from_cname` — -1 when unknown.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_input_from_cname(cname: *const c_char) -> c_int {
    match cname_to_str(cname).and_then(crate::settings::input_index_runtime) {
        Some(i) => i as c_int,
        None => -1,
    }
}

/// `mypaint_brush_set_base_value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_set_base_value(
    brush: *mut Brush,
    setting: c_int,
    value: f32,
) {
    if !brush.is_null() && setting >= 0 {
        (*brush).set_base_value_at(setting as usize, value);
    }
}

/// `mypaint_brush_get_base_value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_get_base_value(
    brush: *mut Brush,
    setting: c_int,
) -> f32 {
    if !brush.is_null() && setting >= 0 {
        if let Some(id) = SettingId::from_index(setting as usize) {
            return (*brush).get_base_value(id);
        }
    }
    0.0
}

/// `mypaint_brush_set_mapping_n`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_set_mapping_n(
    brush: *mut Brush,
    setting: c_int,
    input: c_int,
    n: c_int,
) {
    if !brush.is_null() && setting >= 0 && input >= 0 && n >= 0 {
        (*brush).set_mapping_n_at(setting as usize, input as usize, n as usize);
    }
}

/// `mypaint_brush_set_mapping_point`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_set_mapping_point(
    brush: *mut Brush,
    setting: c_int,
    input: c_int,
    index: c_int,
    x: f32,
    y: f32,
) {
    if !brush.is_null() && setting >= 0 && input >= 0 && index >= 0 {
        (*brush).set_mapping_point_at(setting as usize, input as usize, index as usize, x, y);
    }
}

/// `afterglow_brush_stroke_start` against the host's C surface.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn maipo_brush_stroke_start(
    brush: *mut Brush,
    surface: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    pressure: f32,
    xtilt: f32,
    ytilt: f32,
    dtime: f64,
    viewzoom: f32,
    viewrotation: f32,
    barrel_rotation: f32,
    linear: c_int,
    dab_budget: c_int,
) -> c_int {
    if brush.is_null() || surface.is_null() || stroke_pending() {
        return -1;
    }
    let mut c_surface = CBrushSurface(surface);
    ACTIVE_BRUSH = brush;
    ACTIVE_SURFACE = surface;
    (*brush).cooperative_stroke_start(
        &mut c_surface,
        &mut *stroke_state(),
        x,
        y,
        pressure,
        xtilt,
        ytilt,
        dtime,
        viewzoom,
        viewrotation,
        barrel_rotation,
        linear != 0,
        dab_budget,
    )
}

/// Continuation state globals: the active brush/surface pair (stored at
/// stroke start) plus the cooperative budget state.
static mut ACTIVE_BRUSH: *mut Brush = std::ptr::null_mut();
static mut ACTIVE_SURFACE: *mut std::ffi::c_void = std::ptr::null_mut();

/// `afterglow_brush_stroke_continue` (brush/surface come from start).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_stroke_continue(dab_budget: c_int) -> c_int {
    if !stroke_pending() {
        return -1;
    }
    let brush = ACTIVE_BRUSH;
    let surface = ACTIVE_SURFACE;
    if brush.is_null() || surface.is_null() {
        return -1;
    }
    let mut c_surface = CBrushSurface(surface);
    (*brush).cooperative_stroke_continue(&mut c_surface, &mut *stroke_state(), dab_budget)
}

/// `afterglow_brush_stroke_pending`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_stroke_pending() -> c_int {
    stroke_pending() as c_int
}

/// Unbudgeted direct `mypaint_brush_stroke_to` (warm-up path).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn maipo_brush_stroke_to(
    brush: *mut Brush,
    surface: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    pressure: f32,
    xtilt: f32,
    ytilt: f32,
    dtime: f64,
    viewzoom: f32,
    viewrotation: f32,
    barrel_rotation: f32,
    linear: c_int,
) -> c_int {
    if brush.is_null() || surface.is_null() {
        return -1;
    }
    let mut c_surface = CBrushSurface(surface);
    (*brush).stroke_to(
        &mut c_surface, x, y, pressure, xtilt, ytilt, dtime, viewzoom,
        viewrotation, barrel_rotation, linear != 0,
    ) as c_int
}

/// `afterglow_brush_stroke_cancel`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn maipo_brush_stroke_cancel() {
    stroke_cancel();
    ACTIVE_BRUSH = std::ptr::null_mut();
    ACTIVE_SURFACE = std::ptr::null_mut();
}
