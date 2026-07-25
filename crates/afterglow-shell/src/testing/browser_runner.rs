// Run a real three.js `examples/<name>.html` in the deno_core + deno_webgpu
// runtime, with the REAL three.js determinism injection, and capture the canvas.
//
// NO HACKS (see AGENTS.md): the example module and all addons run VERBATIM.
// The host only provides the environment (DOM shim, WebGPU, fetch, timers) and
// gates _renderStarted on the event loop going idle (init + asset loads done),
// exactly like three.js's own e2e harness (waitForNetworkIdle + delay).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::browser::{
    BrowserDocument, BrowserSnapshot, CanvasRaster, DomBoxMetrics, DomIntersection, DomRect,
};
use crate::runtime::{ReadinessRegistry, ReadinessSubsystem, RuntimeLifecycle, RuntimePhase};
use deno_core::v8;
use deno_core::{
    JsRuntime, ModuleId, ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader,
    ModuleResolveResponse, ModuleSource, ModuleSourceCode, ModuleSpecifier, ModuleType, OpState,
    PollEventLoopOptions, ResolutionKind, RuntimeOptions, op2, serde,
};
use deno_error::JsErrorBox;
use deno_image::image::{DynamicImage, RgbaImage};
use deno_webgpu::canvas::{ContextData, GPUCanvasContext, create as create_canvas_context};
use futures_util::FutureExt;

static SNAPSHOT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/SNAPSHOT.bin"));

struct CaptureCanvas {
    id: u32,
    native_node_id: Option<u64>,
    ctx: v8::Global<v8::Value>,
    image: Rc<RefCell<DynamicImage>>,
}
struct CaptureCanvases(Vec<CaptureCanvas>);
struct RenderFinished(Arc<std::sync::atomic::AtomicBool>);
struct RenderReady(Arc<std::sync::atomic::AtomicBool>);
struct LoadedAssetBytes(Arc<std::sync::atomic::AtomicU64>);
struct PendingFetches(Arc<std::sync::atomic::AtomicU32>);
struct FetchActivity(Arc<std::sync::atomic::AtomicU64>);
struct PendingReadinessTokens(HashMap<u32, crate::runtime::ReadinessToken>);
type CaptureSlot = Arc<std::sync::Mutex<Option<Vec<u8>>>>;

#[derive(Clone)]
struct ResourceLoader {
    client: reqwest::Client,
    cache: Arc<std::sync::Mutex<HashMap<String, Vec<u8>>>>,
    readiness: Arc<ReadinessRegistry>,
}

impl ResourceLoader {
    async fn load(&self, specifier: &url::Url) -> Result<Vec<u8>, JsErrorBox> {
        let key = specifier.as_str().to_string();
        if let Some(bytes) = self.cache.lock().unwrap().get(&key).cloned() {
            return Ok(bytes);
        }

        let readiness = self.readiness.begin(
            ReadinessSubsystem::Resource,
            "load",
            Some(specifier.to_string()),
        );
        let bytes = match specifier.scheme() {
            "file" => {
                let path = specifier
                    .to_file_path()
                    .map_err(|_| JsErrorBox::generic(format!("not a file URL: {specifier}")))?;
                tokio::fs::read(&path)
                    .await
                    .map_err(|error| JsErrorBox::generic(format!("read {path:?}: {error}")))?
            }
            "http" | "https" => {
                let response = self
                    .client
                    .get(specifier.clone())
                    .send()
                    .await
                    .map_err(|error| JsErrorBox::generic(format!("GET {specifier}: {error}")))?
                    .error_for_status()
                    .map_err(|error| JsErrorBox::generic(format!("GET {specifier}: {error}")))?;
                response
                    .bytes()
                    .await
                    .map_err(|error| JsErrorBox::generic(format!("read {specifier}: {error}")))?
                    .to_vec()
            }
            scheme => {
                return Err(JsErrorBox::generic(format!(
                    "unsupported resource URL scheme {scheme}: {specifier}"
                )));
            }
        };

        self.cache.lock().unwrap().insert(key, bytes.clone());
        readiness.complete();
        Ok(bytes)
    }
}

deno_core::extension!(
    probe_ext,
    ops = [
        op_probe_log,
        op_create_capture_canvas,
        op_resize_canvas,
        op_bind_canvas_node,
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
        op_capture_png,
        op_signal_finished,
        op_signal_ready,
        op_signal_gpu_submission,
        op_signal_gpu_phase,
        op_begin_readiness,
        op_finish_readiness,
        op_set_loaded_asset_bytes,
        op_set_fetch_state,
        op_fetch_url,
        op_decode_image,
    ],
);

#[op2(fast)]
fn op_probe_log(#[string] msg: String) {
    eprintln!("[js] {msg}");
}

#[op2]
fn op_create_capture_canvas(
    state: &mut OpState,
    scope: &mut v8::PinScope,
    id: u32,
    w: u32,
    h: u32,
) -> Result<v8::Global<v8::Value>, JsErrorBox> {
    let image = Rc::new(RefCell::new(DynamicImage::from(RgbaImage::new(
        w.max(1),
        h.max(1),
    ))));
    let canvas_local = v8::Object::new(scope);
    let canvas_global = v8::Global::new(scope, canvas_local);
    let ctx = create_canvas_context(
        None,
        canvas_global,
        ContextData::Canvas(image.clone()),
        scope,
        v8::undefined(scope).into(),
        "",
        "webgpu",
    )?;
    state.borrow::<Arc<ReadinessRegistry>>().register_canvas();
    state.borrow_mut::<CaptureCanvases>().0.push(CaptureCanvas {
        id,
        native_node_id: None,
        ctx: ctx.clone(),
        image,
    });
    Ok(ctx)
}

