//! Native paint RPC service backed directly by the demo `PaintApp`.
//!
//! `PaintApp` owns `Rc`-backed tile storage and therefore stays on the worker
//! thread.  The RPC worker itself is intentionally just a unit value so the
//! native transport can move it to its OS thread without pretending the paint
//! state is `Send`.

use afterglow_rpc::{RpcError, RpcResult, ServeFuture};
use afterglow_rpc_macros::rpc;
use maipointo::app::{PaintApp, WEB_MAX_GROUPS};
use maipointo::brush::Brush;
use maipointo::compositor::{BLEND_MODE_COUNT, BlendMode};
use serde_json::{Map, Value, json};
use std::{cell::RefCell, path::{Path, PathBuf}, rc::Rc, sync::OnceLock};
use document::Document;
use paged_store::{DiskLimits, PaintStore};

static STORAGE_ROOT: OnceLock<PathBuf> = OnceLock::new();

mod tile_threads;
mod memory;
mod paged_store;
mod document;

pub const TILE_SIZE: i32 = 64;
pub const TILE_BYTES: usize = (TILE_SIZE as usize) * (TILE_SIZE as usize) * 4;
pub const MAX_COMMAND_BYTES: usize = 64 * 1024;
const MIN_DOCUMENT_SIZE: i32 = 64;
const MAX_DOCUMENT_SIZE: i32 = 16_384;
const MAX_CONFIG_SETTINGS: usize = 256;
const MAX_STROKE_CONTINUATIONS: usize = 100_000;
const MEMORY_BLOCK_MIB: f64 = 64.0;
const TILE_STORAGE_BYTES: usize = TILE_BYTES * 2;

#[derive(Clone, Copy)]
struct WorkerState {
    blocked: bool,
    display_scale: u32,
    display_mip: u32,
    stroke_open: bool,
    stroke_rejected: bool,
    last_stroke_time: Option<f64>,
    memory: Option<memory::Budget>,
}

impl WorkerState {
    const fn initial() -> Self {
        Self {
            blocked: true,
            display_scale: 1,
            display_mip: 0,
            stroke_open: false,
            stroke_rejected: false,
            last_stroke_time: None,
            memory: None,
        }
    }
}

// PaintApp is !Send (`Rc<TileBudget>`).  This TLS is only accessed by the
// native worker thread: PaintWorker remains a plain Send unit value.
thread_local! {
    static PAINT_APP: RefCell<Option<Document>> = const { RefCell::new(None) };
    static PENDING_STORE: RefCell<Option<Rc<RefCell<PaintStore>>>> = const { RefCell::new(None) };
    static WORKER_STATE: RefCell<WorkerState> = const { RefCell::new(WorkerState::initial()) };
}

#[rpc(worker = PaintWorker, singleton)]
pub trait Paint {
    /// Execute one browser-worker-shaped JSON command and return its state.
    async fn command(command_json: String) -> RpcResult<String>;
    /// Read one visible composite (`layer == -1`) or layer tile.
    async fn tile(layer: i32, tx: i32, ty: i32, scale: u32) -> RpcResult<Vec<u8>>;
    /// Write one RGBA8 tile to `layer` and return the normal state response.
    async fn write_tile(layer: u32, tx: i32, ty: i32, data: Vec<u8>) -> RpcResult<String>;
}

/// The native transport moves this value, not the `PaintApp`.
#[derive(Default)]
pub struct PaintWorker;

impl PaintWorker {
    /// Set the trusted native storage directory before the worker starts.
    /// RPC messages cannot select a filesystem path.
    pub fn set_storage_root(root: impl AsRef<Path>) -> Result<(), String> {
        std::fs::create_dir_all(root.as_ref()).map_err(|error| error.to_string())?;
        let root = std::fs::canonicalize(root).map_err(|error| error.to_string())?;
        if let Some(current) = STORAGE_ROOT.get() {
            return if current == &root { Ok(()) } else { Err("paint storage root is already set".into()) };
        }
        STORAGE_ROOT.set(root).map_err(|_| "paint storage root is already set".into())
    }
}

impl PaintServer for PaintWorker {
    fn command(&self, command_json: String) -> ServeFuture {
        let response = execute_command(&command_json);
        Box::pin(async move { afterglow_rpc::encode(&response) })
    }

    fn tile(&self, layer: i32, tx: i32, ty: i32, scale: u32) -> ServeFuture {
        let result = render_tile(layer, tx, ty, scale);
        Box::pin(async move { afterglow_rpc::encode(&result?) })
    }

    fn write_tile(&self, layer: u32, tx: i32, ty: i32, data: Vec<u8>) -> ServeFuture {
        let result = write_tile_response(layer, tx, ty, &data);
        Box::pin(async move { afterglow_rpc::encode(&result?) })
    }
}

fn rpc_error(message: impl Into<String>) -> RpcError {
    RpcError::Server(message.into())
}

fn no_document_response(message: impl Into<String>) -> String {
    json!({
        "state": Value::Null,
        "dirty": Value::Null,
        "error": message.into(),
    })
    .to_string()
}

fn current_error_response(message: impl Into<String>) -> String {
    let message = message.into();
    PAINT_APP.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(app) = slot.as_mut() else {
            return no_document_response(message);
        };
        WORKER_STATE.with(|state| complete_response(app, &*state.borrow(), None, Some(message)))
    })
}

fn execute_command(command_json: &str) -> String {
    if command_json.len() > MAX_COMMAND_BYTES {
        return current_error_response("paint command exceeds 64 KiB");
    }

    let root = match serde_json::from_str::<Value>(command_json) {
        Ok(value) => value,
        Err(error) => {
            return current_error_response(format!("invalid paint command JSON: {error}"));
        }
    };
    let Some(command) = root.as_object() else {
        return current_error_response("paint command must be a JSON object");
    };
    let Some(name) = command.get("cmd").and_then(Value::as_str) else {
        return current_error_response("paint command is missing cmd");
    };

    if name == "init" {
        return initialize(command);
    }
    if WORKER_STATE.with(|state| state.borrow().blocked) {
        return no_document_response("paint needs initialization and a recovery decision");
    }

    PAINT_APP.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(app) = slot.as_mut() else {
            return no_document_response("paint document is not initialized");
        };
        WORKER_STATE.with(|state| {
            let mut state = state.borrow_mut();
            match execute_document_command(app, &mut state, name, command) {
                Ok(dirty) => complete_response(app, &state, dirty, None),
                Err(error) => rollback_response(app, &mut state, error),
            }
        })
    })
}

fn initialize(command: &Map<String, Value>) -> String {
    match initialize_document(command) {
        Ok(response) => response,
        Err(error) => current_error_response(error),
    }
}

