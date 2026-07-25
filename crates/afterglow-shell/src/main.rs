use std::cell::RefCell;
use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use afterglow_shell::runtime::{MonotonicClock, RuntimeClock, RuntimeLifecycle, RuntimePhase};
use deno_core::v8;
use deno_core::{
    FsModuleLoader, JsRuntime, ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse,
    ModuleLoader, ModuleResolveResponse, ModuleSource, ModuleSourceCode, ModuleSpecifier,
    ModuleType, OpState, PollEventLoopOptions, ResolutionKind, RuntimeOptions, op2,
};
use deno_error::JsErrorBox;
use deno_webgpu::canvas::{
    ContextData, GPUCanvasContext, SurfaceData, create as create_canvas_context,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::application::ApplicationHandler;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, CursorIcon, Window, WindowId};

static SNAPSHOT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/SNAPSHOT.bin"));

fn dom_physical_code(key: PhysicalKey) -> String {
    match key {
        // winit's KeyCode names follow the UI Events `KeyboardEvent.code`
        // vocabulary; formatting PhysicalKey itself adds the non-standard
        // `Code(...)` wrapper.
        PhysicalKey::Code(code) => format!("{code:?}"),
        PhysicalKey::Unidentified(_) => "Unidentified".to_string(),
    }
}

struct GpuHudPresenter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

impl GpuHudPresenter {
    fn create_texture(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Blitz Vello GPU HUD"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    unsafe fn new(
        instance: Arc<wgpu_core::global::Global>,
        device_id: wgpu_core::id::DeviceId,
        queue_id: wgpu_core::id::QueueId,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let features = instance.device_features(device_id);
        let (device, queue) =
            unsafe { wgpu::Device::from_shared_core(instance, device_id, queue_id, features) };
        let renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                use_cpu: false,
                num_init_threads: None,
                antialiasing_support: vello::AaSupport::all(),
                pipeline_cache: None,
            },
        )
        .map_err(|error| format!("create Vello GPU renderer: {error}"))?;
        let (texture, texture_view) = Self::create_texture(&device, width, height);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Blitz HUD sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blitz HUD compositor"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var hud_sampler: sampler;
