//! Production LinkeDOM ↔ Blitz bridge and transparent HUD paint source.
//!
//! The game frame remains entirely on the shared WebGPU device. Blitz emits
//! the DOM/HUD paint scene; Vello rasterizes and composites it on that device
//! whenever the DOM epoch or interaction state changes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::browser::{BrowserDocument, BrowserSnapshot, DomBoxMetrics, DomIntersection, DomRect};
use deno_core::{OpState, op2};
use deno_error::JsErrorBox;

pub struct HudPaintState {
    dirty: bool,
    width: u32,
    height: u32,
    surface_canvas_node: Option<u64>,
}

#[derive(Clone)]
struct NativeResourceLoader {
    client: reqwest::Client,
    cache: Arc<std::sync::Mutex<HashMap<String, Vec<u8>>>>,
}

impl NativeResourceLoader {
    async fn load(&self, specifier: &url::Url) -> Result<Vec<u8>, JsErrorBox> {
        let key = specifier.as_str().to_string();
        if let Some(bytes) = self.cache.lock().unwrap().get(&key).cloned() {
            return Ok(bytes);
        }
        let bytes = match specifier.scheme() {
            "file" => {
                let path = specifier
                    .to_file_path()
                    .map_err(|_| JsErrorBox::generic(format!("not a file URL: {specifier}")))?;
                tokio::fs::read(&path)
                    .await
                    .map_err(|error| JsErrorBox::generic(format!("read {path:?}: {error}")))?
            }
            "http" | "https" => self
                .client
                .get(specifier.clone())
                .send()
                .await
                .map_err(|error| JsErrorBox::generic(format!("GET {specifier}: {error}")))?
                .error_for_status()
                .map_err(|error| JsErrorBox::generic(format!("GET {specifier}: {error}")))?
                .bytes()
                .await
                .map_err(|error| JsErrorBox::generic(format!("read {specifier}: {error}")))?
                .to_vec(),
            scheme => {
                return Err(JsErrorBox::generic(format!(
                    "unsupported resource URL scheme {scheme}: {specifier}"
                )));
            }
        };
        self.cache.lock().unwrap().insert(key, bytes.clone());
        Ok(bytes)
    }
}

struct NativeAssetState {
    loaded_bytes: AtomicU64,
    pending_fetches: AtomicU32,
    fetch_activity: AtomicU64,
}

deno_core::extension!(
    native_browser_ext,
    ops = [
        op_probe_log,
        op_sync_browser_document,
        op_browser_computed_property,
        op_browser_box_metrics,
        op_browser_media_query_matches,
        op_browser_intersection,
        op_browser_set_focus,
        op_browser_set_pointer_state,
        op_browser_set_scroll,
        op_browser_hit_test,
        op_browser_hit_tests,
        op_browser_rect,
        op_resize_hud,
        op_set_loaded_asset_bytes,
        op_set_fetch_state,
        op_fetch_url,
        op_decode_image,
    ],
);

pub fn install_state(state: &mut OpState, width: u32, height: u32, scale: f64, base_url: url::Url) {
    let mut document = BrowserDocument::new(width, height);
    document.resize_viewport(width, height, scale);
    state.put::<BrowserDocument>(document);
    state.put::<HudPaintState>(HudPaintState {
        dirty: true,
        width,
        height,
        surface_canvas_node: None,
    });
    state.put::<url::Url>(base_url);
    state.put::<NativeResourceLoader>(NativeResourceLoader {
        client: reqwest::Client::builder()
            .user_agent("afterglow-shell/0.1")
            .build()
            .expect("create native browser resource client"),
        cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
    });
    state.put::<NativeAssetState>(NativeAssetState {
        loaded_bytes: AtomicU64::new(0),
        pending_fetches: AtomicU32::new(0),
        fetch_activity: AtomicU64::new(0),
    });
}

fn mark_hud_dirty(state: &mut OpState) {
    state.borrow_mut::<HudPaintState>().dirty = true;
}

