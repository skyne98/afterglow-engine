//! Production LinkeDOM ↔ Blitz bridge and transparent HUD paint source.
//!
//! The game frame remains entirely on the shared WebGPU device. Blitz emits
//! the DOM/HUD paint scene; Vello rasterizes and composites it on that device
//! whenever the DOM epoch or interaction state changes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::browser::{BrowserDocument, BrowserSnapshot, DomBoxMetrics, DomIntersection, DomRect};
use deno_core::{JsBuffer, OpState, convert::Uint8Array, op2};
use deno_error::JsErrorBox;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_DIALOG_FILE_BYTES: u64 = 256 * 1024 * 1024;
static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);
const MAX_CLIPBOARD_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Default)]
struct NativeClipboard {
    clipboard: Arc<std::sync::Mutex<Option<arboard::Clipboard>>>,
    busy: Arc<std::sync::atomic::AtomicBool>,
}

#[op2]
#[string]
async fn op_browser_clipboard(
    state: Rc<RefCell<OpState>>,
    write: bool,
    #[string] text: String,
) -> Result<String, JsErrorBox> {
    if text.len() > MAX_CLIPBOARD_BYTES {
        return Err(JsErrorBox::range_error("clipboard text exceeds 32 MiB"));
    }
    let service = state.borrow().borrow::<NativeClipboard>().clone();
    if service.busy.swap(true, Ordering::AcqRel) {
        return Err(JsErrorBox::generic("clipboard operation is already active"));
    }
    tokio::task::spawn_blocking(move || {
        struct Release(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Release {
            fn drop(&mut self) { self.0.store(false, Ordering::Release); }
        }
        let _release = Release(service.busy);
        let mut slot = service.clipboard.lock().map_err(|_| JsErrorBox::generic("clipboard lock is poisoned"))?;
        if slot.is_none() {
            *slot = Some(arboard::Clipboard::new().map_err(|error| JsErrorBox::generic(error.to_string()))?);
        }
        let clipboard = slot.as_mut().unwrap();
        if write {
            clipboard.set_text(text).map_err(|error| JsErrorBox::generic(error.to_string()))?;
            Ok(String::new())
        } else {
            let text = clipboard.get_text().map_err(|error| JsErrorBox::generic(error.to_string()))?;
            if text.len() > MAX_CLIPBOARD_BYTES {
                return Err(JsErrorBox::range_error("clipboard text exceeds 32 MiB"));
            }
            Ok(text)
        }
    }).await.map_err(|error| JsErrorBox::generic(error.to_string()))?
}

pub struct HudPaintState {
    dirty: bool,
    width: u32,
    height: u32,
    surface_canvas_node: Option<u64>,
}

impl HudPaintState {
    /// The host must present a changed HUD even without a pending rAF callback.
    pub fn needs_redraw(&self) -> bool {
        self.dirty
    }
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
        op_browser_document_dirty,
        op_sync_browser_document,
        op_browser_computed_property,
        op_browser_box_metrics,
        op_browser_media_query_matches,
        op_browser_intersection,
        op_browser_set_focus,
        op_browser_text_input,
        op_browser_clipboard,
        op_browser_set_pointer_state,
        op_browser_set_scroll,
        op_browser_hit_test,
        op_browser_hit_tests,
        op_browser_rect,
        op_browser_set_canvas_raster,
        op_resize_hud,
        op_set_loaded_asset_bytes,
        op_set_fetch_state,
        op_fetch_url,
        op_decode_image,
        op_encode_png,
        op_random_bytes,
        op_open_file,
        op_save_file,
    ],
);