fn initialize_document(command: &Map<String, Value>) -> Result<String, String> {
    let decision = command.get("recovery").map(|value| value.as_str().ok_or("invalid recovery decision")).transpose()?;
    if decision.is_some_and(|value| !matches!(value, "restore" | "discard")) {
        return Err("recovery decision must be restore or discard".into());
    }
    PAINT_APP.with(|slot| {
        let mut slot = slot.borrow_mut();
        let store = if let Some(document) = slot.as_ref() { document.store.clone() } else {
            PENDING_STORE.with(|pending| -> Result<_, String> {
                let mut pending = pending.borrow_mut();
                if pending.is_none() {
                    let root = STORAGE_ROOT.get().ok_or("native paint storage root is not configured")?;
                    let limits = if cfg!(test) { DiskLimits { scratch_bytes: 512 * 1024 * 1024, free_reserve_bytes: 0 } }
                        else { DiskLimits::default() };
                    *pending = Some(Rc::new(RefCell::new(PaintStore::open(&root.join("paint.sqlite"), limits)?)));
                }
                Ok(pending.as_ref().unwrap().clone())
            })?
        };
        let has_recovery = store.borrow().has_recovery();
        if has_recovery && decision.is_none() {
            WORKER_STATE.with(|state| state.borrow_mut().blocked = true);
            return Ok(json!({"state":null,"dirty":null,"recoveryRequired":true}).to_string());
        }
        if !has_recovery && decision == Some("restore") { return Err("paint recovery document is missing".into()); }
        let restored;
        let dimensions = if decision == Some("restore") {
            restored = serde_json::from_slice::<Value>(store.borrow().metadata()).map_err(|error| error.to_string())?;
            restored.as_object().ok_or("paint recovery metadata is not an object")?
        } else { command };
        let width = required_i32(dimensions, "width")?;
        let height = required_i32(dimensions, "height")?;
        if !(MIN_DOCUMENT_SIZE..=MAX_DOCUMENT_SIZE).contains(&width)
            || !(MIN_DOCUMENT_SIZE..=MAX_DOCUMENT_SIZE).contains(&height)
        {
            return Err(format!(
                "paint dimensions must be between {MIN_DOCUMENT_SIZE} and {MAX_DOCUMENT_SIZE}"
            ));
        }
        let requested = command.get("memoryLimitMiB").map(|value| value.as_f64().ok_or_else(|| "memoryLimitMiB must be a number".to_string())).transpose()?;
        let tile_threads = tile_threads::initialize()?;
        let memory = memory::Budget::new(memory::installed_bytes()?, requested, width as u32, height as u32, tile_threads)?;
        let initial = memory.maximum_tiles.min(4096);
        let maximum = memory.maximum_tiles;
        let growth = MEMORY_BLOCK_MIB as usize * 1024 * 1024 / TILE_STORAGE_BYTES;
        let mut document = if decision == Some("restore") {
            let document = Document::restore(store.clone(), initial, maximum, growth)?;
            store.borrow_mut().rollback()?;
            document
        } else {
            let app = PaintApp::new_with_tile_limits(width, height, initial, maximum)
                .ok_or("paint engine initialization failed")?;
            if has_recovery {
                Document::replacement(app, store.clone(), initial, maximum, growth)?
            } else { Document::new(app, store.clone(), initial, maximum, growth)? }
        };
        if let Some(previous) = slot.as_mut() {
            document.app.brush = previous.app.brush.take();
            reset_brush(&mut document.app);
        }
        let max_dimension = width.max(height) as f64;
        let ratio = max_dimension / 4096.0;
        let display_scale = if ratio <= 1.0 {
            1
        } else if ratio <= 2.0 {
            2
        } else {
            4
        };
        let state = WorkerState {
            blocked: false,
            display_scale,
            display_mip: display_scale.trailing_zeros(),
            stroke_open: false,
            stroke_rejected: false,
            last_stroke_time: None,
            memory: Some(memory),
        };
        *slot = Some(document);
        PENDING_STORE.with(|pending| *pending.borrow_mut() = None);
        WORKER_STATE.with(|worker_state| *worker_state.borrow_mut() = state);
        Ok(complete_response(slot.as_mut().unwrap(), &state, Some([0, 0, width, height]), None))
    })
}

fn execute_document_command(document: &mut Document, state: &mut WorkerState, name: &str,
    command: &Map<String, Value>) -> Result<Option<[i32; 4]>, String> {
    if state.stroke_open && !matches!(name, "beginStroke" | "strokeSample" | "strokeBatch" | "commit") {
        finish_stroke(&mut document.app, state)?;
        document.commit()?;
    }
    if matches!(name, "undo" | "redo") {
        return Ok(document.select(if name == "undo" { -1 } else { 1 })?.then(|| full_dirty(&document.app)));
    }
    let dirty = execute_ready_command(&mut document.app, state, name, command)?;
    document.check_storage()?;
    if matches!(name, "commit" | "clear" | "setBackground" | "clearBackground" | "group")
        || (name == "layer" && command.get("op").and_then(Value::as_str) != Some("setActive")) {
        document.commit()?;
    }
    Ok(dirty)
}

fn rollback_response(document: &mut Document, state: &mut WorkerState, error: String) -> String {
    match document.rollback() {
        Ok(()) => {
            state.stroke_open = false;
            state.stroke_rejected = true;
            state.last_stroke_time = None;
            let dirty = full_dirty(&document.app);
            let response = complete_response(document, state, Some(dirty), Some(format!("{error} The operation was rolled back.")));
            let mut response: Value = serde_json::from_str(&response).expect("generated paint response");
            response["documentRolledBack"] = Value::Bool(true);
            response.to_string()
        }
        Err(rollback) => {
            state.blocked = true;
            complete_response(document, state, None, Some(format!("{error} Paint rollback failed: {rollback}")))
        }
    }
}