#[op2(fast)]
fn op_bind_canvas_node(state: &mut OpState, id: u32, native_node_id: u32) {
    let canvas = state
        .borrow_mut::<CaptureCanvases>()
        .0
        .iter_mut()
        .find(|canvas| canvas.id == id)
        .unwrap_or_else(|| panic!("unknown capture canvas {id}"));
    canvas.native_node_id = Some(native_node_id as u64);
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
        .map_err(JsErrorBox::generic)
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
    state
        .borrow_mut::<BrowserDocument>()
        .set_focus((native_node_id != 0).then_some(native_node_id as u64))
        .map_err(JsErrorBox::generic)
}

#[op2(fast)]
fn op_browser_set_pointer_state(
    state: &mut OpState,
    action: u32,
    x: f64,
    y: f64,
) -> Result<bool, JsErrorBox> {
    state
        .borrow_mut::<BrowserDocument>()
        .set_pointer_state(action, x, y)
        .map_err(JsErrorBox::generic)
}

#[op2(fast)]
fn op_browser_set_scroll(
    state: &mut OpState,
    native_node_id: u32,
    left: f64,
    top: f64,
) -> Result<bool, JsErrorBox> {
    state
        .borrow_mut::<BrowserDocument>()
        .set_scroll(native_node_id as u64, left, top)
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
fn op_resize_canvas(state: &mut OpState, id: u32, w: u32, h: u32) {
    if let Some(cap) = state
        .try_borrow::<CaptureCanvases>()
        .and_then(|canvases| canvases.0.iter().find(|canvas| canvas.id == id))
    {
        let mut img = cap.image.borrow_mut();
        if img.width() != w || img.height() != h {
            *img = DynamicImage::from(RgbaImage::new(w.max(1), h.max(1)));
        }
    }
}

fn copy_texture_bytes(
    texture: &deno_webgpu::texture::GPUTexture,
    bytes_per_pixel: u32,
) -> Result<Vec<u8>, JsErrorBox> {
    let size = &texture.size;
    let unpadded_bytes_per_row = size
        .width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| JsErrorBox::range_error("texture row size overflow"))?;
    let alignment = wgpu_types::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = (unpadded_bytes_per_row + alignment - 1) / alignment * alignment;
    let buffer_size = padded_bytes_per_row as u64 * size.height as u64;

    let (buffer, error) = texture.instance.device_create_buffer(
        texture.device_id,
        &wgpu_types::BufferDescriptor {
            label: Some("native canvas readback".into()),
            size: buffer_size,
            usage: wgpu_types::BufferUsages::MAP_READ | wgpu_types::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        },
        None,
    );
    if let Some(error) = error {
        return Err(JsErrorBox::generic(format!(
            "create readback buffer: {error}"
        )));
    }

    let (encoder, error) = texture.instance.device_create_command_encoder(
        texture.device_id,
        &wgpu_types::CommandEncoderDescriptor {
            label: Some("native canvas readback".into()),
        },
        None,
    );
    if let Some(error) = error {
        return Err(JsErrorBox::generic(format!(
            "create readback encoder: {error}"
        )));
    }

    texture
        .instance
        .command_encoder_copy_texture_to_buffer(
            encoder,
            &wgpu_types::TexelCopyTextureInfo {
                texture: texture.id,
                mip_level: 0,
                origin: Default::default(),
                aspect: Default::default(),
            },
            &wgpu_types::TexelCopyBufferInfo {
                buffer,
                layout: wgpu_types::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: None,
                },
            },
            size,
        )
        .map_err(|error| JsErrorBox::generic(format!("copy canvas texture: {error}")))?;

    let (command_buffer, error) = texture.instance.command_encoder_finish(
        encoder,
        &wgpu_types::CommandBufferDescriptor {
            label: Some("native canvas readback".into()),
        },
        None,
    );
    if let Some((_, error)) = error {
        return Err(JsErrorBox::generic(format!(
            "finish readback encoder: {error}"
        )));
    }
    texture
        .instance
        .queue_submit(texture.queue_id, &[command_buffer])
        .map_err(|(_, error)| JsErrorBox::generic(format!("submit canvas readback: {error}")))?;

    let submission = texture
        .instance
        .buffer_map_async(
            buffer,
            0,
            None,
            wgpu_core::resource::BufferMapOperation {
                host: wgpu_core::device::HostMap::Read,
                callback: None,
            },
        )
        .map_err(|error| JsErrorBox::generic(format!("map canvas readback: {error}")))?;
    texture
        .instance
        .device_poll(
            texture.device_id,
            wgpu_types::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            },
        )
        .map_err(|error| JsErrorBox::generic(format!("poll canvas readback: {error}")))?;

    let (pointer, mapped_size) = texture
        .instance
        .buffer_get_mapped_range(buffer, 0, None)
        .map_err(|error| JsErrorBox::generic(format!("read mapped canvas buffer: {error}")))?;
    let mapped = unsafe { std::slice::from_raw_parts(pointer.as_ptr(), mapped_size as usize) };
    let mut result = Vec::with_capacity(unpadded_bytes_per_row as usize * size.height as usize);
    for row in 0..size.height as usize {
        let start = row * padded_bytes_per_row as usize;
        result.extend_from_slice(&mapped[start..start + unpadded_bytes_per_row as usize]);
    }
    texture
        .instance
        .buffer_unmap(buffer)
        .map_err(|error| JsErrorBox::generic(format!("unmap canvas buffer: {error}")))?;
    texture.instance.buffer_drop(buffer);
    Ok(result)
}