@group(0) @binding(1) var hud_texture: texture_2d<f32>;
struct VertexOutput { @builtin(position) position: vec4f, @location(0) uv: vec2f }
@vertex fn vertex_main(@builtin(vertex_index) index: u32) -> VertexOutput {
  var positions = array<vec2f, 3>(vec2f(-1, -1), vec2f(3, -1), vec2f(-1, 3));
  var uvs = array<vec2f, 3>(vec2f(0, 1), vec2f(2, 1), vec2f(0, -1));
  var output: VertexOutput;
  output.position = vec4f(positions[index], 0, 1);
  output.uv = uvs[index];
  return output;
}
@fragment fn fragment_main(input: VertexOutput) -> @location(0) vec4f {
  return textureSample(hud_texture, hud_sampler, input.uv);
}
"#
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blitz HUD compositor"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blitz HUD compositor"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
            ],
        });
        Ok(Self {
            device,
            queue,
            renderer,
            texture,
            texture_view,
            sampler,
            pipeline,
            bind_group,
            width,
            height,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        (self.texture, self.texture_view) = Self::create_texture(&self.device, width, height);
        self.bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blitz HUD compositor"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.texture_view),
                },
            ],
        });
        self.width = width;
        self.height = height;
    }

    fn render_and_composite(
        &mut self,
        hud_scene: Option<afterglow_shell::native_browser::HudGpuScene>,
        surface_texture: &wgpu::Texture,
    ) -> Result<(), String> {
        if let Some(hud) = hud_scene {
            self.resize(hud.width, hud.height);
            self.renderer
                .render_to_texture(
                    &self.device,
                    &self.queue,
                    &hud.scene,
                    &self.texture_view,
                    &vello::RenderParams {
                        base_color: vello::peniko::Color::TRANSPARENT,
                        width: hud.width,
                        height: hud.height,
                        antialiasing_method: vello::AaConfig::Msaa16,
                    },
                )
                .map_err(|error| format!("render Blitz HUD on GPU: {error}"))?;
        }
        let surface_view = surface_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Blitz HUD compositor"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Blitz HUD compositor"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

struct NativeSurface {
    data: Rc<RefCell<SurfaceData>>,
    context: Option<v8::Global<v8::Value>>,
    canvas_id: Option<u32>,
    native_node_id: Option<u64>,
    hud_presenter: Option<GpuHudPresenter>,
}

struct EngineReady(Arc<AtomicBool>);
struct DeviceLoss(Arc<std::sync::Mutex<Option<String>>>);

struct NativePointerLock {
    window: Arc<Window>,
    locked: Arc<AtomicBool>,
}

struct NativeAnimationFrames {
    referenced: bool,
    pending: u32,
    high_water: u32,
    overflows: u64,
    batches: u64,
    callbacks: u64,
}

struct NativePresentationStats {
    attempts: u64,
    presented: u64,
}

#[derive(Clone, Copy, Debug)]
enum HostEvent {
    RuntimeWake,
}

struct RuntimeWake {
    proxy: EventLoopProxy<HostEvent>,
    queued: AtomicBool,
}

impl RuntimeWake {
    fn clear(&self) {
        self.queued.store(false, Ordering::Release);
    }

    fn signal(&self) {
        if !self.queued.swap(true, Ordering::AcqRel) {
            let _ = self.proxy.send_event(HostEvent::RuntimeWake);
        }
    }
}

impl Wake for RuntimeWake {
    fn wake(self: Arc<Self>) {
        self.signal();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.signal();
    }
}

type GameEvaluation = Pin<Box<dyn Future<Output = Result<(), String>>>>;

struct PendingResize {
    width: u32,
    height: u32,
    scale_factor: f64,
    deadline: Instant,
}

impl PendingResize {
    const QUIET_PERIOD: Duration = Duration::from_millis(175);

    fn trailing(width: u32, height: u32, scale_factor: f64, now: Instant) -> Self {
        Self {
            width,
            height,
            scale_factor,
            deadline: now + Self::QUIET_PERIOD,
        }
    }

    fn is_due(&self, now: Instant) -> bool {
        now >= self.deadline
    }
}

#[op2(fast)]
fn op_engine_now(state: &mut OpState) -> f64 {
    state.borrow::<Arc<MonotonicClock>>().now_millis()
}

#[op2(fast)]
fn op_animation_frame_requested(state: &mut OpState, pending: u32) {
    let tracker = state.external_ops_tracker.clone();
    let frames = state.borrow_mut::<NativeAnimationFrames>();
    frames.pending = pending;
    frames.high_water = frames.high_water.max(pending);
    if !frames.referenced {
        tracker.ref_op();
        frames.referenced = true;
    }
}

#[op2(fast)]
fn op_animation_frame_empty(state: &mut OpState) {
    let tracker = state.external_ops_tracker.clone();
    let frames = state.borrow_mut::<NativeAnimationFrames>();
    frames.pending = 0;
    if frames.referenced {
        tracker.unref_op();
        frames.referenced = false;
    }
}

#[op2(fast)]
fn op_animation_frame_overflow(state: &mut OpState) {
    let frames = state.borrow_mut::<NativeAnimationFrames>();
    frames.overflows = frames.overflows.saturating_add(1);
}

#[op2(fast)]
fn op_animation_frame_drained(state: &mut OpState, callbacks: u32) {
    let frames = state.borrow_mut::<NativeAnimationFrames>();
    frames.batches = frames.batches.saturating_add(1);
    frames.callbacks = frames.callbacks.saturating_add(u64::from(callbacks));
}

#[op2]
fn op_create_capture_canvas(
    state: &mut OpState,
    scope: &mut v8::PinScope,
    canvas_id: u32,
    _width: u32,
    _height: u32,
) -> Result<v8::Global<v8::Value>, JsErrorBox> {
    if let Some(context) = state.borrow::<NativeSurface>().context.as_ref() {
        let registered_id = state.borrow::<NativeSurface>().canvas_id;
        if registered_id != Some(canvas_id) {
            return Err(JsErrorBox::generic(
                "the native presenter supports one surface-backed WebGPU canvas",
            ));
        }
        return Ok(context.clone());
    }
    let surface_data = state.borrow::<NativeSurface>().data.clone();
    let canvas = v8::Object::new(scope);
    let canvas = v8::Global::new(scope, canvas);
    let context = create_canvas_context(
        None,
        canvas,
        ContextData::Surface(surface_data),
        scope,
        v8::undefined(scope).into(),
        "",
        "webgpu",
    )?;
    {
        let surface = state.borrow_mut::<NativeSurface>();
        surface.context = Some(context.clone());
        surface.canvas_id = Some(canvas_id);
    }
    state
        .borrow::<Arc<RuntimeLifecycle>>()
        .transition(RuntimePhase::CanvasReady)
        .map_err(|error| JsErrorBox::generic(error.to_string()))?;
    Ok(context)
}

#[op2(fast)]
fn op_bind_canvas_node(
    state: &mut OpState,
    canvas_id: u32,
    native_node_id: u32,
) -> Result<(), JsErrorBox> {
    {
        let surface = state.borrow_mut::<NativeSurface>();
        if surface.canvas_id != Some(canvas_id) {
            return Err(JsErrorBox::generic(format!(
                "unknown native canvas {canvas_id}"
            )));
        }
        surface.native_node_id = Some(native_node_id as u64);
    }
    afterglow_shell::native_browser::set_surface_canvas_node(state, native_node_id as u64);
    Ok(())
}

fn resize_native_surface(
    state: &mut OpState,
    scope: &mut v8::PinScope,
    width: u32,
    height: u32,
) -> Result<(), JsErrorBox> {
    let context = {
        let surface = state.borrow_mut::<NativeSurface>();
        let mut data = surface.data.borrow_mut();
        data.width = width.max(1);
        data.height = height.max(1);
        surface.context.clone()
    };
    if let Some(context) = context {
        let local = v8::Local::new(scope, &context);
        let context = deno_core::cppgc::try_unwrap_cppgc_object::<GPUCanvasContext>(scope, local)
            .ok_or_else(|| JsErrorBox::generic("native surface context is invalid"))?;
        context.resize(scope);
    }
    Ok(())
}

#[op2(fast)]
fn op_request_pointer_lock(state: &mut OpState) -> Result<(), JsErrorBox> {
    let pointer_lock = state.borrow::<NativePointerLock>();
    pointer_lock
        .window
        .set_cursor_grab(CursorGrabMode::Locked)
        .map_err(|error| JsErrorBox::generic(format!("lock native pointer: {error}")))?;
    pointer_lock.window.set_cursor_visible(false);
    pointer_lock.locked.store(true, Ordering::Release);
    Ok(())
}

#[op2(fast)]
fn op_exit_pointer_lock(state: &mut OpState) -> Result<(), JsErrorBox> {
    let pointer_lock = state.borrow::<NativePointerLock>();
    pointer_lock
        .window
        .set_cursor_grab(CursorGrabMode::None)
        .map_err(|error| JsErrorBox::generic(format!("release native pointer: {error}")))?;
    pointer_lock.window.set_cursor_visible(true);
    pointer_lock.locked.store(false, Ordering::Release);
    Ok(())
}

#[op2(fast)]
fn op_resize_canvas(
    state: &mut OpState,
    scope: &mut v8::PinScope,
    canvas_id: u32,
    width: u32,
    height: u32,
) -> Result<(), JsErrorBox> {
    if state.borrow::<NativeSurface>().canvas_id != Some(canvas_id) {
        return Err(JsErrorBox::generic(format!(
            "unknown native canvas {canvas_id}"
        )));
    }
    resize_native_surface(state, scope, width, height)
}

fn present_surface(state: &mut OpState, scope: &mut v8::PinScope) -> Result<bool, JsErrorBox> {
    let context = state
        .borrow::<NativeSurface>()
        .context
        .clone()
        .ok_or_else(|| JsErrorBox::generic("native surface canvas has not been created"))?;
    let local = v8::Local::new(scope, &context);
    let context = deno_core::cppgc::try_unwrap_cppgc_object::<GPUCanvasContext>(scope, local)
        .ok_or_else(|| JsErrorBox::generic("native surface context is invalid"))?;
    let hud_scene = afterglow_shell::native_browser::take_gpu_hud_scene(state)?;
    let (instance, texture_id, device_id, queue_id, format, size) = {
        let current = context.current_texture.borrow();
        let texture = current
            .as_ref()
            .ok_or_else(|| JsErrorBox::generic("surface has no acquired texture"))?;
        let texture = v8::Local::new(scope, texture).cast::<v8::Value>();
        let texture =
            deno_core::cppgc::try_unwrap_cppgc_object::<deno_webgpu::texture::GPUTexture>(
                scope, texture,
            )
            .ok_or_else(|| JsErrorBox::generic("surface texture is invalid"))?;
        (
            texture.instance.clone(),
            texture.id,
            texture.device_id,
            texture.queue_id,
            wgpu_types::TextureFormat::from(texture.format.clone()),
            texture.size,
        )
    };
    let surface_texture = unsafe {
        wgpu::Texture::from_shared_core(
            instance.clone(),
            texture_id,
            &wgpu::TextureDescriptor {
                label: Some("shared JavaScript surface texture"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        )
    };
    let surface = state.borrow_mut::<NativeSurface>();
    if surface.hud_presenter.is_none() {
        surface.hud_presenter = Some(unsafe {
            GpuHudPresenter::new(
                instance,
                device_id,
                queue_id,
                format,
                size.width,
                size.height,
            )
            .map_err(JsErrorBox::generic)?
        });
    }
    surface
        .hud_presenter
        .as_mut()
        .unwrap()
        .render_and_composite(hud_scene, &surface_texture)
        .map_err(JsErrorBox::generic)?;
    context.present()
}

#[op2(fast)]
fn op_present_surface(state: &mut OpState, scope: &mut v8::PinScope) -> Result<bool, JsErrorBox> {
    present_surface(state, scope)
}

#[op2(fast)]
fn op_try_present_surface(
    state: &mut OpState,
    scope: &mut v8::PinScope,
) -> Result<bool, JsErrorBox> {
    {
        let stats = state.borrow_mut::<NativePresentationStats>();
        stats.attempts = stats.attempts.saturating_add(1);
    }
    let Some(context) = state.borrow::<NativeSurface>().context.clone() else {
        return Ok(false);
    };
    let local = v8::Local::new(scope, &context);
    let context = deno_core::cppgc::try_unwrap_cppgc_object::<GPUCanvasContext>(scope, local)
        .ok_or_else(|| JsErrorBox::generic("native surface context is invalid"))?;
    if context.current_texture.borrow().is_none() {
        return Ok(false);
    }
    let presented = present_surface(state, scope)?;
    if presented {
        let stats = state.borrow_mut::<NativePresentationStats>();
        stats.presented = stats.presented.saturating_add(1);
    }
    Ok(presented)
}

#[op2(fast)]
fn op_resize_surface(
    state: &mut OpState,
    scope: &mut v8::PinScope,
    width: u32,
    height: u32,
) -> Result<(), JsErrorBox> {
    resize_native_surface(state, scope, width, height)
}

#[op2(fast)]
fn op_adapter_report(state: &mut OpState, #[string] report: String) -> Result<(), JsErrorBox> {
    eprintln!("WebGPU adapter: {report}");
    state
        .borrow::<Arc<RuntimeLifecycle>>()
        .transition(RuntimePhase::AdapterReady)
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2(fast)]
fn op_device_ready(state: &mut OpState) -> Result<(), JsErrorBox> {
    state
        .borrow::<Arc<RuntimeLifecycle>>()
        .transition(RuntimePhase::DeviceReady)
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2(fast)]
fn op_device_lost(
    state: &mut OpState,
    #[string] reason: String,
    #[string] message: String,
) -> Result<(), JsErrorBox> {
    let detail = if message.is_empty() {
        reason
    } else {
        format!("{reason}: {message}")
    };
    *state.borrow::<DeviceLoss>().0.lock().unwrap() = Some(detail);
    state
        .borrow::<Arc<RuntimeLifecycle>>()
        .transition(RuntimePhase::DeviceLost)
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2(fast)]
fn op_engine_ready(state: &mut OpState) -> Result<(), JsErrorBox> {
    state
        .borrow::<EngineReady>()
        .0
        .store(true, Ordering::Release);
    let lifecycle = state.borrow::<Arc<RuntimeLifecycle>>();
    lifecycle
        .transition(RuntimePhase::ResourcesReady)
        .and_then(|_| lifecycle.transition(RuntimePhase::RendererReady))
        .and_then(|_| lifecycle.transition(RuntimePhase::Running))
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

deno_core::extension!(
    engine_ext,
    ops = [
        op_engine_now,
        op_animation_frame_requested,
        op_animation_frame_empty,
        op_animation_frame_overflow,
        op_animation_frame_drained,
        op_create_capture_canvas,
        op_bind_canvas_node,
        op_resize_canvas,
        op_request_pointer_lock,
        op_exit_pointer_lock,
        op_present_surface,
        op_try_present_surface,
        op_resize_surface,
        op_adapter_report,
        op_device_ready,
        op_device_lost,
        op_engine_ready,
        afterglow_shell::rpc_bridge::op_afterglow_rpc_call,
        afterglow_shell::rpc_bridge::op_afterglow_rpc_call_async,
        afterglow_shell::rpc_bridge::op_afterglow_arena_view,
        afterglow_shell::rpc_bridge::op_native_asset_size,
        afterglow_shell::rpc_bridge::op_native_asset_read_copy,
        afterglow_shell::rpc_bridge::op_native_asset_read_handle,
        afterglow_shell::rpc_bridge::op_native_asset_read_many_handle,
    ],
);

const ANIMATION_FRAME_BOOTSTRAP: &str = include_str!("../raf.ts");
const SCHEDULER_BOOTSTRAP: &str = include_str!("../scheduler.ts");

const WEB_PLATFORM_BOOTSTRAP: &str = r#"
const url = Deno.core.loadExtScript("ext:deno_web/00_url.js");
const exceptions = Deno.core.loadExtScript("ext:deno_web/01_dom_exception.js");
const clone = Deno.core.loadExtScript("ext:deno_web/02_structured_clone.js");
const timers = Deno.core.loadExtScript("ext:deno_web/02_timers.js");
const abort = Deno.core.loadExtScript("ext:deno_web/03_abort_signal.js");
const encoding = Deno.core.loadExtScript("ext:deno_web/08_text_encoding.js");
const file = Deno.core.loadExtScript("ext:deno_web/09_file.js");
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
globalThis.Performance = timing.Performance;
globalThis.PerformanceEntry = timing.PerformanceEntry;
globalThis.PerformanceMark = timing.PerformanceMark;
globalThis.PerformanceMeasure = timing.PerformanceMeasure;
globalThis.PerformanceObserver = timing.PerformanceObserver;
globalThis.PerformanceObserverEntryList = timing.PerformanceObserverEntryList;
globalThis.performance = timing.performance;
globalThis.__afterglowSchedulerNative = { defer: timers.defer };
globalThis.__afterglowTimersNative = {
  setTimeout: timers.setTimeout,
  clearTimeout: timers.clearTimeout,
  setInterval: timers.setInterval,
  clearInterval: timers.clearInterval,
};
"#;

const ENGINE_BOOTSTRAP: &str = r#"
const { loadWebGPU } = Deno.core.loadExtScript("ext:deno_webgpu/00_init.js");
const webgpu = loadWebGPU();
webgpu.initGPU();
globalThis.navigator = globalThis.navigator || {};
globalThis.navigator.gpu = webgpu.gpu;
for (const key of Object.keys(webgpu)) {
  if (key.startsWith("GPU")) globalThis[key] = webgpu[key];
}
// deno_webgpu does not expose WebGPU's external-image copy entry point. Back it
// with queue.writeTexture so real TextureLoader and CanvasTexture sources upload
// their decoded RGBA pixels on the same device.
if (globalThis.GPUQueue && !GPUQueue.prototype.copyExternalImageToTexture) {
  const flipRows = (data, width, height) => {
    const stride = width * 4;
    const output = new Uint8Array(data.length);
    for (let y = 0; y < height; y++) {
      output.set(data.subarray((height - y - 1) * stride, (height - y) * stride), y * stride);
    }
    return output;
  };
  const padRows = (data, width, height) => {
    const stride = width * 4;
    const paddedStride = Math.ceil(stride / 256) * 256;
    if (stride === paddedStride) return { data, bytesPerRow: stride };
    const output = new Uint8Array(paddedStride * height);
    for (let y = 0; y < height; y++) {
      output.set(data.subarray(y * stride, (y + 1) * stride), y * paddedStride);
    }
    return { data: output, bytesPerRow: paddedStride };
  };
  GPUQueue.prototype.copyExternalImageToTexture = function (source, destination, copySize) {
    const image = source.source;
    const width = Number(image.width);
    const height = Number(image.height);
    let data = image.data ?? image._canvas2d?.data;
    if (!data || !Number.isFinite(width) || !Number.isFinite(height)) {
      throw new TypeError('copyExternalImageToTexture requires a decoded image or 2D canvas source');
    }
    data = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    if (source.flipY) data = flipRows(data, width, height);
    if (destination.premultipliedAlpha) {
      data = data.slice();
      for (let i = 0; i < data.length; i += 4) {
        const alpha = data[i + 3] / 255;
        data[i] = Math.round(data[i] * alpha);
        data[i + 1] = Math.round(data[i + 1] * alpha);
        data[i + 2] = Math.round(data[i + 2] * alpha);
      }
    }
    const upload = padRows(data, width, height);
    this.writeTexture(
      {
        texture: destination.texture,
        mipLevel: destination.mipLevel ?? 0,
        origin: destination.origin ?? { x: 0, y: 0, z: 0 },
        aspect: destination.aspect ?? 'all',
      },
      upload.data,
      { offset: 0, bytesPerRow: upload.bytesPerRow, rowsPerImage: height },
      copySize,
    );
  };
}
globalThis.self = globalThis;
globalThis.window = globalThis;
globalThis.performance = { now: () => Deno.core.ops.op_engine_now() };
globalThis.__afterglowAnimationFrameNative = {
  requested: (pending) => Deno.core.ops.op_animation_frame_requested(pending),
  empty: () => Deno.core.ops.op_animation_frame_empty(),
  overflow: () => Deno.core.ops.op_animation_frame_overflow(),
  drained: (callbacks) => Deno.core.ops.op_animation_frame_drained(callbacks),
  report: (error) => Deno.core.reportUnhandledException(error),
};

document.documentElement.style.cssText = 'margin:0;width:100%;height:100%;background:transparent;overflow:hidden';
document.body.style.cssText = 'margin:0;width:100%;height:100%;background:transparent;overflow:hidden';
const canvas = document.createElement('canvas');
canvas.id = 'game-surface';
canvas.width = globalThis.__surfaceWidth;
canvas.height = globalThis.__surfaceHeight;
canvas.style.cssText = 'position:fixed;inset:0;width:100%;height:100%;display:block';
if (!globalThis.__officialExample) document.body.appendChild(canvas);
globalThis.engineCanvas = canvas;
globalThis.dispatchNativeInput = (type, init = {}) => {
  type = String(type);
  if (type.startsWith('pointer')) return globalThis.__dispatchBrowserPointerEvent(type, init);
  if (type === 'wheel') return globalThis.__dispatchBrowserWheelEvent(init);
  if (type === 'keydown' || type === 'keyup') return globalThis.__dispatchBrowserKeyboardEvent(type, init);
  const event = new Event(type, { bubbles: false, cancelable: false });
  Object.assign(event, init);
  return globalThis.dispatchEvent(event);
};
globalThis.resizeEngineCanvas = (width, height, scaleFactor = devicePixelRatio) => {
  width = Math.max(1, Number(width));
  height = Math.max(1, Number(height));
  globalThis.devicePixelRatio = Number(scaleFactor) || 1;
  globalThis.__surfaceWidth = width;
  globalThis.__surfaceHeight = height;
  globalThis.__viewportWidth = width / globalThis.devicePixelRatio;
  globalThis.__viewportHeight = height / globalThis.devicePixelRatio;
  globalThis.innerWidth = globalThis.__viewportWidth;
  globalThis.innerHeight = globalThis.__viewportHeight;
  canvas.width = width;
  canvas.height = height;
  Deno.core.ops.op_resize_surface(width, height);
  Deno.core.ops.op_resize_hud(width, height, globalThis.devicePixelRatio);
};

const requestAdapter = GPU.prototype.requestAdapter;
GPU.prototype.requestAdapter = async function (...args) {
  const adapter = await requestAdapter.apply(this, args);
  if (adapter) Deno.core.ops.op_adapter_report(JSON.stringify({
    info: {
      vendor: adapter.info.vendor,
      architecture: adapter.info.architecture,
      device: adapter.info.device,
      description: adapter.info.description,
      subgroupMinSize: adapter.info.subgroupMinSize,
      subgroupMaxSize: adapter.info.subgroupMaxSize,
    },
    features: Array.from(adapter.features).sort(),
    limits: {
      maxTextureDimension2D: adapter.limits.maxTextureDimension2D,
      maxBindGroups: adapter.limits.maxBindGroups,
      maxBufferSize: adapter.limits.maxBufferSize,
      maxStorageBufferBindingSize: adapter.limits.maxStorageBufferBindingSize,
      maxComputeInvocationsPerWorkgroup: adapter.limits.maxComputeInvocationsPerWorkgroup,
    },
  }));
  return adapter;
};
const requestDevice = GPUAdapter.prototype.requestDevice;
GPUAdapter.prototype.requestDevice = async function (...args) {
  const device = await requestDevice.apply(this, args);
  globalThis.__engineDevice = device;
  Deno.core.ops.op_device_ready();
  device.lost.then((info) => {
    Deno.core.ops.op_device_lost(String(info.reason || "unknown"), String(info.message || ""));
  });
  return device;
};
"#;

struct HtmlModuleLoader {
    exact: HashMap<String, String>,
    prefixes: Vec<(String, String)>,
    html_url: url::Url,
}

impl ModuleLoader for HtmlModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> ModuleResolveResponse {
        if let Some(target) = self.exact.get(specifier) {
            return self
                .html_url
                .join(target)
                .map_err(|error| JsErrorBox::generic(error.to_string()));
        }
        for (prefix, target) in &self.prefixes {
            if let Some(rest) = specifier.strip_prefix(prefix) {
                let base = self
                    .html_url
                    .join(target)
                    .map_err(|error| JsErrorBox::generic(error.to_string()))?;
                return base
                    .join(rest)
                    .map_err(|error| JsErrorBox::generic(error.to_string()));
            }
        }
        let base = url::Url::parse(referrer).unwrap_or_else(|_| self.html_url.clone());
        base.join(specifier)
            .map_err(|error| JsErrorBox::generic(format!("resolve {specifier}: {error}")))
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _referrer: Option<&ModuleLoadReferrer>,
        _options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let result = (|| {
            let path = module_specifier.to_file_path().map_err(|_| {
                JsErrorBox::generic(format!("not a file module: {module_specifier}"))
            })?;
            let code = std::fs::read(path).map_err(JsErrorBox::from_err)?;
            Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
                module_specifier,
                None,
            ))
        })();
        ModuleLoadResponse::Sync(result)
    }
}

fn extract_html_section<'a>(html: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start = html.find(start)? + start.len();
    let end = html[start..].find(end)? + start;
    Some(&html[start..end])
}