fn execute_ready_command(
    app: &mut PaintApp,
    state: &mut WorkerState,
    name: &str,
    command: &Map<String, Value>,
) -> Result<Option<[i32; 4]>, String> {
    // Commands are handled synchronously here. Close an open stroke before a
    // command that cannot join it, rather than leaving PaintApp history and
    // capture state open while mutating layers, groups, or brush settings.
    if state.stroke_open && !matches!(name, "beginStroke" | "strokeSample" | "strokeBatch" | "commit") {
        finish_stroke(app, state)?;
    }

    match name {
        "loadBrush" => {
            let text = required_string(command, "json")?;
            if text.len() > MAX_COMMAND_BYTES {
                return Err("brush data exceeds 64 KiB".into());
            }
            let mut brush = default_brush();
            let loaded = maipointo::capi_json::load(&mut brush, text);
            if loaded {
                brush.new_stroke();
            }
            app.brush = Some(brush);
            if loaded {
                Ok(None)
            } else {
                Err("Brush load failed because the .myb data is incorrect.".into())
            }
        }
        "config" => {
            let settings = command
                .get("settings")
                .and_then(Value::as_array)
                .ok_or_else(|| "config settings must be an array".to_string())?;
            if settings.len() > MAX_CONFIG_SETTINGS {
                return Err("config setting capacity exceeded".into());
            }
            let Some(brush) = app.brush.as_mut() else {
                return Err("paint brush is not initialized".into());
            };
            for setting in settings {
                let pair = setting
                    .as_array()
                    .ok_or_else(|| "each config setting must be [name, value]".to_string())?;
                if pair.len() != 2 {
                    return Err("each config setting must be [name, value]".into());
                }
                let Some(name) = pair[0].as_str() else {
                    return Err("config setting name must be a string".into());
                };
                let value = value_f32(&pair[1], "config setting value")?;
                // Browser behavior intentionally ignores names unknown to the
                // shared brush settings table.
                if let Some(index) = maipointo::settings::setting_index_runtime(name) {
                    brush.set_base_value_at(index, value);
                }
            }
            Ok(None)
        }
        "beginStroke" => {
            if state.stroke_open {
                return Err("a new stroke started before the current stroke ended".into());
            }
            let x = field_f32(command, "x")?;
            let y = field_f32(command, "y")?;
            let xtilt = field_f32(command, "xtilt")?;
            let ytilt = field_f32(command, "ytilt")?;
            let zoom = positive_field_f32(command, "zoom")?;
            let rotation = field_f32(command, "rotation")?;
            let barrel = field_f32(command, "barrel")?;
            validate_point(x, y)?;
            app.begin_stroke(x, y, xtilt, ytilt, zoom, rotation, barrel);
            state.stroke_open = true;
            state.stroke_rejected = false;
            state.last_stroke_time = None;
            Ok(None)
        }
        "strokeBatch" => {
            let samples = command.get("samples").and_then(Value::as_array)
                .ok_or("stroke batch needs a sample array")?;
            if samples.is_empty() || samples.len() > 32 {
                return Err("stroke batch needs 1..32 samples".into());
            }
            let mut dirty = None;
            for sample in samples {
                let sample = sample.as_object().ok_or("stroke sample must be an object")?;
                dirty = merge_rects(dirty, execute_ready_command(app, state, "strokeSample", sample)?);
                if let Some(error) = engine_error_message(app.error_code) {
                    return Err(error);
                }
            }
            Ok(dirty)
        }
        "strokeSample" => {
            // Discard samples from the canceled stroke until its boundary.
            if state.stroke_rejected {
                return Ok(None);
            }
            if !state.stroke_open {
                return Err("a stroke sample has no active stroke".into());
            }
            let x = field_f32(command, "x")?;
            let y = field_f32(command, "y")?;
            let pressure = field_f32(command, "pressure")?;
            let xtilt = field_f32(command, "xtilt")?;
            let ytilt = field_f32(command, "ytilt")?;
            let time = field_f64(command, "time")?;
            let zoom = positive_field_f32(command, "zoom")?;
            let rotation = field_f32(command, "rotation")?;
            let barrel = field_f32(command, "barrel")?;
            validate_point(x, y)?;
            if time.abs() > 1.0e12 {
                return Err("stroke time is outside the supported range".into());
            }
            let dtime = state
                .last_stroke_time
                .map_or(0.016, |last| ((time - last) * 0.001).max(0.0));
            let mut result = app.stroke_to(
                x, y, pressure, xtilt, ytilt, dtime, zoom, rotation, barrel, false,
            );
            let mut dirty = take_dirty(app);
            let mut continuations = 0;
            while result == 0 && app.has_stroke_continuation() {
                continuations += 1;
                if continuations > MAX_STROKE_CONTINUATIONS {
                    return Err("stroke continuation exceeded its bounded work limit".into());
                }
                result = app.continue_stroke_to();
                dirty = merge_rects(dirty, take_dirty(app));
            }
            if result <= 0 {
                return Err("the brush stroke made no progress".into());
            }
            state.last_stroke_time = Some(time);
            Ok(dirty)
        }
        "commit" => {
            if state.stroke_open {
                finish_stroke(app, state)?;
            }
            state.stroke_rejected = false;
            Ok(None)
        }
        "clear" => {
            finish_stroke_if_open(app, state)?;
            reset_brush(app);
            app.clear();
            Ok(Some(full_dirty(app)))
        }
        "setBackground" => {
            let r = field_f32(command, "r")?;
            let g = field_f32(command, "g")?;
            let b = field_f32(command, "b")?;
            app.set_background_color(r, g, b);
            Ok(Some(full_dirty(app)))
        }
        "clearBackground" => {
            app.clear_background();
            Ok(Some(full_dirty(app)))
        }
        "setView" => {
            // Native rendering is keyed by the document display scale. Keep
            // the browser command accepted without introducing another render
            // cache or a second scale policy.
            let _ = positive_field_f32(command, "zoom")?;
            Ok(None)
        }
        "layer" => execute_layer_command(app, command),
        "group" => execute_group_command(app, command),
        "requestState" => Ok(None),
        _ => Err(format!("unknown paint command {name}")),
    }
}

fn execute_layer_command(
    app: &mut PaintApp,
    command: &Map<String, Value>,
) -> Result<Option<[i32; 4]>, String> {
    let operation = required_string(command, "op")?;
    match operation {
        "create" => {
            if app.create_layer() < 0 {
                return Err("layer capacity exceeded".into());
            }
        }
        "delete" => {
            let layer = layer_id(command, app)?;
            if !app.delete_layer(layer) {
                return Err("layer delete failed".into());
            }
        }
        "move" => {
            let layer = layer_id(command, app)?;
            let direction = required_direction(command)?;
            if !app.move_layer(layer, direction) {
                return Err("layer move failed".into());
            }
        }
        "setActive" => {
            let layer = layer_id(command, app)?;
            if app.set_active_layer(layer) == 0 {
                return Err("invalid active layer".into());
            }
        }
        "setVisible" => {
            let layer = layer_id(command, app)?;
            let value = optional_f32(command, "value", 0.0)?;
            app.set_layer_visible(layer, value != 0.0);
        }
        "setOpacity" => {
            let layer = layer_id(command, app)?;
            let value = optional_f32(command, "value", 1.0)?;
            app.set_layer_opacity(layer, value);
        }
        "setMode" => {
            let layer = layer_id(command, app)?;
            let mode = blend_mode(command)?;
            app.set_layer_mode(layer, mode);
        }
        "setGroup" => {
            let layer = layer_id(command, app)?;
            let group = optional_group(command, "value", app)?;
            if !app.set_layer_group(layer, group) {
                return Err("invalid layer group".into());
            }
        }
        _ => return Err(format!("unknown layer operation {operation}")),
    }
    Ok(Some(full_dirty(app)))
}