pub fn install_state(state: &mut OpState, width: u32, height: u32, scale: f64, base_url: url::Url) {
    let mut document = BrowserDocument::new(width, height);
    document.resize_viewport(width, height, scale);
    state.put::<BrowserDocument>(document);
    state.put(NativeClipboard::default());
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

#[op2(fast)]
fn op_browser_document_dirty(state: &mut OpState) {
    mark_hud_dirty(state);
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

#[op2]
#[serde]
fn op_browser_text_input(
    state: &mut OpState,
    native_node_id: u32,
    #[serde] action: crate::browser::TextInputAction,
) -> Result<crate::browser::TextInputState, JsErrorBox> {
    let changes_editor = action.action != "query";
    let result = state.borrow_mut::<BrowserDocument>()
        .text_input(native_node_id as u64, action).map_err(JsErrorBox::generic)?;
    if changes_editor { mark_hud_dirty(state); }
    Ok(result)
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

fn validate_canvas_raster(width: u32, height: u32, byte_len: usize) -> Result<(), String> {
    const MAX_CANVAS_DIMENSION: u32 = 16_384;
    if width == 0 || height == 0 || width > MAX_CANVAS_DIMENSION || height > MAX_CANVAS_DIMENSION {
        return Err("invalid canvas raster dimensions".to_string());
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "canvas raster byte length overflow".to_string())?;
    if byte_len != expected {
        return Err(format!(
            "canvas raster has {byte_len} bytes, expected {expected}"
        ));
    }
    Ok(())
}

#[op2(fast)]
fn op_browser_set_canvas_raster(
    state: &mut OpState,
    native_node_id: u32,
    width: u32,
    height: u32,
    #[buffer] rgba: &[u8],
    x: u32,
    y: u32,
    region_width: u32,
    region_height: u32,
) -> Result<(), JsErrorBox> {
    state
        .borrow_mut::<BrowserDocument>()
        .update_canvas_raster(native_node_id as u64, width, height,
            crate::browser::RasterRegion { x, y, width: region_width, height: region_height }, rgba)
        .map_err(JsErrorBox::generic)?;
    mark_hud_dirty(state);
    Ok(())
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

pub fn hud_needs_redraw(state: &OpState) -> bool {
    state.borrow::<HudPaintState>().needs_redraw()
        || state.borrow::<BrowserDocument>().is_animating()
}

pub fn take_gpu_hud_scene(state: &mut OpState, device: &wgpu::Device, queue: &wgpu::Queue,
    renderer: &mut vello::Renderer) -> Result<Option<HudGpuScene>, JsErrorBox> {
    if !hud_needs_redraw(state) {
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
    state.borrow_mut::<BrowserDocument>().advance_animations();
    state.borrow_mut::<BrowserDocument>().prepare_canvas_textures(device, queue, renderer)
        .map_err(JsErrorBox::generic)?;
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

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn validate_open_extensions(extensions: &[String]) -> Result<(), JsErrorBox> {
    if extensions.is_empty()
        || extensions.iter().any(|extension| {
            !extension
                .trim()
                .trim_start_matches('.')
                .eq_ignore_ascii_case("ora")
        })
    {
        return Err(JsErrorBox::type_error(
            "only OpenRaster (.ora) files can be opened",
        ));
    }
    Ok(())
}

fn validate_suggested_name(name: &str) -> Result<(String, &'static str), JsErrorBox> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or_default();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(':')
        || name.chars().any(char::is_control)
    {
        return Err(JsErrorBox::type_error("invalid save file name"));
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| JsErrorBox::type_error("save file name must end in .ora or .png"))?;
    let expected = match extension.as_str() {
        "ora" => "ora",
        "png" => "png",
        _ => {
            return Err(JsErrorBox::type_error(
                "save file name must end in .ora or .png",
            ));
        }
    };
    Ok((name.to_string(), expected))
}

async fn read_bounded_file(path: &Path) -> io::Result<Vec<u8>> {
    let metadata = tokio::fs::metadata(path).await?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "selected path is not a file",
        ));
    }
    if metadata.len() > MAX_DIALOG_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "selected file is larger than 256 MiB",
        ));
    }
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_DIALOG_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_DIALOG_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "selected file grew beyond 256 MiB while reading",
        ));
    }
    Ok(bytes)
}

async fn create_temp_file(parent: &Path) -> io::Result<(PathBuf, tokio::fs::File)> {
    for _ in 0..16 {
        let id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".afterglow-save-{}-{id}.tmp", std::process::id()));
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a unique temporary save file",
    ))
}

async fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let (temporary_path, mut temporary) = create_temp_file(parent).await?;
    let result = async {
        temporary.write_all(data).await?;
        temporary.sync_all().await?;
        drop(temporary);
        tokio::fs::rename(&temporary_path, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary_path).await;
    }
    result
}