/// Extract the entry module from an HTML page: either an inline
/// `<script type="module">CODE</script>` (official three.js style) or an
/// external `<script type="module" src="URL"></script>` (the afterglow demos).
/// For the external form, emits `import "URL";` so the module loader resolves
/// the src against the page's directory.
fn extract_module_script(html: &str) -> Option<String> {
    // Inline module script.
    for (start, end) in [
        (r#"<script type="module">"#, "</script>"),
        (r"<script type='module'>", "</script>"),
    ] {
        if let Some(code) = extract_html_section(html, start, end) {
            if !code.trim().is_empty() {
                return Some(code.trim().to_string());
            }
        }
    }
    // External `src` module script.
    for tag in [
        r#"<script type="module" src="#,
        r"<script type='module' src=",
    ] {
        if let Some(idx) = html.find(tag) {
            let after = &html[idx + tag.len()..];
            let quote = after.chars().next()?;
            if quote != '"' && quote != '\'' {
                continue;
            }
            let src = after.strip_prefix(quote)?.split(quote).next()?;
            if !src.is_empty() {
                return Some(format!(
                    "import {};",
                    serde_json::to_string(src).unwrap_or_else(|_| format!("'{src}'"))
                ));
            }
        }
    }
    None
}

fn parse_official_example(html_path: &std::path::Path) -> (String, String, HtmlModuleLoader) {
    let html = std::fs::read_to_string(html_path)
        .unwrap_or_else(|error| panic!("read official example {html_path:?}: {error}"));
    let module = extract_module_script(&html).expect("official example has a module script");
    let import_map = extract_html_section(&html, "<script type=\"importmap\">", "</script>")
        .or_else(|| extract_html_section(&html, "<script type='importmap'>", "</script>"))
        .unwrap_or("{}");
    let imports: serde_json::Value =
        serde_json::from_str(import_map.trim()).expect("parse official example import map");
    let mut exact = HashMap::new();
    let mut prefixes = Vec::new();
    if let Some(entries) = imports
        .get("imports")
        .and_then(serde_json::Value::as_object)
    {
        for (specifier, target) in entries {
            let Some(target) = target.as_str() else {
                continue;
            };
            if specifier.ends_with('/') {
                prefixes.push((specifier.clone(), target.to_string()));
            } else {
                exact.insert(specifier.clone(), target.to_string());
            }
        }
    }
    let html_url = url::Url::from_file_path(
        html_path
            .canonicalize()
            .expect("canonicalize official example path"),
    )
    .expect("convert official example path to URL");
    (
        html,
        module,
        HtmlModuleLoader {
            exact,
            prefixes,
            html_url,
        },
    )
}

struct App {
    window: Option<Arc<Window>>,
    runtime: Option<JsRuntime>,
    tokio: Option<tokio::runtime::Runtime>,
    game_evaluation: Option<GameEvaluation>,
    runtime_wake: Arc<RuntimeWake>,
    runtime_waker: Waker,
    startup_timeout: Duration,
    startup_remaining: Duration,
    startup_last_active: Option<Instant>,
    startup_reported: bool,
    official_example: bool,
    fatal_error: Option<String>,
    ready: Arc<AtomicBool>,
    lifecycle: Arc<RuntimeLifecycle>,
    device_loss: Arc<std::sync::Mutex<Option<String>>>,
    clock: Arc<MonotonicClock>,
    cursor: [f64; 2],
    modifiers: winit::keyboard::ModifiersState,
    scale_factor: f64,
    pending_pointer_move: Option<serde_json::Value>,
    pending_relative_move: [f64; 2],
    pointer_locked: Arc<AtomicBool>,
    pending_resize: Option<PendingResize>,
    host_trace: bool,
    host_trace_last: Instant,
    host_redraws: u64,
    host_wakes: u64,
    host_pointer_moves: u64,
    host_resizes: u64,
    host_resize_applies: u64,
    frame_interval: Duration,
    next_frame_deadline: Instant,
    game_module: std::path::PathBuf,
    builder: afterglow_shell::builder::ShellBuilder,
}

impl App {
    const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

    fn new(proxy: EventLoopProxy<HostEvent>) -> Self {
        let runtime_wake = Arc::new(RuntimeWake {
            proxy,
            queued: AtomicBool::new(false),
        });
        let runtime_waker = Waker::from(runtime_wake.clone());
        let builder = afterglow_shell::builder::ShellBuilder::new()
            .with_workers(afterglow_shell::builder::ShellBuilder::reference_composition());
        let startup_timeout = std::env::var("AFTERGLOW_STARTUP_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|millis| *millis > 0)
            .map(Duration::from_millis)
            .unwrap_or(Self::STARTUP_TIMEOUT);
        let game_module = std::env::args_os()
            .nth(1)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("native_game.ts")
            });
        Self {
            window: None,
            runtime: None,
            tokio: None,
            game_evaluation: None,
            runtime_wake,
            runtime_waker,
            startup_timeout,
            startup_remaining: startup_timeout,
            startup_last_active: None,
            startup_reported: false,
            official_example: false,
            fatal_error: None,
            ready: Arc::new(AtomicBool::new(false)),
            lifecycle: Arc::new(RuntimeLifecycle::new()),
            device_loss: Arc::new(std::sync::Mutex::new(None)),
            clock: Arc::new(MonotonicClock::new()),
            cursor: [0.0, 0.0],
            modifiers: winit::keyboard::ModifiersState::empty(),
            scale_factor: 1.0,
            pending_pointer_move: None,
            pending_relative_move: [0.0, 0.0],
            pointer_locked: Arc::new(AtomicBool::new(false)),
            pending_resize: None,
            host_trace: std::env::var_os("AFTERGLOW_HOST_TRACE").is_some(),
            host_trace_last: Instant::now(),
            host_redraws: 0,
            host_wakes: 0,
            host_pointer_moves: 0,
            host_resizes: 0,
            host_resize_applies: 0,
            frame_interval: Duration::from_nanos(16_666_667),
            next_frame_deadline: Instant::now(),
            game_module,
            builder,
        }
    }

    fn poll_evaluation(&mut self, cx: &mut Context<'_>) -> Result<bool, String> {
        let result = match self.game_evaluation.as_mut() {
            Some(evaluation) => evaluation.as_mut().poll(cx),
            None => return Ok(true),
        };
        match result {
            Poll::Ready(Ok(())) => {
                self.game_evaluation = None;
                Ok(true)
            }
            Poll::Ready(Err(error)) => {
                self.game_evaluation = None;
                Err(error)
            }
            Poll::Pending => Ok(false),
        }
    }

    /// Advance one bounded deno_core turn. The shell is a persistent browser
    /// host, so it must never run the JavaScript event loop to completion.
    fn poll_runtime_turn_inner(&mut self) -> Result<bool, String> {
        if let Some(runtime) = self.runtime.as_ref() {
            let op_state = runtime.op_state();
            let state = op_state.borrow();
            afterglow_shell::rpc_bridge::poll_native_assets(&state);
            afterglow_shell::rpc_bridge::poll_async_workers(&state);
        }
        let waker = self.runtime_waker.clone();
        let mut cx = Context::from_waker(&waker);
        let mut evaluation_complete = self.poll_evaluation(&mut cx)?;
        let poll = self
            .runtime
            .as_mut()
            .ok_or("JavaScript runtime is absent")?
            .poll_event_loop(
                &mut cx,
                PollEventLoopOptions {
                    wait_for_inspector: false,
                },
            );
        if let Poll::Ready(Err(error)) = poll {
            return Err(error.to_string());
        }
        if !evaluation_complete {
            evaluation_complete = self.poll_evaluation(&mut cx)?;
        }
        Ok(evaluation_complete)
    }

    fn poll_runtime_turn(&mut self) -> Result<bool, String> {
        let tokio = self
            .tokio
            .take()
            .ok_or("JavaScript async runtime is absent")?;
        // `poll_event_loop` is deliberately one bounded turn, but it must be
        // polled inside Tokio so lazy deno async ops (including op_defer) can
        // enter and advance the runtime's current-thread driver.
        let result = tokio.block_on(async {
            // Give lazy async ops dispatched by the preceding deno turn one
            // bounded executor opportunity before collecting their results.
            tokio::task::yield_now().await;
            poll_fn(|_| Poll::Ready(self.poll_runtime_turn_inner())).await
        });
        self.tokio = Some(tokio);
        result
    }

    fn execute_sync(&mut self, name: &'static str, source: String) -> Result<(), String> {
        let tokio_handle = self
            .tokio
            .as_ref()
            .ok_or("JavaScript async runtime is absent")?
            .handle()
            .clone();
        let _tokio_guard = tokio_handle.enter();
        self.runtime
            .as_mut()
            .ok_or("JavaScript runtime is absent")?
            .execute_script(name, source)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn execute_static(&mut self, name: &'static str, source: &'static str) -> Result<bool, String> {
        let tokio_handle = self
            .tokio
            .as_ref()
            .ok_or("JavaScript async runtime is absent")?
            .handle()
            .clone();
        {
            let _tokio_guard = tokio_handle.enter();
            self.runtime
                .as_mut()
                .ok_or("JavaScript runtime is absent")?
                .execute_script(name, source)
                .map_err(|error| error.to_string())?;
        }
        self.poll_runtime_turn()
    }

    fn has_pending_native_workers(&self) -> bool {
        let Some(runtime) = self.runtime.as_ref() else {
            return false;
        };
        let op_state = runtime.op_state();
        let state = op_state.borrow();
        afterglow_shell::rpc_bridge::async_workers_pending(&state)
    }

    fn has_pending_native_assets(&self) -> bool {
        let Some(runtime) = self.runtime.as_ref() else {
            return false;
        };
        let op_state = runtime.op_state();
        let state = op_state.borrow();
        afterglow_shell::rpc_bridge::native_assets_pending(&state)
    }

    fn has_pending_animation_frames(&self) -> bool {
        let Some(runtime) = self.runtime.as_ref() else {
            return false;
        };
        let op_state = runtime.op_state();
        let state = op_state.borrow();
        state.borrow::<NativeAnimationFrames>().pending > 0
    }

    fn animation_frame_diagnostics(&self) -> String {
        let Some(runtime) = self.runtime.as_ref() else {
            return "rAF unavailable".to_string();
        };
        let op_state = runtime.op_state();
        let state = op_state.borrow();
        let frames = state.borrow::<NativeAnimationFrames>();
        format!(
            "rAF pending={}, high_water={}, overflows={}, referenced={}, batches={}, callbacks={}",
            frames.pending,
            frames.high_water,
            frames.overflows,
            frames.referenced,
            frames.batches,
            frames.callbacks
        )
    }

    fn account_startup_time(&mut self) -> Result<(), String> {
        let now = Instant::now();
        if let Some(previous) = self.startup_last_active.replace(now) {
            self.startup_remaining = self
                .startup_remaining
                .saturating_sub(now.saturating_duration_since(previous));
        }
        if self.startup_remaining.is_zero() {
            return Err(format!(
                "native WebGPU startup exceeded {} active milliseconds ({})",
                self.startup_timeout.as_millis(),
                self.animation_frame_diagnostics()
            ));
        }
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.ready.store(false, Ordering::Release);
        self.lifecycle.transition(RuntimePhase::Stopped).ok();
        self.fatal_error = Some(error);
        event_loop.exit();
    }

    fn flush_pointer_move(&mut self) {
        if self.pointer_locked.load(Ordering::Acquire) {
            self.pending_pointer_move = None;
            let [movement_x, movement_y] = std::mem::take(&mut self.pending_relative_move);
            if movement_x != 0.0 || movement_y != 0.0 {
                self.dispatch_input(
                    "pointermove",
                    serde_json::json!({
                        "pointerId": 1,
                        "pointerType": "mouse",
                        "clientX": self.cursor[0] / self.scale_factor,
                        "clientY": self.cursor[1] / self.scale_factor,
                        "movementX": movement_x,
                        "movementY": movement_y,
                        "shiftKey": self.modifiers.shift_key(),
                        "ctrlKey": self.modifiers.control_key(),
                        "altKey": self.modifiers.alt_key(),
                        "metaKey": self.modifiers.super_key(),
                    }),
                );
            }
        } else if let Some(data) = self.pending_pointer_move.take() {
            self.dispatch_input("pointermove", data);
            self.update_cursor_icon();
        }
    }

    fn update_cursor_icon(&mut self) {
        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };
        let cursor = {
            let op_state = runtime.op_state();
            let state = op_state.borrow();
            state
                .borrow::<afterglow_shell::browser::BrowserDocument>()
                .cursor_at(
                    self.cursor[0] / self.scale_factor,
                    self.cursor[1] / self.scale_factor,
                )
        };
        let Ok(cursor) = cursor else {
            return;
        };
        let keyword = cursor.rsplit(',').next().unwrap_or(&cursor).trim();
        let icon = match keyword {
            "context-menu" => CursorIcon::ContextMenu,
            "help" => CursorIcon::Help,
            "pointer" => CursorIcon::Pointer,
            "progress" => CursorIcon::Progress,
            "wait" => CursorIcon::Wait,
            "cell" => CursorIcon::Cell,
            "crosshair" => CursorIcon::Crosshair,
            "text" => CursorIcon::Text,
            "vertical-text" => CursorIcon::VerticalText,
            "alias" => CursorIcon::Alias,
            "copy" => CursorIcon::Copy,
            "move" => CursorIcon::Move,
            "no-drop" => CursorIcon::NoDrop,
            "not-allowed" => CursorIcon::NotAllowed,
            "grab" => CursorIcon::Grab,
            "grabbing" => CursorIcon::Grabbing,
            "e-resize" => CursorIcon::EResize,
            "n-resize" => CursorIcon::NResize,
            "ne-resize" => CursorIcon::NeResize,
            "nw-resize" => CursorIcon::NwResize,
            "s-resize" => CursorIcon::SResize,
            "se-resize" => CursorIcon::SeResize,
            "sw-resize" => CursorIcon::SwResize,
            "w-resize" => CursorIcon::WResize,
            "ew-resize" => CursorIcon::EwResize,
            "ns-resize" => CursorIcon::NsResize,
            "nesw-resize" => CursorIcon::NeswResize,
            "nwse-resize" => CursorIcon::NwseResize,
            "col-resize" => CursorIcon::ColResize,
            "row-resize" => CursorIcon::RowResize,
            "all-scroll" => CursorIcon::AllScroll,
            "zoom-in" => CursorIcon::ZoomIn,
            "zoom-out" => CursorIcon::ZoomOut,
            _ => CursorIcon::Default,
        };
        if let Some(window) = &self.window {
            window.set_cursor(icon);
        }
    }

    fn render(&mut self) -> Result<(), String> {
        if let Some(reason) = self.device_loss.lock().unwrap().take() {
            self.ready.store(false, Ordering::Release);
            return Err(format!("WebGPU device lost; rendering stopped: {reason}"));
        }

        let starting = self.game_evaluation.is_some() || !self.ready.load(Ordering::Acquire);
        if starting {
            self.account_startup_time()?;
        }

        self.flush_resize();

        // Browsers coalesce high-frequency pointer motion to the presentation
        // cadence. Dispatch only the latest sample before each game frame so a
        // 1000 Hz mouse cannot starve rendering with synchronous V8 entries.
        if self.ready.load(Ordering::Acquire) {
            self.flush_pointer_move();
        }

        let evaluation_complete = if self.official_example && !self.ready.load(Ordering::Acquire) {
            self.execute_static(
                "<native-startup-frame>",
                "__runNativeAnimationFrames(performance.now()); __syncBrowserDocument(false); if (Deno.core.ops.op_try_present_surface()) Deno.core.ops.op_engine_ready();",
            )?
        } else {
            self.execute_static(
                "<native-frame>",
                "__runNativeAnimationFrames(performance.now()); __syncBrowserDocument(false); Deno.core.ops.op_try_present_surface();",
            )?
        };

        let ready =
            self.ready.load(Ordering::Acquire) && self.lifecycle.phase() == RuntimePhase::Running;
        if evaluation_complete && ready && !self.startup_reported {
            self.startup_reported = true;
            eprintln!(
                "afterglow-shell renderer ready after {} active milliseconds ({})",
                self.startup_timeout
                    .saturating_sub(self.startup_remaining)
                    .as_millis(),
                self.animation_frame_diagnostics()
            );
        }
        let _ = (evaluation_complete, ready);
        Ok(())
    }

    fn trace_host(&mut self) {
        if !self.host_trace || self.host_trace_last.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.host_trace_last = Instant::now();
        let (present_attempts, presented, surface_width, surface_height) =
            self.runtime.as_ref().map_or((0, 0, 0, 0), |runtime| {
                let op_state = runtime.op_state();
                let state = op_state.borrow();
                let stats = state.borrow::<NativePresentationStats>();
                let surface = state.borrow::<NativeSurface>().data.borrow();
                (
                    stats.attempts,
                    stats.presented,
                    surface.width,
                    surface.height,
                )
            });
        let window_size = self
            .window
            .as_ref()
            .map_or(winit::dpi::PhysicalSize::new(0, 0), |window| {
                window.inner_size()
            });
        eprintln!(
            "[host-trace] redraws={} wakes={} pointer_moves={} resize_events={} resize_applies={} pending_resize={} window={}x{} surface={}x{} scale={} present_attempts={} presented={} {}",
            self.host_redraws,
            self.host_wakes,
            self.host_pointer_moves,
            self.host_resizes,
            self.host_resize_applies,
            self.pending_resize.is_some(),
            window_size.width,
            window_size.height,
            surface_width,
            surface_height,
            self.scale_factor,
            present_attempts,
            presented,
            self.animation_frame_diagnostics()
        );
    }

    fn dispatch_input(&mut self, event_type: &str, data: serde_json::Value) {
        if !self.ready.load(Ordering::Acquire) {
            return;
        }
        let source = format!(
            "dispatchNativeInput({}, {})",
            serde_json::to_string(event_type).unwrap(),
            serde_json::to_string(&data).unwrap()
        );
        if let Err(error) = self.execute_sync("<native-input>", source) {
            eprintln!("native input dispatch failed: {error}");
        }
    }

    fn queue_resize(&mut self, width: u32, height: u32, scale_factor: f64) {
        self.scale_factor = scale_factor.max(f64::EPSILON);
        if width == 0 || height == 0 || self.lifecycle.phase() == RuntimePhase::Suspended {
            self.pending_resize = None;
            self.apply_resize(width, height, self.scale_factor);
            return;
        }
        self.pending_resize = Some(PendingResize::trailing(
            width,
            height,
            self.scale_factor,
            Instant::now(),
        ));
    }

    fn flush_resize(&mut self) {
        let due = self
            .pending_resize
            .as_ref()
            .is_some_and(|resize| resize.is_due(Instant::now()));
        if !due {
            return;
        }
        let resize = self.pending_resize.take().unwrap();
        self.apply_resize(resize.width, resize.height, resize.scale_factor);
    }

    fn apply_resize(&mut self, width: u32, height: u32, scale_factor: f64) {
        self.scale_factor = scale_factor.max(f64::EPSILON);
        self.refresh_frame_interval();
        if !self.ready.load(Ordering::Acquire) {
            return;
        }
        if width == 0 || height == 0 {
            if self.lifecycle.phase() == RuntimePhase::Running {
                self.lifecycle.transition(RuntimePhase::Suspended).ok();
            }
            return;
        }
        if self.lifecycle.phase() == RuntimePhase::Suspended {
            self.lifecycle
                .transition(RuntimePhase::Running)
                .expect("resume runtime after surface restore");
        }
        self.host_resize_applies = self.host_resize_applies.saturating_add(1);
        if let Err(error) = self.execute_sync(
            "<native-resize>",
            format!("if (typeof resizeEngineGame === 'function') resizeEngineGame({width}, {height}, {scale_factor}); else resizeEngineCanvas({width}, {height}, {scale_factor});"),
        ) {
            eprintln!("native resize failed: {error}");
            return;
        }
        self.dispatch_input(
            "resize",
            serde_json::json!({
                "width": width as f64 / scale_factor,
                "height": height as f64 / scale_factor,
                "devicePixelRatio": scale_factor,
            }),
        );
    }

    /// Re-query the monitor refresh rate. On Wayland the compositor does not
    /// configure the output until after the first surface configure, so the
    /// initial detection at window creation may report the 60 Hz fallback.
    fn refresh_frame_interval(&mut self) {
        let Some(window) = &self.window else { return };
        let current = window
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz());
        // On Wayland, current_monitor() may report the wrong rate or None
        // before surface configuration completes. Fall back to the highest
        // refresh rate among all available monitors as a best-effort match.
        let best_available = window
            .available_monitors()
            .into_iter()
            .filter_map(|monitor| monitor.refresh_rate_millihertz())
            .max();
        let refresh_millihertz = current
            .or(best_available)
            .unwrap_or(60_000)
            .clamp(30_000, 360_000);
        let new_interval = Duration::from_nanos(1_000_000_000_000_u64 / refresh_millihertz as u64);
        if new_interval == self.frame_interval {
            return;
        }
        self.frame_interval = new_interval;
        eprintln!(
            "afterglow-shell frame cadence updated to {:.3} ms ({}.{:03} Hz) [current={:?}, best={:?}]",
            self.frame_interval.as_secs_f64() * 1_000.0,
            refresh_millihertz / 1_000,
            refresh_millihertz % 1_000,
            current,
            best_available,
        );
    }
}