fn execute_group_command(
    app: &mut PaintApp,
    command: &Map<String, Value>,
) -> Result<Option<[i32; 4]>, String> {
    let operation = required_string(command, "op")?;
    match operation {
        "create" => {
            if app.create_group() < 0 {
                return Err("group capacity exceeded".into());
            }
        }
        "delete" => {
            let group = group_id(command, app)?;
            if !app.delete_group(group) {
                return Err("group delete failed".into());
            }
        }
        "move" => {
            let group = group_id(command, app)?;
            let direction = required_direction(command)?;
            if !app.move_group(group, direction) {
                return Err("group move failed".into());
            }
        }
        "setParent" => {
            let group = group_id(command, app)?;
            let parent = optional_group(command, "value", app)?;
            if !app.set_group_parent(group, parent) {
                return Err("invalid group parent".into());
            }
        }
        "setVisible" => {
            let group = group_id(command, app)?;
            let value = optional_f32(command, "value", 0.0)?;
            app.group_visible[group] = value != 0.0;
        }
        "setOpacity" => {
            let group = group_id(command, app)?;
            let value = optional_f32(command, "value", 1.0)?;
            app.group_opacity[group] = value.clamp(0.0, 1.0);
        }
        "setMode" => {
            let group = group_id(command, app)?;
            app.group_mode[group] = blend_mode(command)?;
        }
        "setPassThrough" => {
            let group = group_id(command, app)?;
            let value = optional_f32(command, "value", 0.0)?;
            app.group_pass_through[group] = value != 0.0;
        }
        "setIsolated" => {
            let group = group_id(command, app)?;
            let value = optional_f32(command, "value", 0.0)?;
            app.group_isolated[group] = value != 0.0;
        }
        _ => return Err(format!("unknown group operation {operation}")),
    }
    Ok(Some(full_dirty(app)))
}

fn finish_stroke_if_open(app: &mut PaintApp, state: &mut WorkerState) -> Result<(), String> {
    if state.stroke_open {
        finish_stroke(app, state)?;
    }
    Ok(())
}

fn finish_stroke(app: &mut PaintApp, state: &mut WorkerState) -> Result<(), String> {
    let mut continuations = 0;
    while app.has_stroke_continuation() {
        continuations += 1;
        if continuations > MAX_STROKE_CONTINUATIONS {
            return Err("stroke continuation exceeded its bounded work limit".into());
        }
        if app.continue_stroke_to() < 0 {
            return Err("the brush stroke continuation failed".into());
        }
    }
    app.history_commit();
    state.stroke_open = false;
    state.last_stroke_time = None;
    Ok(())
}

fn reset_brush(app: &mut PaintApp) {
    if let Some(brush) = app.brush.as_mut() {
        brush.request_reset();
    }
}

fn default_brush() -> Brush {
    let mut brush = Brush::new();
    brush.from_defaults();
    brush.new_stroke();
    brush
}

fn layer_id(command: &Map<String, Value>, app: &PaintApp) -> Result<usize, String> {
    let id = required_i32(command, "layer")?;
    if id < 0 || id as usize >= app.layer_count() {
        return Err("invalid layer id".into());
    }
    Ok(id as usize)
}

fn group_id(command: &Map<String, Value>, app: &PaintApp) -> Result<usize, String> {
    let id = required_i32(command, "group")?;
    if id < 0 || id as usize >= WEB_MAX_GROUPS || !app.group_alive[id as usize] {
        return Err("invalid group id".into());
    }
    Ok(id as usize)
}

fn optional_group(command: &Map<String, Value>, key: &str, app: &PaintApp) -> Result<i32, String> {
    let value = optional_i32(command, key, -1)?;
    if value >= 0 && (value as usize >= WEB_MAX_GROUPS || !app.group_alive[value as usize]) {
        return Err("invalid group id".into());
    }
    if value < -1 {
        return Err("group parent must be -1 or a live group".into());
    }
    Ok(value)
}

fn required_direction(command: &Map<String, Value>) -> Result<i32, String> {
    let value = required_i32(command, "value")?;
    if value == -1 || value == 1 {
        Ok(value)
    } else {
        Err("move direction must be -1 or 1".into())
    }
}

fn blend_mode(command: &Map<String, Value>) -> Result<BlendMode, String> {
    let value = optional_i32(command, "value", 0)?;
    if !(0..BLEND_MODE_COUNT as i32).contains(&value) {
        return Err("invalid blend mode".into());
    }
    Ok(BlendMode::from_int(value))
}

fn render_tile(layer: i32, tx: i32, ty: i32, scale: u32) -> RpcResult<Vec<u8>> {
    if WORKER_STATE.with(|state| state.borrow().blocked) { return Err(rpc_error("paint needs a recovery decision")); }
    PAINT_APP.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(document) = slot.as_mut() else {
            return Err(rpc_error("paint document is not initialized"));
        };
        let app = &mut document.app;
        if !matches!(scale, 1 | 2 | 4) {
            return Err(rpc_error("tile scale must be 1, 2, or 4"));
        }
        if layer < -1 || layer as usize >= app.layer_count() && layer != -1 {
            return Err(rpc_error("invalid tile layer"));
        }
        let (tiles_width, tiles_height) = tile_grid(app.width, app.height, scale);
        if tx < 0 || ty < 0 || tx >= tiles_width || ty >= tiles_height {
            return Err(rpc_error("tile coordinates are outside the display grid"));
        }
        let bytes = if layer == -1 {
            let bytes = if scale == 1 {
                app.render_rgba8_tile(tx, ty)
            } else {
                app.render_rgba8_mip_tile(tx, ty, scale.trailing_zeros() as i32)
            };
            bytes.to_vec()
        } else if scale == 1 {
            app.render_layer_rgba8_tile(layer as usize, tx, ty)
                .ok_or_else(|| rpc_error("invalid tile layer"))?
                .to_vec()
        } else {
            render_layer_mip(app, layer as usize, tx, ty, scale)
        };
        if bytes.len() != TILE_BYTES {
            return Err(rpc_error("paint tile has an invalid size"));
        }
        if let Err(error) = document.check_storage() {
            WORKER_STATE.with(|state| state.borrow_mut().blocked = true);
            return Err(rpc_error(error));
        }
        Ok(bytes)
    })
}