pub fn set_surface_canvas_node(state: &mut OpState, native_id: u64) {
    let hud = state.borrow_mut::<HudPaintState>();
    hud.surface_canvas_node = Some(native_id);
    hud.dirty = true;
}

#[op2(fast)]
fn op_probe_log(#[string] message: String) {
    eprintln!("[browser] {message}");
}

#[op2]
fn op_sync_browser_document(
    state: &mut OpState,
    epoch: u32,
    #[serde] snapshot: BrowserSnapshot,
    #[string] base_url: String,
    #[serde] _dirty_node_ids: Vec<u64>,
    _full_paint: bool,
) -> Result<(), JsErrorBox> {
    state
        .borrow_mut::<BrowserDocument>()
        .sync(epoch as u64, snapshot, &base_url)
        .map_err(JsErrorBox::generic)?;
    mark_hud_dirty(state);
    Ok(())
}

#[op2]
#[string]
fn op_browser_computed_property(
    state: &mut OpState,
    native_node_id: u32,
    #[string] property_name: String,
    #[string] pseudo: String,
) -> Result<String, JsErrorBox> {
    state
        .borrow::<BrowserDocument>()
        .computed_property(native_node_id as u64, &property_name, &pseudo)
        .map_err(JsErrorBox::generic)
}

#[op2]
#[serde]
fn op_browser_box_metrics(
    state: &mut OpState,
    native_node_id: u32,
) -> Result<DomBoxMetrics, JsErrorBox> {
    state
        .borrow::<BrowserDocument>()
        .box_metrics(native_node_id as u64)
        .map_err(JsErrorBox::generic)
}

#[op2(fast)]
fn op_browser_media_query_matches(
    state: &mut OpState,
    #[string] query: String,
) -> Result<bool, JsErrorBox> {
    state
        .borrow::<BrowserDocument>()
        .media_query_matches(&query)
        .map_err(JsErrorBox::generic)
}

#[op2]
#[serde]
fn op_browser_intersection(
    state: &mut OpState,
    native_node_id: u32,
    root_native_node_id: u32,
    margin_top: f64,
    margin_right: f64,
    margin_bottom: f64,
    margin_left: f64,
) -> Result<DomIntersection, JsErrorBox> {
    state
        .borrow::<BrowserDocument>()
        .intersection(
            native_node_id as u64,
            (root_native_node_id != 0).then_some(root_native_node_id as u64),
            [margin_top, margin_right, margin_bottom, margin_left],
        )
        .map_err(JsErrorBox::generic)
}

#[op2(fast)]
fn op_browser_set_focus(state: &mut OpState, native_node_id: u32) -> Result<bool, JsErrorBox> {
    let changed = state
        .borrow_mut::<BrowserDocument>()
        .set_focus((native_node_id != 0).then_some(native_node_id as u64))
        .map_err(JsErrorBox::generic)?;
    if changed {
        mark_hud_dirty(state);
    }
    Ok(changed)
}

#[op2(fast)]
fn op_browser_set_pointer_state(
    state: &mut OpState,
    action: u32,
    x: f64,
    y: f64,
) -> Result<bool, JsErrorBox> {
    let changed = state
        .borrow_mut::<BrowserDocument>()
        .set_pointer_state(action, x, y)
        .map_err(JsErrorBox::generic)?;
    if changed {
        mark_hud_dirty(state);
    }
    Ok(changed)
}

#[op2(fast)]
fn op_browser_set_scroll(
    state: &mut OpState,
    native_node_id: u32,
    left: f64,
    top: f64,
) -> Result<bool, JsErrorBox> {
    let changed = state
        .borrow_mut::<BrowserDocument>()
        .set_scroll(native_node_id as u64, left, top)
        .map_err(JsErrorBox::generic)?;
    if changed {
        mark_hud_dirty(state);
    }
    Ok(changed)
}

#[op2]
#[serde]
fn op_browser_hit_test(state: &mut OpState, x: f64, y: f64) -> Result<Option<u64>, JsErrorBox> {
    state
        .borrow::<BrowserDocument>()
        .hit_test(x, y)
        .map_err(JsErrorBox::generic)
}