impl ApplicationHandler<HostEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            self.startup_last_active = Some(Instant::now());
            self.next_frame_deadline = Instant::now();
            if self.lifecycle.phase() == RuntimePhase::Suspended {
                self.lifecycle.transition(RuntimePhase::Running).ok();
            }
            window.request_redraw();
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("afterglow-shell")
                        .with_inner_size(winit::dpi::PhysicalSize::new(1280, 720)),
                )
                .expect("create native window"),
        );
        let size = window.inner_size();
        self.scale_factor = window.scale_factor();
        let refresh_millihertz = window
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz())
            .unwrap_or(60_000)
            .clamp(30_000, 360_000);
        self.frame_interval =
            Duration::from_nanos(1_000_000_000_000_u64 / refresh_millihertz as u64);
        self.next_frame_deadline = Instant::now();
        eprintln!(
            "afterglow-shell frame cadence {:.3} ms ({}.{:03} Hz)",
            self.frame_interval.as_secs_f64() * 1_000.0,
            refresh_millihertz / 1_000,
            refresh_millihertz % 1_000
        );

        let instance = Arc::new(wgpu_core::global::Global::new(
            "afterglow-shell",
            wgpu_types::InstanceDescriptor {
                backends: wgpu_types::Backends::all(),
                flags: wgpu_types::InstanceFlags::from_build_config(),
                memory_budget_thresholds: wgpu_types::MemoryBudgetThresholds::default(),
                backend_options: wgpu_types::BackendOptions::default(),
                display: None,
            },
            None,
        ));
        let surface_id = unsafe {
            instance.instance_create_surface(
                Some(window.display_handle().unwrap().as_raw()),
                window.window_handle().unwrap().as_raw(),
                None,
            )
        }
        .expect("create shared WebGPU surface");
        let surface = Rc::new(RefCell::new(SurfaceData {
            width: size.width.max(1),
            height: size.height.max(1),
            id: surface_id,
            instance: instance.clone(),
        }));

        let tokio = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("create JavaScript async runtime");
        let _guard = tokio.enter();
        let official_example = self
            .game_module
            .extension()
            .is_some_and(|ext| ext == "html");
        let (document_html, game_source, location, module_loader): (
            String,
            Option<String>,
            url::Url,
            Rc<dyn ModuleLoader>,
        ) = if official_example {
            let (html, module, loader) = parse_official_example(&self.game_module);
            let location = loader.html_url.clone();
            (html, Some(module), location, Rc::new(loader))
        } else {
            (
                "<!doctype html><html><head></head><body></body></html>".to_string(),
                None,
                url::Url::parse("file:///afterglow-shell/").unwrap(),
                Rc::new(FsModuleLoader),
            )
        };
        let mut runtime = JsRuntime::new(RuntimeOptions {
            startup_snapshot: Some(SNAPSHOT),
            module_loader: Some(module_loader),
            extensions: vec![
                deno_webidl::deno_webidl::init(),
                deno_web::deno_web::init(
                    Arc::new(deno_web::BlobStore::default()),
                    Some(location.clone()),
                    false,
                    deno_web::InMemoryBroadcastChannel::default(),
                ),
                deno_webgpu::deno_webgpu::init(),
                afterglow_shell::native_browser::native_browser_ext::init(),
                engine_ext::init(),
            ],
            ..Default::default()
        });
        {
            let op_state = runtime.op_state();
            let mut state = op_state.borrow_mut();
            state.put::<deno_webgpu::Instance>(instance);
            state.put::<NativeSurface>(NativeSurface {
                data: surface,
                context: None,
                canvas_id: None,
                native_node_id: None,
                hud_presenter: None,
            });
            state.put::<EngineReady>(EngineReady(self.ready.clone()));
            state.put::<DeviceLoss>(DeviceLoss(self.device_loss.clone()));
            state.put::<NativePointerLock>(NativePointerLock {
                window: window.clone(),
                locked: self.pointer_locked.clone(),
            });
            state.put::<NativeAnimationFrames>(NativeAnimationFrames {
                referenced: false,
                pending: 0,
                high_water: 0,
                overflows: 0,
                batches: 0,
                callbacks: 0,
            });
            state.put::<NativePresentationStats>(NativePresentationStats {
                attempts: 0,
                presented: 0,
            });
            state.put::<Arc<RuntimeLifecycle>>(self.lifecycle.clone());
            state.put::<Arc<MonotonicClock>>(self.clock.clone());
            state.put::<afterglow_shell::rpc_bridge::WorkerRegistry>(
                afterglow_shell::rpc_bridge::WorkerRegistry::new(),
            );
            state.put::<afterglow_shell::rpc_bridge::ArenaRegistry>(
                afterglow_shell::rpc_bridge::ArenaRegistry::new(),
            );
            let asset_root_path = self.builder.asset_root.clone().unwrap_or_else(|| {
                self.game_module
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_path_buf()
            });
            let asset_root = afterglow_assets::AssetRoot::new(&asset_root_path)
                .unwrap_or_else(|| panic!("invalid native asset root {asset_root_path:?}"));
            afterglow_shell::rpc_bridge::register_native_assets(&mut state, asset_root);
            // Run the worker-composition hook (ShellBuilder::with_workers) —
            // spawns the engine's native workers (assets/texture/audio/…)
            // into the registry. Defaults to the reference Physics worker.
            if let Some(hook) = self.builder.take_workers() {
                hook(&mut state);
            } else {
                let mut registry =
                    state.borrow_mut::<afterglow_shell::rpc_bridge::WorkerRegistry>();
                afterglow_shell::rpc_bridge::register_physics(&mut registry, 0);
            }
            afterglow_shell::native_browser::install_state(
                &mut state,
                size.width.max(1),
                size.height.max(1),
                self.scale_factor,
                location.clone(),
            );
        }
        runtime
            .execute_script("<web-platform>", WEB_PLATFORM_BOOTSTRAP)
            .expect("install native web platform primitives");
        runtime
            .execute_script("<scheduler>", SCHEDULER_BOOTSTRAP)
            .expect("install native task continuation scheduler");
        runtime
            .execute_script(
                "<native-document>",
                format!(
                    "globalThis.__officialExample = {}; globalThis.__documentHTML = {}; globalThis.__surfaceWidth = {}; globalThis.__surfaceHeight = {}; globalThis.__viewportWidth = {}; globalThis.__viewportHeight = {}; globalThis.__devicePixelRatio = {}; globalThis.__exampleURL = {}; globalThis.location = new URL(globalThis.__exampleURL);",
                    official_example,
                    serde_json::to_string(&document_html).unwrap(),
                    size.width.max(1),
                    size.height.max(1),
                    size.width as f64 / self.scale_factor,
                    size.height as f64 / self.scale_factor,
                    self.scale_factor,
                    serde_json::to_string(location.as_str()).unwrap(),
                ),
            )
            .expect("configure native browser document");
        let dom_specifier = ModuleSpecifier::from_file_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("dom_setup.ts"),
        )
        .expect("resolve production DOM bridge");
        let dom_module = tokio
            .block_on(
                runtime
                    .load_side_es_module_from_code(&dom_specifier, include_str!("../dom_setup.ts")),
            )
            .expect("load production DOM bridge");
        let dom_evaluation = Box::pin(runtime.mod_evaluate(dom_module));
        tokio
            .block_on(runtime.with_event_loop_promise(
                dom_evaluation,
                PollEventLoopOptions {
                    wait_for_inspector: false,
                },
            ))
            .expect("initialize production DOM bridge");
        self.lifecycle
            .transition(RuntimePhase::EnvironmentReady)
            .expect("enter environment-ready state");
        runtime
            .execute_script("<engine-bootstrap>", ENGINE_BOOTSTRAP)
            .expect("start native WebGPU engine");
        runtime
            .execute_script("<animation-frame>", ANIMATION_FRAME_BOOTSTRAP)
            .expect("install native animation-frame scheduler");
        let game_specifier =
            ModuleSpecifier::from_file_path(self.game_module.canonicalize().unwrap_or_else(
                |error| panic!("resolve game module {:?}: {error}", self.game_module),
            ))
            .expect("convert native game module to file URL");
        let game_module = if let Some(source) = game_source {
            tokio
                .block_on(runtime.load_side_es_module_from_code(&game_specifier, source))
                .expect("load official three.js example module verbatim")
        } else {
            tokio
                .block_on(runtime.load_main_es_module(&game_specifier))
                .expect("load native game module")
        };
        let evaluation = runtime.mod_evaluate(game_module);
        self.game_evaluation = Some(Box::pin(async move {
            evaluation.await.map_err(|error| error.to_string())
        }));
        self.official_example = official_example;
        self.startup_remaining = self.startup_timeout;
        self.startup_last_active = Some(Instant::now());
        self.startup_reported = false;
        self.window = Some(window.clone());
        self.runtime = Some(runtime);
        self.tokio = Some(tokio);
        // Return to winit immediately. Module evaluation, including top-level
        // rAF awaits used by Three.js compileAsync, advances from real redraws.
        window.request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.startup_last_active = None;
        if self.lifecycle.phase() == RuntimePhase::Running {
            self.lifecycle.transition(RuntimePhase::Suspended).ok();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: HostEvent) {
        match event {
            HostEvent::RuntimeWake => {
                self.host_wakes = self.host_wakes.saturating_add(1);
                self.runtime_wake.clear();
                if self.runtime.is_none() {
                    return;
                }
                // A referenced rAF token keeps deno's event loop alive and can
                // wake its registered waker. Once module evaluation is done,
                // repolling deno for that wake creates a self-sustaining
                // UserEvent loop that starves winit redraw and resize events.
                // Pending rAF is presentation work, so hand it back to winit;
                // idle-runtime wakes with no pending rAF still poll immediately.
                if self.game_evaluation.is_none() && self.has_pending_animation_frames() {
                    return;
                }
                if (self.game_evaluation.is_some() || !self.ready.load(Ordering::Acquire))
                    && let Err(error) = self.account_startup_time()
                {
                    self.fail(event_loop, error);
                    return;
                }
                if let Err(error) = self.poll_runtime_turn() {
                    self.fail(
                        event_loop,
                        format!("native JavaScript event loop failed: {error}"),
                    );
                    return;
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.trace_host();
        if self.runtime.is_none() || self.lifecycle.phase() == RuntimePhase::Suspended {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }

        let now = Instant::now();
        let resize_deadline = self.pending_resize.as_ref().map(|resize| resize.deadline);
        let resize_due = resize_deadline.is_some_and(|deadline| now >= deadline);

        let needs_redraw = self.pending_resize.is_some()
            || self.game_evaluation.is_some()
            || !self.ready.load(Ordering::Acquire)
            || self.has_pending_animation_frames()
            || self.has_pending_native_assets()
            || self.has_pending_native_workers()
            || self.pending_pointer_move.is_some()
            || self.pending_relative_move != [0.0, 0.0];
        if !needs_redraw {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }

        if !resize_due && now < self.next_frame_deadline {
            let wake_at = resize_deadline.map_or(self.next_frame_deadline, |deadline| {
                deadline.min(self.next_frame_deadline)
            });
            event_loop.set_control_flow(ControlFlow::WaitUntil(wake_at));
            return;
        }

        // `request_redraw` is not itself paced by Wayland/wgpu. Admit one
        // browser rAF batch per monitor interval so redraw requests cannot
        // flood submissions and starve configure/input event processing.
        let cadence_deadline = self.next_frame_deadline + self.frame_interval;
        self.next_frame_deadline = if cadence_deadline > now {
            cadence_deadline
        } else {
            now + self.frame_interval
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame_deadline));
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        match event {
            DeviceEvent::MouseMotion { delta } => {
                if self.pointer_locked.load(Ordering::Acquire) {
                    self.pending_relative_move[0] += delta.0;
                    self.pending_relative_move[1] += delta.1;
                }
            }
            _ => {}
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.lifecycle.transition(RuntimePhase::Stopped).ok();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.host_resizes = self.host_resizes.saturating_add(1);
                if self.host_trace {
                    eprintln!(
                        "[host-trace] resize event {}x{} scale={}",
                        size.width,
                        size.height,
                        self.window
                            .as_ref()
                            .map_or(1.0, |window| window.scale_factor())
                    );
                }
                let scale_factor = self
                    .window
                    .as_ref()
                    .map_or(1.0, |window| window.scale_factor());
                self.queue_resize(size.width, size.height, scale_factor);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(window) = &self.window {
                    let size = window.inner_size();
                    self.queue_resize(size.width, size.height, scale_factor);
                    self.refresh_frame_interval();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.host_pointer_moves = self.host_pointer_moves.saturating_add(1);
                if !self.pointer_locked.load(Ordering::Acquire) {
                    self.cursor = [position.x, position.y];
                    self.pending_pointer_move = Some(serde_json::json!({
                        "pointerId": 1,
                        "pointerType": "mouse",
                        "clientX": position.x / self.scale_factor,
                        "clientY": position.y / self.scale_factor,
                        "shiftKey": self.modifiers.shift_key(),
                        "ctrlKey": self.modifiers.control_key(),
                        "altKey": self.modifiers.alt_key(),
                        "metaKey": self.modifiers.super_key(),
                    }));
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.flush_pointer_move();
                self.dispatch_input("pointerleave", serde_json::json!({ "pointerId": 1 }));
                if let Some(window) = &self.window {
                    window.set_cursor(CursorIcon::Default);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.flush_pointer_move();
                let event_type = if state == ElementState::Pressed {
                    "pointerdown"
                } else {
                    "pointerup"
                };
                let button = match button {
                    MouseButton::Left => 0,
                    MouseButton::Middle => 1,
                    MouseButton::Right => 2,
                    MouseButton::Back => 3,
                    MouseButton::Forward => 4,
                    MouseButton::Other(value) => value as u32,
                };
                self.dispatch_input(
                    event_type,
                    serde_json::json!({
                        "pointerId": 1,
                        "pointerType": "mouse",
                        "button": button,
                        "clientX": self.cursor[0] / self.scale_factor,
                        "clientY": self.cursor[1] / self.scale_factor,
                        "shiftKey": self.modifiers.shift_key(),
                        "ctrlKey": self.modifiers.control_key(),
                        "altKey": self.modifiers.alt_key(),
                        "metaKey": self.modifiers.super_key(),
                    }),
                );
            }
            WindowEvent::Touch(touch) => {
                let event_type = match touch.phase {
                    TouchPhase::Started => "pointerdown",
                    TouchPhase::Moved => "pointermove",
                    TouchPhase::Ended => "pointerup",
                    TouchPhase::Cancelled => "pointercancel",
                };
                let pressure = touch.force.map_or(0.5, |force| match force {
                    winit::event::Force::Normalized(value) => value,
                    winit::event::Force::Calibrated {
                        force,
                        max_possible_force,
                        ..
                    } => {
                        if max_possible_force > 0.0 {
                            force / max_possible_force
                        } else {
                            0.5
                        }
                    }
                });
                self.dispatch_input(
                    event_type,
                    serde_json::json!({
                        "pointerId": touch.id,
                        "pointerType": "touch",
                        "clientX": touch.location.x / self.scale_factor,
                        "clientY": touch.location.y / self.scale_factor,
                        "pressure": pressure,
                    }),
                );
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.flush_pointer_move();
                let (delta_x, delta_y, delta_mode) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x as f64, y as f64, 1),
                    MouseScrollDelta::PixelDelta(position) => (
                        position.x / self.scale_factor,
                        position.y / self.scale_factor,
                        0,
                    ),
                };
                self.dispatch_input(
                    "wheel",
                    serde_json::json!({
                        "deltaX": delta_x,
                        "deltaY": delta_y,
                        "deltaMode": delta_mode,
                        "clientX": self.cursor[0] / self.scale_factor,
                        "clientY": self.cursor[1] / self.scale_factor,
                        "shiftKey": self.modifiers.shift_key(),
                        "ctrlKey": self.modifiers.control_key(),
                        "altKey": self.modifiers.alt_key(),
                        "metaKey": self.modifiers.super_key(),
                    }),
                );
            }
            WindowEvent::KeyboardInput { event, .. } => self.dispatch_input(
                if event.state == ElementState::Pressed {
                    "keydown"
                } else {
                    "keyup"
                },
                serde_json::json!({
                    "code": dom_physical_code(event.physical_key),
                    "key": event.logical_key.to_text().unwrap_or(""),
                    "repeat": event.repeat,
                    "shiftKey": self.modifiers.shift_key(),
                    "ctrlKey": self.modifiers.control_key(),
                    "altKey": self.modifiers.alt_key(),
                    "metaKey": self.modifiers.super_key(),
                }),
            ),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::Focused(focused) => {
                if !focused && self.pointer_locked.load(Ordering::Acquire) {
                    let _ = self.window.as_ref().map(|window| {
                        let _ = window.set_cursor_grab(CursorGrabMode::None);
                        window.set_cursor_visible(true);
                    });
                    self.pointer_locked.store(false, Ordering::Release);
                    let _ = self.execute_sync(
                        "<native-pointer-unlock>",
                        "globalThis.__clearPointerLock?.()".to_string(),
                    );
                }
                self.dispatch_input(
                    if focused { "focus" } else { "blur" },
                    serde_json::json!({}),
                );
            }
            WindowEvent::RedrawRequested => {
                self.host_redraws = self.host_redraws.saturating_add(1);
                if let Err(error) = self.render() {
                    self.fail(event_loop, format!("native frame failed: {error}"));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_physical_codes_match_the_browser_api() {
        use winit::keyboard::KeyCode;

        assert_eq!(dom_physical_code(PhysicalKey::Code(KeyCode::KeyW)), "KeyW");
        assert_eq!(
            dom_physical_code(PhysicalKey::Code(KeyCode::ShiftLeft)),
            "ShiftLeft"
        );
        assert_eq!(
            dom_physical_code(PhysicalKey::Code(KeyCode::Digit3)),
            "Digit3"
        );
    }

    #[test]
    fn resize_debounce_uses_the_latest_event_after_175_ms_of_quiet() {
        let start = Instant::now();
        let first = PendingResize::trailing(800, 600, 1.0, start);
        assert!(!first.is_due(start + Duration::from_millis(174)));
        assert!(first.is_due(start + Duration::from_millis(175)));

        let replacement_time = start + Duration::from_millis(100);
        let replacement = PendingResize::trailing(1280, 720, 1.0, replacement_time);
        assert_eq!(replacement.width, 1280);
        assert_eq!(replacement.height, 720);
        assert!(!replacement.is_due(start + Duration::from_millis(274)));
        assert!(replacement.is_due(start + Duration::from_millis(275)));
    }
}

fn main() {
    env_logger::init();
    let event_loop = EventLoop::<HostEvent>::with_user_event()
        .build()
        .expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy());
    event_loop.run_app(&mut app).expect("run native engine");
    if let Some(error) = app.fatal_error {
        panic!("{error}");
    }
}