fn render_layer_mip(app: &mut PaintApp, layer: usize, tx: i32, ty: i32, scale: u32) -> Vec<u8> {
    let source_tile_count = (scale * scale) as usize;
    let mut sources = Vec::with_capacity(source_tile_count);
    for sy in 0..scale {
        for sx in 0..scale {
            let source = app
                .render_layer_rgba8_tile(
                    layer,
                    tx * scale as i32 + sx as i32,
                    ty * scale as i32 + sy as i32,
                )
                .expect("layer was validated before rendering");
            sources.push(source.to_vec());
        }
    }

    let mut output = vec![0u8; TILE_BYTES];
    let scale_usize = scale as usize;
    let sample_count = u32::from(scale * scale);
    for y in 0..64usize {
        for x in 0..64usize {
            let mut sum = [0u32; 4];
            for sample_y in 0..scale_usize {
                let source_y = y * scale_usize + sample_y;
                let source_tile_y = source_y / 64;
                let local_y = source_y % 64;
                for sample_x in 0..scale_usize {
                    let source_x = x * scale_usize + sample_x;
                    let source_tile_x = source_x / 64;
                    let local_x = source_x % 64;
                    let source = &sources[source_tile_y * scale_usize + source_tile_x];
                    let offset = (local_y * 64 + local_x) * 4;
                    for channel in 0..4 {
                        sum[channel] += u32::from(source[offset + channel]);
                    }
                }
            }
            let offset = (y * 64 + x) * 4;
            for channel in 0..4 {
                output[offset + channel] = ((sum[channel] + sample_count / 2) / sample_count) as u8;
            }
        }
    }
    output
}

fn write_tile_response(layer: u32, tx: i32, ty: i32, data: &[u8]) -> RpcResult<String> {
    if WORKER_STATE.with(|state| state.borrow().blocked) { return Err(rpc_error("paint needs a recovery decision")); }
    if data.len() != TILE_BYTES {
        return Err(rpc_error("paint tile data must be exactly 16384 bytes"));
    }
    PAINT_APP.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(document) = slot.as_mut() else {
            return Err(rpc_error("paint document is not initialized"));
        };
        let app = &mut document.app;
        let layer = usize::try_from(layer).map_err(|_| rpc_error("invalid tile layer"))?;
        if layer >= app.layer_count() {
            return Err(rpc_error("invalid tile layer"));
        }
        let (tiles_width, tiles_height) = tile_grid(app.width, app.height, 1);
        if tx < 0 || ty < 0 || tx >= tiles_width || ty >= tiles_height {
            return Err(rpc_error("tile coordinates are outside the document"));
        }
        WORKER_STATE.with(|state| {
            let mut state = state.borrow_mut();
            let result = (|| -> Result<_, String> {
                if state.stroke_open {
                    finish_stroke(&mut document.app, &mut state)?;
                    document.commit()?;
                }
                let app = &mut document.app;
                if app.set_active_layer(layer) == 0 || !app.write_rgba8_tile(tx, ty, data) {
                    return Err("paint tile write failed".into());
                }
                let dirty = [tx * TILE_SIZE, ty * TILE_SIZE,
                    (app.width - tx * TILE_SIZE).min(TILE_SIZE),
                    (app.height - ty * TILE_SIZE).min(TILE_SIZE)];
                document.commit()?;
                Ok(dirty)
            })();
            Ok(match result {
                Ok(dirty) => complete_response(document, &state, Some(dirty), None),
                Err(error) => rollback_response(document, &mut state, error),
            })
        })
    })
}

fn tile_grid(width: i32, height: i32, scale: u32) -> (i32, i32) {
    let edge = TILE_SIZE * scale as i32;
    (ceil_div(width, edge), ceil_div(height, edge))
}

fn ceil_div(value: i32, divisor: i32) -> i32 {
    (value + divisor - 1) / divisor
}

fn full_dirty(app: &PaintApp) -> [i32; 4] {
    [0, 0, app.width, app.height]
}

fn complete_response(
    document: &mut Document,
    state: &WorkerState,
    forced_dirty: Option<[i32; 4]>,
    explicit_error: Option<String>,
) -> String {
    let app = &mut document.app;
    let mut dirty = take_dirty(app);
    if let Some(forced) = forced_dirty {
        dirty = merge_rects(dirty, Some(forced));
    }
    let engine_error = app.error_code;
    let mut state_value = state_value(app, state, engine_error);
    state_value["canUndo"] = Value::Bool(document.store.borrow().can_undo());
    state_value["canRedo"] = Value::Bool(document.store.borrow().can_redo());
    state_value["scratchPeakFileBytes"] = json!(document.store.borrow().peak_file_bytes());
    if let Ok(memory) = afterglow_memory::process() {
        let stats = memory.stats();
        state_value["sharedMemory"] = json!({ "limitBytes": stats.limit, "reservedBytes": stats.used, "peakBytes": stats.peak });
    }
    let error = explicit_error.or_else(|| engine_error_message(engine_error));
    // The browser reports an engine error once and clears it after reporting.
    app.error_code = 0;

    let mut response = Map::new();
    response.insert("state".into(), state_value);
    response.insert(
        "dirty".into(),
        dirty.map_or(Value::Null, |rect| json!(rect)),
    );
    if let Some(error) = error {
        response.insert("error".into(), Value::String(error));
    }
    Value::Object(response).to_string()
}

fn state_value(app: &PaintApp, state: &WorkerState, error_code: i32) -> Value {
    let layers = (0..app.layer_count())
        .map(|id| {
            json!({
                "id": id,
                "active": id == app.active_layer(),
                "visible": i32::from(app.layer_visible[id]),
                "opacity": app.layer_opacity[id],
                "mode": app.layer_mode[id].as_int(),
                "group": app.layer_parent[id],
            })
        })
        .collect::<Vec<_>>();
    let groups = (0..app.group_count())
        .map(|id| {
            json!({
                "id": id,
                "alive": app.group_alive[id],
                "parent": app.group_parent[id],
                "visible": i32::from(app.group_visible[id]),
                "opacity": app.group_opacity[id],
                "mode": app.group_mode[id].as_int(),
                "passThrough": i32::from(app.group_pass_through[id]),
                "isolated": i32::from(app.group_isolated[id]),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "layers": layers,
        "groups": groups,
        "activeLayer": app.active_layer(),
        "canUndo": app.history_can_undo(),
        "canRedo": app.history_can_redo(),
        "width": app.width,
        "height": app.height,
        "displayScale": state.display_scale,
        "mipLevel": state.display_mip,
        "tilesWidth": ceil_div(app.width, TILE_SIZE),
        "tilesHeight": ceil_div(app.height, TILE_SIZE),
        "residentTiles": app.resident_tile_count(),
        "residentTileLimit": app.resident_tile_limit(),
        "maximumResidentTiles": app.maximum_resident_tile_limit(),
        "nativeMemory": state.memory.map(|budget| json!({
            "maximumMiB": budget.maximum_bytes / (1024 * 1024),
            "limitMiB": budget.limit_bytes / (1024 * 1024),
            "reservedBytes": budget.reserved_bytes,
        })),
        "error": error_code,
    })
}

fn engine_error_message(code: i32) -> Option<String> {
    match code {
        0 => None,
        1 => Some("Paint tile allocation failed.".into()),
        2 => Some("Paint history capture allocation failed.".into()),
        3 => Some("The libmypaint dab loop made no progress.".into()),
        other => Some(format!("Unknown paint error code {other}.")),
    }
}

fn take_dirty(app: &mut PaintApp) -> Option<[i32; 4]> {
    let rects = std::mem::take(&mut app.dirty_roi);
    let mut dirty = None;
    for rect in rects {
        if rect.width <= 0 || rect.height <= 0 {
            continue;
        }
        let x0 = i64::from(rect.x).max(0).min(i64::from(app.width));
        let y0 = i64::from(rect.y).max(0).min(i64::from(app.height));
        let x1 = (i64::from(rect.x) + i64::from(rect.width))
            .max(0)
            .min(i64::from(app.width));
        let y1 = (i64::from(rect.y) + i64::from(rect.height))
            .max(0)
            .min(i64::from(app.height));
        if x1 > x0 && y1 > y0 {
            dirty = merge_rects(
                dirty,
                Some([x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32]),
            );
        }
    }
    dirty
}

fn merge_rects(a: Option<[i32; 4]>, b: Option<[i32; 4]>) -> Option<[i32; 4]> {
    match (a, b) {
        (None, None) => None,
        (Some(rect), None) | (None, Some(rect)) => Some(rect),
        (Some(a), Some(b)) => {
            let x0 = a[0].min(b[0]);
            let y0 = a[1].min(b[1]);
            let x1 = (a[0] + a[2]).max(b[0] + b[2]);
            let y1 = (a[1] + a[3]).max(b[1] + b[3]);
            Some([x0, y0, x1 - x0, y1 - y0])
        }
    }
}

fn required_string<'a>(command: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    command
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string"))
}

fn required_i32(command: &Map<String, Value>, key: &str) -> Result<i32, String> {
    command
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| format!("{key} must be an integer"))
}