fn read_capture_canvas(
    cap: &CaptureCanvas,
    scope: &mut v8::PinScope,
) -> Result<RgbaImage, JsErrorBox> {
    let ctx_local = v8::Local::new(scope, &cap.ctx);
    let ctx = deno_core::cppgc::try_unwrap_cppgc_object::<GPUCanvasContext>(scope, ctx_local)
        .ok_or_else(|| JsErrorBox::generic("not a GPUCanvasContext"))?;
    let format = ctx
        .configuration
        .borrow()
        .as_ref()
        .map(|configuration| configuration.format.clone());
    let mut rgba = if matches!(
        format,
        Some(deno_webgpu::texture::GPUTextureFormat::Rgba16float)
    ) {
        let current = ctx.current_texture.borrow();
        let texture = current
            .as_ref()
            .ok_or_else(|| JsErrorBox::generic("canvas has no current texture"))?;
        let local = v8::Local::new(scope, texture).cast::<v8::Value>();
        let texture =
            deno_core::cppgc::try_unwrap_cppgc_object::<deno_webgpu::texture::GPUTexture>(
                scope, local,
            )
            .ok_or_else(|| JsErrorBox::generic("invalid canvas texture"))?;
        let raw = copy_texture_bytes(&texture, 8)?;
        let mut converted =
            Vec::with_capacity(texture.size.width as usize * texture.size.height as usize * 4);
        for pixel in raw.chunks_exact(8) {
            for channel in 0..4 {
                let bits = u16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]);
                let value = half::f16::from_bits(bits).to_f32();
                converted.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        RgbaImage::from_raw(texture.size.width, texture.size.height, converted)
            .ok_or_else(|| JsErrorBox::generic("invalid RGBA16F canvas readback"))?
    } else {
        ctx.copy_image_contents_to_canvas_data(scope)?;
        cap.image.borrow().to_rgba8()
    };
    if matches!(
        format,
        Some(deno_webgpu::texture::GPUTextureFormat::Bgra8unorm)
    ) {
        for pixel in rgba.chunks_mut(4) {
            pixel.swap(0, 2);
        }
    }
    Ok(rgba)
}

/// Read the page's WebGPU canvases and let Blitz paint them together with
/// the complete styled DOM into the fixed browser viewport.
#[op2]
#[buffer]
fn op_capture_png(state: &mut OpState, scope: &mut v8::PinScope) -> Result<Vec<u8>, JsErrorBox> {
    let rasters = {
        let canvases = state
            .try_borrow::<CaptureCanvases>()
            .ok_or_else(|| JsErrorBox::generic("no capture canvases"))?;
        if canvases.0.is_empty() {
            return Err(JsErrorBox::generic("no capture canvases"));
        }
        canvases
            .0
            .iter()
            .map(|canvas| {
                let native_id = canvas.native_node_id.ok_or_else(|| {
                    JsErrorBox::generic(format!(
                        "capture canvas {} is not bound to a DOM node",
                        canvas.id
                    ))
                })?;
                let image = read_capture_canvas(canvas, scope)?;
                Ok(CanvasRaster {
                    native_id,
                    width: image.width(),
                    height: image.height(),
                    rgba: image.into_raw(),
                })
            })
            .collect::<Result<Vec<_>, JsErrorBox>>()?
    };
    let rgba = state
        .borrow_mut::<BrowserDocument>()
        .render(rasters)
        .map_err(JsErrorBox::generic)?;
    let image = RgbaImage::from_raw(800, 500, rgba)
        .ok_or_else(|| JsErrorBox::generic("Blitz returned an invalid page buffer"))?;
    let img = DynamicImage::ImageRgba8(image);
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        deno_image::image::ImageFormat::Png,
    )
    .map_err(|error| JsErrorBox::generic(format!("encode png: {error}")))?;
    if let Some(slot) = state.try_borrow::<CaptureSlot>() {
        *slot.lock().unwrap() = Some(bytes.clone());
    }
    Ok(bytes)
}

#[op2(fast)]
fn op_signal_finished(state: &mut OpState) {
    state
        .borrow::<RenderFinished>()
        .0
        .store(true, std::sync::atomic::Ordering::SeqCst);
}

#[op2(fast)]
fn op_signal_ready(state: &mut OpState) {
    state
        .borrow::<RenderReady>()
        .0
        .store(true, std::sync::atomic::Ordering::SeqCst);
}

#[op2(fast)]
fn op_signal_gpu_submission(state: &mut OpState, canvas_id: u32) -> Result<(), JsErrorBox> {
    state
        .borrow::<Arc<ReadinessRegistry>>()
        .mark_canvas_submission(canvas_id as u64);
    let lifecycle = state.borrow::<Arc<RuntimeLifecycle>>();
    if lifecycle.phase() == RuntimePhase::ResourcesReady {
        lifecycle
            .transition(RuntimePhase::RendererReady)
            .and_then(|_| lifecycle.transition(RuntimePhase::Running))
            .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    }
    Ok(())
}

#[op2(fast)]
fn op_signal_gpu_phase(state: &mut OpState, phase: u32) -> Result<(), JsErrorBox> {
    let phase = match phase {
        1 => RuntimePhase::AdapterReady,
        2 => RuntimePhase::DeviceReady,
        3 => RuntimePhase::CanvasReady,
        _ => {
            return Err(JsErrorBox::generic(format!(
                "invalid GPU lifecycle phase {phase}"
            )));
        }
    };
    let lifecycle = state.borrow::<Arc<RuntimeLifecycle>>();
    if lifecycle.phase() >= phase && lifecycle.phase() <= RuntimePhase::Running {
        return Ok(());
    }
    lifecycle
        .transition(phase)
        .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    if phase == RuntimePhase::CanvasReady {
        lifecycle
            .transition(RuntimePhase::ResourcesReady)
            .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    }
    Ok(())
}