#[op2]
#[serde]
fn op_browser_hit_tests(state: &mut OpState, x: f64, y: f64) -> Result<Vec<u64>, JsErrorBox> {
    state
        .borrow::<BrowserDocument>()
        .hit_tests(x, y)
        .map_err(JsErrorBox::generic)
}

#[op2]
#[serde]
fn op_browser_rect(state: &mut OpState, native_node_id: u32) -> Result<DomRect, JsErrorBox> {
    state
        .borrow::<BrowserDocument>()
        .rect(native_node_id as u64)
        .map_err(JsErrorBox::generic)
}

#[op2(fast)]
fn op_resize_hud(state: &mut OpState, width: u32, height: u32, scale: f64) {
    state
        .borrow_mut::<BrowserDocument>()
        .resize_viewport(width, height, scale);
    let hud = state.borrow_mut::<HudPaintState>();
    hud.width = width.max(1);
    hud.height = height.max(1);
    hud.dirty = true;
}

pub struct HudGpuScene {
    pub scene: vello::Scene,
    pub width: u32,
    pub height: u32,
}

pub fn take_gpu_hud_scene(state: &mut OpState) -> Result<Option<HudGpuScene>, JsErrorBox> {
    if !state.borrow::<HudPaintState>().dirty {
        return Ok(None);
    }
    let (width, height) = {
        let hud = state.borrow::<HudPaintState>();
        (hud.width, hud.height)
    };
    let surface_canvas_node = state.borrow::<HudPaintState>().surface_canvas_node;
    if let Some(native_id) = surface_canvas_node {
        state
            .borrow_mut::<BrowserDocument>()
            .suppress_canvas_paint(native_id)
            .map_err(JsErrorBox::generic)?;
    }
    let mut scene = vello::Scene::new();
    state
        .borrow_mut::<BrowserDocument>()
        .paint_overlay(&mut anyrender_vello::VelloScenePainter::new(&mut scene))
        .map_err(JsErrorBox::generic)?;
    state.borrow_mut::<HudPaintState>().dirty = false;
    Ok(Some(HudGpuScene {
        scene,
        width,
        height,
    }))
}

#[op2(fast)]
fn op_set_loaded_asset_bytes(state: &mut OpState, bytes: u32) {
    state
        .borrow::<NativeAssetState>()
        .loaded_bytes
        .store(bytes as u64, Ordering::Release);
}

#[op2(fast)]
fn op_set_fetch_state(state: &mut OpState, pending: u32, activity: u32) {
    let assets = state.borrow::<NativeAssetState>();
    assets.pending_fetches.store(pending, Ordering::Release);
    assets
        .fetch_activity
        .store(activity as u64, Ordering::Release);
}

#[op2]
#[buffer]
async fn op_fetch_url(
    state: Rc<RefCell<OpState>>,
    #[string] requested_url: String,
) -> Result<Vec<u8>, JsErrorBox> {
    let (base, resources) = {
        let state = state.borrow();
        (
            state.borrow::<url::Url>().clone(),
            state.borrow::<NativeResourceLoader>().clone(),
        )
    };
    let absolute = if requested_url.starts_with("file://") || requested_url.starts_with("http") {
        url::Url::parse(&requested_url)
    } else {
        base.join(&requested_url)
    }
    .map_err(|error| JsErrorBox::generic(format!("url {requested_url}: {error}")))?;
    resources.load(&absolute).await
}

#[derive(serde::Serialize)]
struct DecodedImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

#[op2]
#[serde]
fn op_decode_image(#[buffer] bytes: &[u8]) -> Result<DecodedImage, JsErrorBox> {
    let image = deno_image::image::load_from_memory(bytes)
        .map_err(|error| JsErrorBox::generic(format!("decode image: {error}")))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    Ok(DecodedImage {
        width,
        height,
        data: image.into_raw(),
    })
}