fn optional_i32(command: &Map<String, Value>, key: &str, default: i32) -> Result<i32, String> {
    let Some(value) = command.get(key) else {
        return Ok(default);
    };
    value
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| format!("{key} must be an integer"))
}

fn required_f64(command: &Map<String, Value>, key: &str) -> Result<f64, String> {
    let value = command
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{key} must be a number"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{key} must be finite"))
    }
}

fn value_f32(value: &Value, name: &str) -> Result<f32, String> {
    let value = value
        .as_f64()
        .ok_or_else(|| format!("{name} must be a number"))?;
    if !value.is_finite() {
        return Err(format!("{name} must be finite"));
    }
    let value = value as f32;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{name} is outside the f32 range"))
    }
}

fn optional_f32(command: &Map<String, Value>, key: &str, default: f32) -> Result<f32, String> {
    command
        .get(key)
        .map_or(Ok(default), |value| value_f32(value, key))
}

fn field_f32(command: &Map<String, Value>, key: &str) -> Result<f32, String> {
    value_f32(
        command
            .get(key)
            .ok_or_else(|| format!("{key} is required"))?,
        key,
    )
}

fn field_f64(command: &Map<String, Value>, key: &str) -> Result<f64, String> {
    required_f64(command, key)
}

fn positive_field_f32(command: &Map<String, Value>, key: &str) -> Result<f32, String> {
    let value = field_f32(command, key)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(format!("{key} must be positive"))
    }
}