#[op2(fast)]
fn op_begin_readiness(state: &mut OpState, kind: u32) -> Result<u32, JsErrorBox> {
    let (subsystem, operation) = match kind {
        1 => (ReadinessSubsystem::Adapter, "requestAdapter"),
        2 => (ReadinessSubsystem::Device, "requestDevice"),
        3 => (ReadinessSubsystem::Pipeline, "createRenderPipelineAsync"),
        4 => (ReadinessSubsystem::Pipeline, "createComputePipelineAsync"),
        _ => {
            return Err(JsErrorBox::generic(format!(
                "invalid readiness kind {kind}"
            )));
        }
    };
    let token = state
        .borrow::<Arc<ReadinessRegistry>>()
        .begin(subsystem, operation, None);
    let id = u32::try_from(token.id())
        .map_err(|_| JsErrorBox::generic("readiness token ID overflow"))?;
    state
        .borrow_mut::<PendingReadinessTokens>()
        .0
        .insert(id, token);
    Ok(id)
}

#[op2(fast)]
fn op_finish_readiness(
    state: &mut OpState,
    token_id: u32,
    #[string] error: String,
) -> Result<(), JsErrorBox> {
    let token = state
        .borrow_mut::<PendingReadinessTokens>()
        .0
        .remove(&token_id)
        .ok_or_else(|| JsErrorBox::generic(format!("unknown readiness token {token_id}")))?;
    if error.is_empty() {
        token.complete();
    } else {
        token.fail(error);
    }
    Ok(())
}

#[op2(fast)]
fn op_set_loaded_asset_bytes(state: &mut OpState, bytes: u32) {
    state
        .borrow::<LoadedAssetBytes>()
        .0
        .store(bytes as u64, std::sync::atomic::Ordering::SeqCst);
}

#[op2(fast)]
fn op_set_fetch_state(state: &mut OpState, pending: u32, activity: u32) {
    state
        .borrow::<PendingFetches>()
        .0
        .store(pending, std::sync::atomic::Ordering::SeqCst);
    state
        .borrow::<FetchActivity>()
        .0
        .store(activity as u64, std::sync::atomic::Ordering::SeqCst);
}

/// fetch(): read a (possibly relative) URL as bytes; relative resolves against
/// the example HTML URL stored in OpState.
#[op2]
#[buffer]
async fn op_fetch_url(
    state: Rc<RefCell<OpState>>,
    #[string] url: String,
) -> Result<Vec<u8>, JsErrorBox> {
    let (base, resources) = {
        let state = state.borrow();
        (
            state.borrow::<url::Url>().clone(),
            state.borrow::<ResourceLoader>().clone(),
        )
    };
    let abs = if url.starts_with("file://") || url.starts_with("http") {
        url::Url::parse(&url)
    } else {
        base.join(&url)
    }
    .map_err(|e| JsErrorBox::generic(format!("url {url}: {e}")))?;
    resources.load(&abs).await
}

#[derive(serde::Serialize)]
struct DecodedImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

/// createImageBitmap(): decode image bytes (png/jpg/etc) -> RGBA8 {w,h,data}.
#[op2]
#[serde]
fn op_decode_image(#[buffer] bytes: &[u8]) -> Result<DecodedImage, JsErrorBox> {
    let img = deno_image::image::load_from_memory(bytes)
        .map_err(|e| JsErrorBox::generic(format!("decode: {e}")))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(DecodedImage {
        width: w,
        height: h,
        data: rgba.into_raw(),
    })
}

/// Importmap: bare specifiers -> paths relative to the example HTML.
struct ImportMap {
    exact: HashMap<String, String>,
    prefixes: Vec<(String, String)>,
}

struct BrowserModuleLoader {
    map: ImportMap,
    html_url: url::Url,
    resources: ResourceLoader,
}

impl ModuleLoader for BrowserModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> ModuleResolveResponse {
        if specifier.starts_with("ext:") || specifier.starts_with("node:") {
            return Ok(url::Url::parse(specifier).unwrap_or_else(|_| self.html_url.clone()));
        }
        if let Some(val) = self.map.exact.get(specifier) {
            return self
                .html_url
                .join(val)
                .map_err(|e| JsErrorBox::generic(e.to_string()));
        }
        for (prefix, val) in &self.map.prefixes {
            if let Some(rest) = specifier.strip_prefix(prefix.as_str()) {
                let base = self
                    .html_url
                    .join(val)
                    .map_err(|e| JsErrorBox::generic(e.to_string()))?;
                return Ok(base.join(rest).unwrap_or(base));
            }
        }
        let base = url::Url::parse(referrer).unwrap_or_else(|_| self.html_url.clone());
        base.join(specifier)
            .map_err(|e| JsErrorBox::generic(format!("resolve {specifier}: {e}")))
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _referrer: Option<&ModuleLoadReferrer>,
        _options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let specifier = module_specifier.clone();
        let resources = self.resources.clone();
        let fut = async move {
            let code = resources.load(&specifier).await?;
            Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
                &specifier,
                None,
            ))
        };
        ModuleLoadResponse::Async(fut.boxed_local())
    }
}

fn fatal(stage: &str, error: impl std::fmt::Display) -> ! {
    eprintln!("[host] {stage}:\n{error}");
    std::process::exit(1);
}

fn execute_script(runtime: &mut JsRuntime, name: &'static str, source: String) {
    runtime
        .execute_script(name, source)
        .unwrap_or_else(|error| fatal(&format!("failed to execute {name}"), error));
}