#[op2]
async fn op_open_file(#[serde] extensions: Vec<String>) -> Result<Option<Uint8Array>, JsErrorBox> {
    validate_open_extensions(&extensions)?;
    let Some(file) = rfd::AsyncFileDialog::new()
        .add_filter("OpenRaster", &["ora"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let path = file.path().to_path_buf();
    if !has_extension(&path, "ora") {
        return Err(JsErrorBox::type_error(
            "selected file is not an OpenRaster (.ora) file",
        ));
    }
    read_bounded_file(&path)
        .await
        .map(Uint8Array::from)
        .map(Some)
        .map_err(|error| JsErrorBox::generic(format!("open file: {error}")))
}

#[op2]
async fn op_save_file(
    #[string] suggested_name: String,
    #[buffer] data: JsBuffer,
) -> Result<bool, JsErrorBox> {
    if data.len() as u64 > MAX_DIALOG_FILE_BYTES {
        return Err(JsErrorBox::type_error("save file is larger than 256 MiB"));
    }
    let (suggested_name, extension) = validate_suggested_name(&suggested_name)?;
    // JsBuffer borrows V8 memory; clone it before the dialog yields control.
    let data = data.to_vec();
    let Some(file) = rfd::AsyncFileDialog::new()
        .add_filter(
            if extension == "png" {
                "PNG image"
            } else {
                "OpenRaster"
            },
            &[extension],
        )
        .set_file_name(suggested_name)
        .save_file()
        .await
    else {
        return Ok(false);
    };
    let path = file.path().to_path_buf();
    let path = if path.extension().is_none() {
        path.with_extension(extension)
    } else if has_extension(&path, extension) {
        path
    } else {
        return Err(JsErrorBox::type_error(format!(
            "selected file must use the .{extension} extension"
        )));
    };
    write_atomic(&path, &data)
        .await
        .map(|_| true)
        .map_err(|error| JsErrorBox::generic(format!("save file: {error}")))
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, JsErrorBox> {
    validate_canvas_raster(width, height, rgba.len()).map_err(JsErrorBox::generic)?;
    let image = deno_image::image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| JsErrorBox::generic("invalid RGBA canvas raster"))?;
    let mut encoded = Vec::new();
    deno_image::image::DynamicImage::ImageRgba8(image)
        .write_to(
            &mut Cursor::new(&mut encoded),
            deno_image::image::ImageFormat::Png,
        )
        .map_err(|error| JsErrorBox::generic(format!("encode PNG: {error}")))?;
    Ok(encoded)
}

#[op2]
#[buffer]
fn op_encode_png(width: u32, height: u32, #[buffer] rgba: &[u8]) -> Result<Vec<u8>, JsErrorBox> {
    encode_png(width, height, rgba)
}

fn random_bytes(bytes: &mut [u8]) -> Result<(), JsErrorBox> {
    if bytes.len() > 65_536 {
        return Err(JsErrorBox::generic("random byte limit exceeded"));
    }
    getrandom::fill(bytes).map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2(fast)]
fn op_random_bytes(#[buffer] bytes: &mut [u8]) -> Result<(), JsErrorBox> {
    random_bytes(bytes)
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

#[cfg(test)]
mod tests {
    use super::{
        encode_png, validate_canvas_raster, validate_open_extensions, validate_suggested_name,
    };

    #[test]
    fn random_bytes_enforces_the_web_quota() {
        assert!(super::random_bytes(&mut []).is_ok());
        assert!(super::random_bytes(&mut vec![0; 65_536]).is_ok());
        let mut excess = vec![7; 65_537];
        assert!(super::random_bytes(&mut excess).is_err());
        assert!(excess.iter().all(|&byte| byte == 7));
    }

    #[test]
    fn hud_changes_need_presentation_without_animation_frames() {
        let mut hud = super::HudPaintState {
            dirty: false, width: 1, height: 1, surface_canvas_node: None,
        };
        assert!(!hud.needs_redraw());
        hud.dirty = true;
        assert!(hud.needs_redraw());
        hud.dirty = false;
        assert!(!hud.needs_redraw());
    }

    #[test]
    fn canvas_raster_validation_is_bounded_and_exact() {
        assert!(validate_canvas_raster(1024, 1024, 1024 * 1024 * 4).is_ok());
        assert!(validate_canvas_raster(0, 1024, 0).is_err());
        assert!(validate_canvas_raster(16_385, 1, 16_385 * 4).is_err());
        assert!(validate_canvas_raster(2, 2, 15).is_err());
    }

    #[test]
    fn png_encoding_preserves_rgba_pixels() {
        let pixels = [255, 0, 0, 255, 0, 128, 255, 64];
        let encoded = encode_png(2, 1, &pixels).unwrap();
        let decoded = deno_image::image::load_from_memory(&encoded)
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded.into_raw(), pixels);
    }

    #[test]
    fn file_dialog_arguments_are_restricted() {
        assert!(validate_open_extensions(&["ora".to_string()]).is_ok());
        assert!(validate_open_extensions(&["ora".to_string(), "png".to_string()]).is_err());
        assert_eq!(
            validate_suggested_name(r"nested\\paint.PNG").unwrap(),
            ("paint.PNG".to_string(), "png")
        );
        assert!(validate_suggested_name("paint.jpg").is_err());
        assert!(validate_suggested_name("C:paint.png").is_err());
        assert!(validate_suggested_name("../../paint.png\n").is_err());
    }
}