fn validate_point(x: f32, y: f32) -> Result<(), String> {
    if x.abs() <= 1.0e10 && y.abs() <= 1.0e10 {
        Ok(())
    } else {
        Err("stroke coordinates are outside the supported range".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::Mutex;
    use std::task::{Context, Poll, Waker};

    pub(super) static TEST_LOCK: Mutex<()> = Mutex::new(());

    struct RpcDirectory { _lock: std::sync::MutexGuard<'static, ()> }
    impl Drop for RpcDirectory {
        fn drop(&mut self) {
            PAINT_APP.with(|slot| *slot.borrow_mut() = None);
            PENDING_STORE.with(|slot| *slot.borrow_mut() = None);
            WORKER_STATE.with(|state| *state.borrow_mut() = WorkerState::initial());
            std::fs::remove_dir_all(STORAGE_ROOT.get().unwrap()).unwrap();
        }
    }
    fn rpc_directory() -> RpcDirectory {
        let lock = TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("afterglow-paint-rpc-{}", std::process::id()));
        PaintWorker::set_storage_root(root).unwrap();
        RpcDirectory { _lock: lock }
    }

    fn drive<F: Future>(client: &PaintClient, future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        loop {
            client.poll();
            if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn command(client: &PaintClient, json: &str) -> Value {
        let mut value: Value = serde_json::from_str(json).unwrap();
        if value["cmd"] == "init" { value["recovery"] = json!("discard"); }
        let future = client.command(value.to_string()).expect("command encode");
        serde_json::from_str(&drive(client, future).expect("command RPC"))
            .expect("command response JSON")
    }

    #[test]
    fn native_tile_threads_match_serial_pixels_and_smudge_history() {
        let _directory = rpc_directory();
        let draw = |parallel: bool| {
            let call = |value: Value| {
                let result: Value = serde_json::from_str(&execute_command(&value.to_string())).unwrap();
                assert!(result.get("error").is_none(), "{result}");
                result
            };
            call(json!({"cmd":"init","width":512,"height":512,"memoryLimitMiB":512,"recovery":"discard"}));
            PAINT_APP.with(|slot| {
                let mut slot = slot.borrow_mut();
                let app = &mut slot.as_mut().unwrap().app;
                if !parallel { app.set_tile_job_dispatcher(None); }
                for y in 0..8 { for x in 0..8 {
                    let mut tile = app.active().get_or_create_tile_mut(x, y).unwrap();
                    for pixel in tile.chunks_exact_mut(4) {
                        pixel.copy_from_slice(&[if x < 4 { 32768 } else { 0 }, 0, 16384, 32768]);
                    }
                }}
            });
            call(json!({"cmd":"commit"}));
            call(json!({"cmd":"config","settings":[["radius_logarithmic",4.0],["smudge",0.8],["smudge_length",0.5]]}));
            call(json!({"cmd":"beginStroke","x":64,"y":256,"xtilt":0,"ytilt":0,"zoom":1,"rotation":0,"barrel":0}));
            for index in 0..12 {
                call(json!({"cmd":"strokeSample","x":64+index*32,"y":240+index*2,"pressure":0.7,"xtilt":0,"ytilt":0,"time":index*16+1,"zoom":1,"rotation":0,"barrel":0}));
            }
            let result = call(json!({"cmd":"commit"}));
            assert_eq!(result["state"]["canUndo"], true);
            let pixels = || PAINT_APP.with(|slot| {
                let slot = slot.borrow();
                let app = &slot.as_ref().unwrap().app;
                let mut data = Vec::new();
                for y in 0..8 { for x in 0..8 {
                    data.extend_from_slice(&app.layers[0].get_tile(x, y).unwrap()[..]);
                }}
                data
            });
            let after = pixels();
            call(json!({"cmd":"undo"}));
            assert_ne!(pixels(), after);
            call(json!({"cmd":"redo"}));
            assert_eq!(pixels(), after);
            PAINT_APP.with(|slot| *slot.borrow_mut() = None);
            after
        };
        assert_eq!(draw(false), draw(true));
    }

    #[test]
    fn native_client_draws_and_round_trips_history() {
        let _directory = rpc_directory();
        let client = PaintClient::spawn_worker().expect("paint worker");
        let init = command(
            &client,
            r#"{"cmd":"init","width":128,"height":128,"memoryLimitMiB":512}"#,
        );
        assert_eq!(init["dirty"], json!([0, 0, 128, 128]));
        assert_eq!(init["state"]["displayScale"], 1);
        let before = drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap();
        assert_eq!(before.len(), TILE_BYTES);

        command(
            &client,
            r#"{"cmd":"beginStroke","x":32,"y":64,"xtilt":0,"ytilt":0,"zoom":1,"rotation":0,"barrel":0}"#,
        );
        command(
            &client,
            r#"{"cmd":"strokeSample","x":32,"y":64,"pressure":0.5,"xtilt":0,"ytilt":0,"time":1,"zoom":1,"rotation":0,"barrel":0}"#,
        );
        command(
            &client,
            r#"{"cmd":"strokeSample","x":96,"y":64,"pressure":0.5,"xtilt":0,"ytilt":0,"time":16,"zoom":1,"rotation":0,"barrel":0}"#,
        );
        let committed = command(&client, r#"{"cmd":"commit"}"#);
        assert!(committed["state"]["canUndo"].as_bool().unwrap());
        let painted = drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap();
        assert_ne!(painted, before);

        let undone = command(&client, r#"{"cmd":"undo"}"#);
        assert!(!undone["state"]["canUndo"].as_bool().unwrap());
        assert_eq!(
            drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap(),
            before
        );
        command(&client, r#"{"cmd":"redo"}"#);
        assert_eq!(
            drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap(),
            painted
        );
    }

    #[test]
    fn stroke_batches_keep_sample_order_and_history() {
        let _directory = rpc_directory();
        let client = PaintClient::spawn_worker().expect("paint worker");
        let samples: Vec<Value> = (0..32).map(|i| json!({
            "cmd": "strokeSample", "x": 16 + i, "y": 32, "pressure": 0.5,
            "xtilt": 0, "ytilt": 0, "time": i * 16, "zoom": 1, "rotation": 0, "barrel": 0
        })).collect();
        let mut reference = Vec::new();
        for batched in [false, true] {
            command(&client, r#"{"cmd":"init","width":64,"height":64,"memoryLimitMiB":512}"#);
            let before = drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap();
            command(&client, r#"{"cmd":"beginStroke","x":16,"y":32,"xtilt":0,"ytilt":0,"zoom":1,"rotation":0,"barrel":0}"#);
            if batched {
                let response = command(&client, &json!({"cmd":"strokeBatch", "samples":samples}).to_string());
                assert!(response.get("error").is_none(), "{response}");
                assert!(!response["dirty"].is_null());
            } else {
                for sample in &samples {
                    let response = command(&client, &sample.to_string());
                    assert!(response.get("error").is_none(), "{response}");
                }
            }
            command(&client, r#"{"cmd":"commit"}"#);
            let painted = drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap();
            assert_ne!(painted, before);
            if batched { assert_eq!(painted, reference); } else { reference = painted.clone(); }
            command(&client, r#"{"cmd":"undo"}"#);
            assert_eq!(drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap(), before);
            command(&client, r#"{"cmd":"redo"}"#);
            assert_eq!(drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap(), painted);
        }
        for count in [0, 33] {
            let response = command(&client, &json!({"cmd":"strokeBatch", "samples":vec![json!({}); count]}).to_string());
            assert!(response["error"].as_str().unwrap().contains("1..32"));
        }
    }

    #[test]
    fn disk_failure_rolls_back_large_strokes_and_keeps_the_worker_available() {
        let _directory = rpc_directory();
        for boundary in ["commit", "strokeBatch"] {
            let init = execute_command(r#"{"cmd":"init","width":4096,"height":2048,"memoryLimitMiB":768,"recovery":"discard"}"#);
            assert!(serde_json::from_str::<Value>(&init).unwrap().get("error").is_none(), "{init}");
            PAINT_APP.with(|slot| {
                let mut slot = slot.borrow_mut();
                let app = &mut slot.as_mut().unwrap().app;
                let mut old = vec![10000; 64 * 64 * 4];
                for pixel in old.chunks_exact_mut(4) { pixel[3] = 32768; }
                assert!(app.write_layer_rgba16_tile_modified(0, 0, 0, &old));
            });
            execute_command(r#"{"cmd":"commit"}"#);
            let before = render_tile(-1, 0, 0, 1).unwrap();
            execute_command(r#"{"cmd":"beginStroke","x":32,"y":32,"xtilt":0,"ytilt":0,"zoom":1,"rotation":0,"barrel":0}"#);
            PAINT_APP.with(|slot| {
                let mut slot = slot.borrow_mut();
                let surface = slot.as_mut().unwrap().app.active();
                surface.begin_atomic();
                // 1,025 different tiles exceed the real 32 MiB capture limit.
                for index in 0..1025 {
                    assert!(surface.draw_dab((index % 64 * 64 + 32) as f32,
                        (index / 64 * 64 + 32) as f32, 4.0, 1.0, 0.0, 0.0,
                        1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0));
                }
                surface.end_atomic();
            });
            let sample = json!({"cmd":"strokeSample", "x":48, "y":32, "pressure":0.5,
                "xtilt":0, "ytilt":0, "time":16, "zoom":1, "rotation":0, "barrel":0});
            PAINT_APP.with(|slot| {
                let mut slot = slot.borrow_mut();
                let document = slot.as_mut().unwrap();
                assert_eq!(document.app.external_history_capture_count(), 0);
                if boundary == "commit" { document.store.borrow_mut().fail_write = true; }
                else { document.app.error_code = 1; }
            });
            let request = if boundary == "commit" { json!({"cmd":"commit"}) }
                else { json!({"cmd":"strokeBatch", "samples":[sample.clone(), sample.clone()]}) };
            let response: Value = serde_json::from_str(&execute_command(&request.to_string())).unwrap();
            assert_eq!(response["documentRolledBack"], true, "{response}");
            assert_eq!(response["dirty"], json!([0, 0, 4096, 2048]));
            assert!(response["error"].as_str().unwrap().contains("rolled back"));
            assert_eq!(render_tile(-1, 0, 0, 1).unwrap(), before);
            PAINT_APP.with(|slot| {
                let mut slot = slot.borrow_mut();
                slot.as_mut().unwrap().store.borrow_mut().fail_write = false;
                let app = &mut slot.as_mut().unwrap().app;
                for index in 1..1025 {
                    if let Some(tile) = app.active().get_tile(index % 64, index / 64) {
                        assert!(tile.iter().all(|&word| word == 0), "tile {index}");
                    }
                }
            });
            let ignored: Value = serde_json::from_str(&execute_command(&sample.to_string())).unwrap();
            assert!(ignored.get("error").is_none(), "{ignored}");
            assert!(ignored["dirty"].is_null());
            assert_eq!(render_tile(-1, 0, 0, 1).unwrap(), before);
            execute_command(r#"{"cmd":"commit"}"#);
            let begin: Value = serde_json::from_str(&execute_command(r#"{"cmd":"beginStroke","x":32,"y":32,"xtilt":0,"ytilt":0,"zoom":1,"rotation":0,"barrel":0}"#)).unwrap();
            assert!(begin.get("error").is_none(), "{begin}");
            let start: Value = serde_json::from_str(&execute_command(r#"{"cmd":"strokeSample","x":32,"y":32,"pressure":0.5,"xtilt":0,"ytilt":0,"time":1,"zoom":1,"rotation":0,"barrel":0}"#)).unwrap();
            assert!(start.get("error").is_none(), "{start}");
            let next: Value = serde_json::from_str(&execute_command(&sample.to_string())).unwrap();
            assert!(next.get("error").is_none(), "{next}");
            let commit: Value = serde_json::from_str(&execute_command(r#"{"cmd":"commit"}"#)).unwrap();
            assert!(commit.get("error").is_none(), "{commit}");
            assert_eq!(commit["state"]["canUndo"], true);
            assert_ne!(render_tile(-1, 0, 0, 1).unwrap(), before);
            execute_command(r#"{"cmd":"undo"}"#);
            assert_eq!(render_tile(-1, 0, 0, 1).unwrap(), before);
        }
        PAINT_APP.with(|slot| *slot.borrow_mut() = None);
    }

    #[test]
    fn resident_tiles_grow_past_the_initial_block_without_resetting_pixels() {
        let _directory = rpc_directory();
        let init: Value = serde_json::from_str(&execute_command(
            r#"{"cmd":"init","width":8192,"height":4096,"memoryLimitMiB":1024}"#)).unwrap();
        assert!(init.get("error").is_none(), "{init}");
        PAINT_APP.with(|slot| {
            let mut slot = slot.borrow_mut();
            let app = &mut slot.as_mut().unwrap().app;
            assert_eq!(app.resident_tile_limit(), 4096);
            assert_eq!(app.resident_tile_count(), 0);
            for index in 0..4097 {
                let mut tile = app.active().get_or_create_tile_mut(index % 128, index / 128).unwrap();
                tile[0] = (index + 1) as u16;
            }
            assert_eq!(app.resident_tile_count(), 4097);
            assert_eq!(app.resident_tile_limit(), 6144);
            assert!(app.resident_tile_limit() <= app.maximum_resident_tile_limit());
            for index in 0..4097 {
                assert_eq!(app.active().get_tile(index % 128, index / 128).unwrap()[0], (index + 1) as u16);
            }
        });
        PAINT_APP.with(|slot| *slot.borrow_mut() = None);
    }

    #[test]
    fn native_rpc_reopens_only_after_restore_or_discard() {
        let _directory = rpc_directory();
        let client = PaintClient::spawn_worker().unwrap();
        command(&client, r#"{"cmd":"init","width":128,"height":64,"memoryLimitMiB":512}"#);
        let mut pixels = vec![0; TILE_BYTES];
        for pixel in pixels.chunks_exact_mut(4) { pixel.copy_from_slice(&[80, 30, 10, 255]); }
        let written: Value = serde_json::from_str(&drive(&client, client.write_tile(0, 0, 0, pixels).unwrap()).unwrap()).unwrap();
        assert!(written.get("error").is_none(), "{written}");
        let before = drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap();
        drop(client);
        let client = PaintClient::spawn_worker().unwrap();
        let call = |value: Value| -> Value {
            serde_json::from_str(&drive(&client, client.command(value.to_string()).unwrap()).unwrap()).unwrap()
        };
        let pending = call(json!({"cmd":"init","width":64,"height":64,"memoryLimitMiB":512}));
        assert_eq!(pending["recoveryRequired"], true);
        assert!(pending["state"].is_null());
        assert!(call(json!({"cmd":"clear"})).get("error").is_some());
        assert!(drive(&client, client.tile(-1, 0, 0, 1).unwrap()).is_err());
        assert!(drive(&client, client.write_tile(0, 0, 0, vec![0; TILE_BYTES]).unwrap()).is_err());
        let restored = call(json!({"cmd":"init","recovery":"restore","width":64,"height":64,"memoryLimitMiB":512}));
        assert!(restored.get("error").is_none(), "{restored}");
        assert_eq!(restored["state"]["width"], 128);
        assert_eq!(restored["state"]["canUndo"], true);
        assert_eq!(drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap(), before);
        let invalid = call(json!({"cmd":"init","recovery":"discard","width":1,"height":64}));
        assert!(invalid.get("error").is_some());
        assert_eq!(drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap(), before);
        let discarded = call(json!({"cmd":"init","recovery":"discard","width":64,"height":64,"memoryLimitMiB":512}));
        assert!(discarded.get("error").is_none(), "{discarded}");
        assert_eq!(discarded["state"]["width"], 64);
        assert_eq!(discarded["state"]["canUndo"], false);
        assert_eq!(discarded["state"]["canRedo"], false);
        assert_ne!(drive(&client, client.tile(-1, 0, 0, 1).unwrap()).unwrap(), before);
        assert_eq!(call(json!({"cmd":"init","width":128,"height":128}))["recoveryRequired"], true);
    }

    #[test]
    fn invalid_commands_and_tile_data_are_bounded() {
        let _directory = rpc_directory();
        let client = PaintClient::spawn_worker().expect("paint worker");
        command(
            &client,
            r#"{"cmd":"init","width":64,"height":64,"memoryLimitMiB":512}"#,
        );
        let invalid = command(&client, r#"{"cmd":"setView","zoom":0}"#);
        assert!(invalid["error"].as_str().unwrap().contains("positive"));
        let oversized = command(
            &client,
            &format!(
                r#"{{"cmd":"unknown","padding":"{}"}}"#,
                "x".repeat(MAX_COMMAND_BYTES)
            ),
        );
        assert!(oversized["error"].as_str().unwrap().contains("64 KiB"));

        let write = client.write_tile(0, 0, 0, vec![0; TILE_BYTES - 1]).unwrap();
        assert!(drive(&client, write).is_err());
        assert!(drive(&client, client.tile(0, 1, 0, 1).unwrap()).is_err());
    }
}