/// Evaluate an ES module while driving the event loop and preserving all JS
/// failures. deno_core 0.408's `with_event_loop_future` intentionally ignores
/// event-loop errors; additionally, a synchronous module rejection may resolve
/// `mod_evaluate` before its unhandled rejection is dispatched. The explicit
/// post-evaluation drain covers both cases and retains the original JS stack.
fn evaluate_module(
    tokio_rt: &tokio::runtime::Runtime,
    runtime: &mut JsRuntime,
    id: ModuleId,
    label: &str,
) {
    let evaluation = Box::pin(runtime.mod_evaluate(id));
    tokio_rt
        .block_on(runtime.with_event_loop_promise(
            evaluation,
            PollEventLoopOptions {
                wait_for_inspector: false,
            },
        ))
        .unwrap_or_else(|error| fatal(&format!("failed to evaluate {label}"), error));

    tokio_rt
        .block_on(runtime.run_event_loop(PollEventLoopOptions {
            wait_for_inspector: false,
        }))
        .unwrap_or_else(|error| fatal(&format!("failed after evaluating {label}"), error));
}

fn extract_between<'a>(html: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let s = html.find(start)? + start.len();
    let e = html[s..].find(end)? + s;
    Some(&html[s..e])
}

fn extract_html(html: &str) -> (ImportMap, String) {
    let im_str = extract_between(html, "<script type=\"importmap\">", "</script>")
        .or_else(|| extract_between(html, "<script type='importmap'>", "</script>"))
        .unwrap_or_default();
    let json: serde_json::Value = serde_json::from_str(im_str.trim()).unwrap_or_default();
    let mut map = ImportMap {
        exact: HashMap::new(),
        prefixes: Vec::new(),
    };
    if let Some(imports) = json.get("imports").and_then(|v| v.as_object()) {
        for (k, v) in imports {
            if let Some(val) = v.as_str() {
                if k.ends_with('/') {
                    map.prefixes.push((k.clone(), val.to_string()));
                } else {
                    map.exact.insert(k.clone(), val.to_string());
                }
            }
        }
    }
    let module = extract_between(html, "<script type=\"module\">", "</script>")
        .or_else(|| extract_between(html, "<script type='module'>", "</script>"))
        .unwrap_or_default()
        .to_string();
    (map, module)
}

pub fn run_from_args() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let base_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/threejs".to_string());
    let example = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "webgpu_clipping".to_string());
    let out = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "/tmp/browser_out.png".to_string());

    let html_path = format!("{base_dir}/examples/{example}.html");
    let html =
        std::fs::read_to_string(&html_path).unwrap_or_else(|e| panic!("read {html_path}: {e}"));
    let (importmap, module) = extract_html(&html);
    let html_url = url::Url::parse(&format!("file://{base_dir}/examples/{example}.html")).unwrap();

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _tg = tokio_rt.enter();

    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readiness = ReadinessRegistry::new();
    let lifecycle = Arc::new(RuntimeLifecycle::new());
    let loaded_asset_bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let pending_fetches = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let fetch_activity = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let resources = ResourceLoader {
        client: reqwest::Client::builder()
            .user_agent("afterglow-shell/0.1")
            .build()
            .unwrap(),
        cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
        readiness: readiness.clone(),
    };
    let mut runtime = JsRuntime::new(RuntimeOptions {
        startup_snapshot: Some(SNAPSHOT),
        module_loader: Some(Rc::new(BrowserModuleLoader {
            map: importmap,
            html_url: html_url.clone(),
            resources: resources.clone(),
        })),
        extensions: vec![
            deno_webidl::deno_webidl::init(),
            deno_web::deno_web::init(
                Arc::new(deno_web::BlobStore::default()),
                Some(html_url.clone()),
                false,
                deno_web::InMemoryBroadcastChannel::default(),
            ),
            deno_webgpu::deno_webgpu::init(),
            probe_ext::init(),
        ],
        ..Default::default()
    });
    {
        let op_state = runtime.op_state();
        let mut st = op_state.borrow_mut();
        st.put::<RenderFinished>(RenderFinished(finished.clone()));
        st.put::<RenderReady>(RenderReady(ready.clone()));
        st.put::<Arc<ReadinessRegistry>>(readiness.clone());
        st.put::<Arc<RuntimeLifecycle>>(lifecycle.clone());
        st.put::<PendingReadinessTokens>(PendingReadinessTokens(HashMap::new()));
        st.put::<LoadedAssetBytes>(LoadedAssetBytes(loaded_asset_bytes.clone()));
        st.put::<PendingFetches>(PendingFetches(pending_fetches.clone()));
        st.put::<FetchActivity>(FetchActivity(fetch_activity.clone()));
        st.put::<url::Url>(html_url.clone());
        st.put::<CaptureSlot>(Arc::new(std::sync::Mutex::new(None)));
        st.put::<CaptureCanvases>(CaptureCanvases(Vec::new()));
        st.put::<BrowserDocument>(BrowserDocument::new(800, 500));
        st.put::<ResourceLoader>(resources);
    }

    // 1. Browser primitives and DOM environment. deno_web provides the
    //    standards-compliant URL and Performance implementations; LinkeDOM
    //    provides the document and element interfaces.
    execute_script(
        &mut runtime,
        "<web-platform>",
        format!(
            r#"
                const url = Deno.core.loadExtScript("ext:deno_web/00_url.js");
                const exceptions = Deno.core.loadExtScript("ext:deno_web/01_dom_exception.js");
                const clone = Deno.core.loadExtScript("ext:deno_web/02_structured_clone.js");
                const abort = Deno.core.loadExtScript("ext:deno_web/03_abort_signal.js");
                const encoding = Deno.core.loadExtScript("ext:deno_web/08_text_encoding.js");
                const file = Deno.core.loadExtScript("ext:deno_web/09_file.js");
                const location = Deno.core.loadExtScript("ext:deno_web/12_location.js");
                const timing = Deno.core.loadExtScript("ext:deno_web/15_performance.js");
                globalThis.URL = url.URL;
                globalThis.URLSearchParams = url.URLSearchParams;
                globalThis.DOMException = exceptions.DOMException;
                globalThis.structuredClone = clone.structuredClone;
                globalThis.AbortController = abort.AbortController;
                globalThis.AbortSignal = abort.AbortSignal;
                globalThis.TextDecoder = encoding.TextDecoder;
                globalThis.TextEncoder = encoding.TextEncoder;
                globalThis.Blob = file.Blob;
                globalThis.File = file.File;
                globalThis.__blobFromObjectURL = file.blobFromObjectUrl;
                location.setLocationHref({:?});
                Object.defineProperty(globalThis, "Location", location.locationConstructorDescriptor);
                Object.defineProperty(globalThis, "location", location.locationDescriptor);
                globalThis.Performance = timing.Performance;
                globalThis.PerformanceEntry = timing.PerformanceEntry;
                globalThis.PerformanceMark = timing.PerformanceMark;
                globalThis.PerformanceMeasure = timing.PerformanceMeasure;
                globalThis.PerformanceObserver = timing.PerformanceObserver;
                globalThis.PerformanceObserverEntryList = timing.PerformanceObserverEntryList;
                globalThis.performance = timing.performance;
                globalThis.__exampleURL = {:?};
                "#,
            html_url.as_str(),
            html_url.as_str()
        ),
    );
    execute_script(
        &mut runtime,
        "<document-source>",
        format!(
            "globalThis.__documentHTML = {}; globalThis.__viewportWidth = 800; globalThis.__viewportHeight = 500;",
            serde_json::to_string(&html).unwrap(),
        ),
    );
    let dom_setup = std::fs::read_to_string(crate_root.join("dom_setup.ts")).unwrap();
    let dom_spec = url::Url::parse(&format!(
        "file://{}/dom_setup.ts",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let dom_id = tokio_rt
        .block_on(runtime.load_side_es_module_from_code(&dom_spec, dom_setup))
        .unwrap_or_else(|error| fatal("failed to load dom_setup.ts", error));
    evaluate_module(&tokio_rt, &mut runtime, dom_id, "dom_setup.ts");
    lifecycle
        .transition(RuntimePhase::EnvironmentReady)
        .unwrap_or_else(|error| fatal("browser environment lifecycle", error));
    let injection =
        std::fs::read_to_string(crate_root.join("e2e/deterministic-injection.ts")).unwrap();
    // 2. navigator.gpu + WebGPU globals (environment).
    execute_script(
        &mut runtime,
        "<gpu-setup>",
        r#"
            const { loadWebGPU } = Deno.core.loadExtScript("ext:deno_webgpu/00_init.js");
            const webgpu = loadWebGPU();
            webgpu.initGPU();
            globalThis.navigator = globalThis.navigator || {};
            globalThis.navigator.gpu = webgpu.gpu;
            for (const k of Object.keys(webgpu)) { if (k.startsWith("GPU")) globalThis[k] = webgpu[k]; }
            globalThis.GPUFeatureName = globalThis.GPUFeatureName || {
              "depthClipControl":"depth-clip-control","depth32floatStencil8":"depth32float-stencil8",
              "textureCompressionBC":"texture-compression-bc","textureCompressionETC2":"texture-compression-etc2",
              "textureCompressionASTC":"texture-compression-astc","timestampQuery":"timestamp-query",
              "indirectFirstInstance":"indirect-first-instance","shaderF16":"shader-f16",
              "rg11b10ufloatRenderable":"rg11b10ufloat-renderable","bgra8unormStorage":"bgra8unorm-storage",
              "float32Filterable":"float32-filterable","float32Blendable":"float32-blendable",
              "clipDistances":"clip-distances","dualSourceBlending":"dual-source-blending",
              "subgroups":"subgroups",
            };
            // deno_webgpu's generated enum currently omits this standardized
            // GPUFeatureName even though its wgpu backend exposes it.
            if (!("subgroups" in globalThis.GPUFeatureName)) {
              Object.defineProperty(globalThis.GPUFeatureName, "subgroups", { value: "subgroups", enumerable: true });
            }
            // Detached example init() promises are not part of module evaluation.
            // Track the real asynchronous GPU initialization operations so the
            // host cannot release the deterministic frame while requestAdapter
            // or requestDevice is still in flight.
            globalThis.__pendingGPURequests = 0;
            const trackGPURequest = (prototype, method, phase, readinessKind = phase) => {
              const original = prototype[method];
              prototype[method] = async function (...args) {
                globalThis.__pendingGPURequests++;
                const token = Deno.core.ops.op_begin_readiness(readinessKind);
                try {
                  const result = await original.apply(this, args);
                  Deno.core.ops.op_finish_readiness(token, "");
                  if (phase !== 0) Deno.core.ops.op_signal_gpu_phase(phase);
                  return result;
                } catch (error) {
                  Deno.core.ops.op_finish_readiness(token, String(error?.stack || error));
                  throw error;
                } finally {
                  globalThis.__pendingGPURequests--;
                }
              };
            };
            trackGPURequest(GPU.prototype, "requestAdapter", 1);
            trackGPURequest(GPUAdapter.prototype, "requestDevice", 2);
            trackGPURequest(GPUDevice.prototype, "createRenderPipelineAsync", 0, 3);
            trackGPURequest(GPUDevice.prototype, "createComputePipelineAsync", 0, 4);
            const configure = GPUCanvasContext.prototype.configure;
            GPUCanvasContext.prototype.configure = function (...args) {
              const result = configure.apply(this, args);
              Deno.core.ops.op_signal_gpu_phase(3);
              return result;
            };
            const acquiredCanvasIds = new Set();
            const getCurrentTexture = GPUCanvasContext.prototype.getCurrentTexture;
            GPUCanvasContext.prototype.getCurrentTexture = function (...args) {
              const texture = getCurrentTexture.apply(this, args);
              const id = globalThis.__gpuCanvasId(this);
              if (id !== 0) acquiredCanvasIds.add(id);
              return texture;
            };
            const submit = GPUQueue.prototype.submit;
            GPUQueue.prototype.submit = function (...args) {
              const result = submit.apply(this, args);
              for (const id of acquiredCanvasIds) Deno.core.ops.op_signal_gpu_submission(id);
              acquiredCanvasIds.clear();
              return result;
            };
            const __flipY = (d, w, h) => { const r = w*4, o = new Uint8Array(d.length); for (let y=0;y<h;y++) o.set(d.subarray((h-1-y)*r,(h-1-y)*r+r),y*r); return o; };
            const __pad256 = (d, w, h) => { const r=w*4, p=Math.ceil(r/256)*256; if(p===r) return d; const o=new Uint8Array(p*h); for(let y=0;y<h;y++) o.set(d.subarray(y*r,y*r+r),y*p); return o; };
            if (globalThis.GPUQueue && !GPUQueue.prototype.copyExternalImageToTexture) {
              GPUQueue.prototype.copyExternalImageToTexture = function (source, dest, size) {
                const img = source.source; let data = img.data; const w = img.width, h = img.height;
                if (source.flipY) data = __flipY(data, w, h);
                if (dest.premultipliedAlpha) { const c = new Uint8Array(data); for (let i=0;i<c.length;i+=4){const a=c[i+3]/255;c[i]=c[i]*a;c[i+1]=c[i+1]*a;c[i+2]=c[i+2]*a;} data=c; }
                const padded = __pad256(data, w, h);
                this.writeTexture({texture: dest.texture, mipLevel: dest.mipLevel||0, origin: dest.origin||{x:0,y:0,z:0}}, padded, {offset:0, bytesPerRow: Math.ceil(w*4/256)*256, rowsPerImage: h}, size);
              };
            }
            "#
        .to_string(),
    );

    // Determinism is installed only after every environment subsystem is
    // initialized, so runtime setup cannot consume the example's seeded RNG.
    execute_script(&mut runtime, "<deterministic-injection>", injection);
    execute_script(
        &mut runtime,
        "<deterministic-random-policy>",
        r#"
        const seededRandom = Math.random;
        Math.random = () => {
          const caller = new Error().stack.split('\n')[2]?.trim();
          // three.js's canonical screenshot harness rewrites generateUUID to
          // Math._random so UUID allocation cannot perturb scene randomness.
          return caller?.includes('generateUUID') ? Math._random() : seededRandom();
        };
        "#
        .to_string(),
    );
    let harness_spec = url::Url::parse("file:///afterglow-shell/e2e-harness.mjs").unwrap();
    let harness_id = tokio_rt
        .block_on(
            runtime.load_side_es_module_from_code(
                &harness_spec,
                r#"
            import { Backend } from 'three';
            Object.defineProperty(Backend.prototype, 'trackTimestamp', {
              configurable: true,
              get: () => false,
              set: () => {},
            });
            "#
                .to_string(),
            ),
        )
        .unwrap_or_else(|error| fatal("failed to load e2e harness policy", error));
    evaluate_module(&tokio_rt, &mut runtime, harness_id, "e2e harness policy");

    execute_script(
        &mut runtime,
        "<render-readiness>",
        r#"
        globalThis.__pendingAnimationFrames = 0;
        const deterministicRequestAnimationFrame = globalThis.requestAnimationFrame;
        globalThis.requestAnimationFrame = (callback) => {
          globalThis.__pendingAnimationFrames++;
          return deterministicRequestAnimationFrame((time) => {
            globalThis.__pendingAnimationFrames--;
            callback(time);
          });
        };
        "#
        .to_string(),
    );

    // 3. Load and evaluate the example module verbatim. Use
    // with_event_loop_promise rather than with_event_loop_future: the latter
    // intentionally discards event-loop errors in deno_core 0.408, hiding
    // module exceptions as successful evaluations.
    let id = tokio_rt
        .block_on(runtime.load_side_es_module_from_code(&html_url, module))
        .unwrap_or_else(|error| fatal(&format!("failed to load {example}.html module"), error));
    evaluate_module(
        &tokio_rt,
        &mut runtime,
        id,
        &format!("{example}.html module"),
    );
    let clean_page = std::fs::read_to_string(crate_root.join("e2e/clean-page.ts"))
        .unwrap_or_else(|error| fatal("failed to read e2e/clean-page.ts", error));
    execute_script(&mut runtime, "<clean-page>", clean_page);
    eprintln!("[host] example module evaluated and page cleaned");

    // Async asset/worker initialization may continue after module evaluation.
    // Wait until three.js actually requests its first deterministic frame.
    let initialization_start = std::time::Instant::now();
    while !ready.load(std::sync::atomic::Ordering::SeqCst) {
        execute_script(
            &mut runtime,
            "<ready-poll>",
            "if (globalThis.__pendingAnimationFrames > 0 && globalThis.__pendingFetches === 0 && globalThis.__pendingGPURequests === 0) Deno.core.ops.op_signal_ready()"
                .to_string(),
        );
        execute_script(
            &mut runtime,
            "<initialization-tick>",
            "globalThis.__tick()".to_string(),
        );
        tokio_rt
            .block_on(runtime.run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
            }))
            .unwrap_or_else(|error| fatal("initialization event loop failed", error));
        if initialization_start.elapsed().as_secs() > 30 {
            execute_script(
                &mut runtime,
                "<initialization-timeout-state>",
                r#"Deno.core.ops.op_probe_log(
                  `initialization timeout state: pendingRAF=${globalThis.__pendingAnimationFrames}, ` +
                  `pendingGPU=${globalThis.__pendingGPURequests}, gpuCanvases=${globalThis.__gpuCanvasCount}, ` +
                  `timers=${globalThis.__timerCount?.() ?? "n/a"}`
                )"#
                .to_string(),
            );
            fatal(
                "initialization timed out after 30 seconds",
                "the example never requested an animation frame",
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    // Match waitForNetworkIdle({ idleTime: 2000 }): the idle deadline starts
    // only when no fetch is in flight and resets after every fetch start or
    // completion. A fixed sleep from the first idle observation is incorrect
    // for loaders that enqueue follow-up resources from decode continuations.
    let mut observed_fetch_activity = fetch_activity.load(std::sync::atomic::Ordering::SeqCst);
    let mut network_idle_start = std::time::Instant::now();
    loop {
        execute_script(
            &mut runtime,
            "<network-idle-tick>",
            "globalThis.__tick()".to_string(),
        );
        tokio_rt
            .block_on(runtime.run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
            }))
            .unwrap_or_else(|error| fatal("network-idle event loop failed", error));

        let current_activity = fetch_activity.load(std::sync::atomic::Ordering::SeqCst);
        let current_pending = pending_fetches.load(std::sync::atomic::Ordering::SeqCst);
        if current_pending != 0 || current_activity != observed_fetch_activity {
            observed_fetch_activity = current_activity;
            network_idle_start = std::time::Instant::now();
        } else if network_idle_start.elapsed() >= std::time::Duration::from_secs(2) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    // The official harness then allows one second per downloaded MiB for CPU
    // decode/parse continuations. In this native host ESM parsing itself has
    // already completed synchronously, so only fetch()-loaded asset bytes need
    // this post-idle allowance.
    let loaded_megabytes =
        loaded_asset_bytes.load(std::sync::atomic::Ordering::SeqCst) as f64 / (1024.0 * 1024.0);
    let parse_deadline =
        std::time::Instant::now() + std::time::Duration::from_secs_f64(loaded_megabytes);
    while std::time::Instant::now() < parse_deadline {
        execute_script(
            &mut runtime,
            "<asset-parse-tick>",
            "globalThis.__tick()".to_string(),
        );
        tokio_rt
            .block_on(runtime.run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
            }))
            .unwrap_or_else(|error| fatal("asset-parse event loop failed", error));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    // 4. kick the deterministic one-frame render.
    execute_script(
        &mut runtime,
        "<start>",
        "window._renderStarted = true; window.__runDeterministicFrame(); window.__setDeterministicTimerTime(1000); window.__tick()".to_string(),
    );
    // 5. drive __tick + event loop until _renderFinished.
    let start = std::time::Instant::now();
    while !finished.load(std::sync::atomic::Ordering::SeqCst) {
        execute_script(
            &mut runtime,
            "<poll>",
            "if (globalThis._renderFinished === true) Deno.core.ops.op_signal_finished()"
                .to_string(),
        );
        execute_script(&mut runtime, "<tick>", "globalThis.__tick()".to_string());
        tokio_rt
            .block_on(runtime.run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
            }))
            .unwrap_or_else(|error| fatal("event loop failed", error));
        execute_script(
            &mut runtime,
            "<post-event-loop-poll>",
            "if (globalThis._renderFinished === true) Deno.core.ops.op_signal_finished()"
                .to_string(),
        );
        if finished.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        if start.elapsed().as_secs() > 30 {
            execute_script(
                &mut runtime,
                "<timeout-state>",
                r#"Deno.core.ops.op_probe_log(
                  `render timeout state: started=${globalThis._renderStarted}, ` +
                  `finished=${globalThis._renderFinished}, timers=${globalThis.__timerCount?.() ?? "n/a"}`
                )"#
                .to_string(),
            );
            fatal(
                "render timed out after 30 seconds",
                "_renderFinished was never set",
            );
        }
    }

    // The deterministic rAF wrapper marks the callback finished immediately,
    // while WebGPURenderer may continue its first asynchronous initialization
    // and render after the callback returns. A real queue submission is the
    // renderer-ready boundary: unlike a sleep or pending-promise heuristic it
    // proves that a configured canvas produced GPU work.
    let renderer_ready_start = std::time::Instant::now();
    loop {
        let has_canvas = !runtime
            .op_state()
            .borrow()
            .borrow::<CaptureCanvases>()
            .0
            .is_empty();
        if has_canvas && readiness.renderer_ready() {
            break;
        }
        execute_script(
            &mut runtime,
            "<renderer-ready-tick>",
            "globalThis.__tick()".to_string(),
        );
        tokio_rt
            .block_on(runtime.run_event_loop(PollEventLoopOptions {
                wait_for_inspector: false,
            }))
            .unwrap_or_else(|error| fatal("renderer-ready event loop failed", error));
        if renderer_ready_start.elapsed().as_secs() > 30 {
            fatal(
                "renderer readiness timed out after 30 seconds",
                format!(
                    "capture canvases={has_canvas}, GPU submissions={}",
                    readiness.gpu_submissions()
                ),
            );
        }
        std::thread::yield_now();
    }

    // 6. capture.
    execute_script(
        &mut runtime,
        "<capture>",
        "globalThis.__syncBrowserDocument(true); Deno.core.ops.op_capture_png()".to_string(),
    );
    let png = runtime.op_state().borrow().borrow::<CaptureSlot>().clone();
    let png = png.lock().unwrap().clone().unwrap_or_default();
    std::fs::write(&out, &png).expect("write png");
    eprintln!("[host] saved {out} ({} bytes)", png.len());
}
