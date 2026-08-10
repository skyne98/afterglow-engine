/* Threaded brush engine worker — owns the WASM module, the motion queue,
 * the OffscreenCanvas, and all engine state. The main thread forwards input
 * and configuration; this worker does all brush + display work on its own
 * thread so pthread_join (Atomics.wait) works without blocking the browser UI.
 */
import { MotionQueue } from './paint-input.ts';

type Msg =
  | { cmd: 'init'; width: number; height: number; canvas: OffscreenCanvas }
  | { cmd: 'loadBrush'; json: string }
  | { cmd: 'config'; settings: [string, number][] }
  | { cmd: 'beginStroke'; x: number; y: number; xtilt: number; ytilt: number; zoom: number; rotation: number; barrel: number }
  | { cmd: 'strokeSample'; x: number; y: number; pressure: number; xtilt: number; ytilt: number; time: number; zoom: number; rotation: number; barrel: number }
  | { cmd: 'commit' } | { cmd: 'undo' } | { cmd: 'redo' } | { cmd: 'clear' }
  | { cmd: 'setBackground'; r: number; g: number; b: number } | { cmd: 'clearBackground' }
  | { cmd: 'setView'; zoom: number }
  | { cmd: 'layer'; op: string; layer: number; value?: number }
  | { cmd: 'group'; op: string; group: number; value?: number }
  | { cmd: 'exportTiles'; layerId: number | null; id: number }
  | { cmd: 'writeTile'; layer: number; tx: number; ty: number; data: ArrayBuffer }
  | { cmd: 'probe'; id: number; y: number }
  | { cmd: 'requestState' };

const TILE = 64, TILE_B = TILE * TILE * 4, EOTF = 2.2, BUDGET = 8;
let mod: any = null, motionQueue = new MotionQueue(8192);
let ctx: OffscreenCanvasRenderingContext2D | null = null, canvas: OffscreenCanvas | null = null;
let rgba8: Uint8Array | null = null, rectPtr = 0, renderedMip = -1, lastT = 0;
let docW = 2048, docH = 2048, dispScale = 1, dispMip = 0, viewMip = 0;
let flushT: number | null = null, commitP = false, batching = false;
let n0 = 0, a0 = 0;
let batchInFlight = false;
let brushOk = false, brushJson = '';
let bgRGB: [number, number, number] = [0xA8 / 255, 0xA4 / 255, 0x98 / 255];
let statsS = 0, statsMs = 0, lastBR = 0, lastRR = 0, lastStats = 0;
let lastRects: number[] = [];

const post = (m: any, t?: Transferable[]) => { if (t && t.length > 0) (self as unknown as Worker).postMessage(m, t); else (self as unknown as Worker).postMessage(m); };

function setB(n: string, v: number) { const b = mod.lengthBytesUTF8(n) + 1; const p = mod._malloc(b); mod.stringToUTF8(n, p, b); mod._set_brush_base_value(p, v); mod._free(p); }
function mipLevel() { return dispMip > viewMip ? dispMip : viewMip; }
function fillBg() { if (!ctx || !canvas) return; const [r, g, b] = bgRGB; ctx.fillStyle = `rgb(${Math.round(r * 255)},${Math.round(g * 255)},${Math.round(b * 255)})`; ctx.fillRect(0, 0, canvas.width, canvas.height); }
function drawTile(tx: number, ty: number, scale: number) {
  if (!ctx) return; if (scale === 1) { ctx.putImageData(new ImageData(new Uint8ClampedArray(rgba8!), TILE, TILE), tx * TILE, ty * TILE); return; }
  const sz = TILE * scale; const img = new ImageData(sz, sz); const s = rgba8!;
  for (let y = 0; y < TILE; y++) for (let x = 0; x < TILE; x++) { const so = (y * TILE + x) * 4; for (let dy = 0; dy < scale; dy++) for (let dx = 0; dx < scale; dx++) { const to = ((y * scale + dy) * sz + x * scale + dx) * 4; img.data[to] = s[so]; img.data[to+1] = s[so+1]; img.data[to+2] = s[so+2]; img.data[to+3] = s[so+3]; } }
  ctx.putImageData(img, tx * sz, ty * sz);
}
function renderTile(tx: number, ty: number, level: number) {
  const os = Math.max(1, (1 << level) / dispScale);
  const ptr = level === 0 ? mod._paint_render_rgba8_tile_ptr(tx, ty) : mod._paint_render_rgba8_mip_tile_ptr(tx, ty, level);
  if (!ptr) return; rgba8 = mod.HEAPU8.subarray(ptr, ptr + TILE_B); drawTile(tx, ty, os);
}
function renderFull() {
  if (!mod || !ctx) return; const tw = mod._paint_get_tiles_width(), th = mod._paint_get_tiles_height(), ml = mipLevel(), sc = 1 << ml;
  fillBg(); renderedMip = ml; const mw = Math.ceil(tw / sc), mh = Math.ceil(th / sc);
  const used = mod._paint_get_used_tile_count(), single = mod._paint_get_layer_count() === 1;
  if (used > 0 || !single) for (let ty = 0; ty < mh; ty++) for (let tx = 0; tx < mw; tx++) if (mod._paint_region_has_paint(tx, ty, ml)) renderTile(tx, ty, ml);
  mod._paint_clear_dirty();
}
function renderDirty(forceAll = false) {
  if (!mod || !ctx) return;
  if (forceAll || mipLevel() !== renderedMip) { renderFull(); checkErr(); return; }
  const tw = mod._paint_get_tiles_width(), th = mod._paint_get_tiles_height(), ml = mipLevel(), sc = 1 << ml, cnt = mod._paint_get_dirty_count();
  lastRects = [];
  for (let i = 0; i < cnt; i++) {
    mod._paint_get_dirty_rect(i, rectPtr); const b = rectPtr >> 2;
    const x = mod.HEAP32[b], y = mod.HEAP32[b+1], w = mod.HEAP32[b+2], h = mod.HEAP32[b+3];
    lastRects.push(x, y, w, h);
    const ts = TILE * sc, mw = ml > 0 ? Math.ceil(tw / sc) : tw, mh = ml > 0 ? Math.ceil(th / sc) : th;
    const tx0 = Math.max(0, Math.floor(x / ts)), ty0 = Math.max(0, Math.floor(y / ts));
    const tx1 = Math.min(mw - 1, Math.floor((x + w) / ts)), ty1 = Math.min(mh - 1, Math.floor((y + h) / ts));
    for (let ty = ty0; ty <= ty1; ty++) for (let tx = tx0; tx <= tx1; tx++) renderTile(tx, ty, ml);
  }
  mod._paint_clear_dirty(); checkErr();
}
function checkErr() { const c = mod._paint_get_error_code(); if (c) { post({ type: 'status', text: c === 1 ? 'Paint storage allocation failed.' : 'Undo history capacity reached.' }); mod._paint_clear_error(); } }
function strokeSample(_t: number, x: number, y: number, p: number, xt: number, yt: number, z: number, r: number, ba: number) {
  const dt = lastT > 0 ? Math.max(0, (_t - lastT) * 0.001) : 0.016; lastT = _t;
  mod._stroke_to(x, y, p, xt, yt, dt, z, r, ba, 0);
}
function emitStats() { const now = performance.now(); if (now - lastStats < 150) return; const wall = now - lastStats; lastStats = now; post({ type: 'stats', queued: motionQueue.length, brushMs: lastBR, renderMs: lastRR, sps: wall > 0 ? Math.round((statsS / wall) * 1000) : 0 }); statsS = 0; statsMs = 0; }
function afterBatch() {
  batchInFlight = false;
  renderDirty();
  const a2 = performance.now(); lastBR = 0; lastRR = a2 - a0; statsS += n0; statsMs += a2 - a0; emitStats();
  if (commitP && motionQueue.length === 0) { commitP = false; doCommit(); }
  else if (motionQueue.length > 0) { if (!batching) { mod._paint_begin_batch(); batching = true; } drainProcess(true); }
}

function pollBatch() {
  if (!mod) return;
  if (mod._paint_is_batch_done()) {
    mod._paint_end_batch_finish();
    afterBatch();
  } else {
    self.setTimeout(pollBatch, 2);
  }
}

function drainProcess(bounded: boolean) {
  if (!mod || batchInFlight) return; n0 = motionQueue.length; a0 = performance.now();
  if (bounded) motionQueue.drainInterpolatedBounded(strokeSample, BUDGET); else motionQueue.drainInterpolated(strokeSample);
  const a1 = performance.now();
  if (batching) { mod._paint_end_batch(); batching = false; }
  if (!mod._paint_is_batch_done()) {
    batchInFlight = true;
    pollBatch();
    return;
  }
  afterBatch();
}
function scheduleFlush() {
  if (flushT !== null || batchInFlight) return; flushT = self.setTimeout(() => {
    flushT = null; if (!mod || batchInFlight) return; drainProcess(true);
  }, 8);
}
function flushNow() { if (flushT !== null) { clearTimeout(flushT); flushT = null; } if (!mod || batchInFlight) return; drainProcess(false); }
function doCommit() { if (!mod) return; if (batching) { mod._paint_end_batch(); if (!mod._paint_is_batch_done()) { /* TODO: async commit */ } batching = false; } mod._paint_history_commit(); pushState(); }
function beginStroke(x: number, y: number, xt: number, yt: number, z: number, r: number, ba: number) {
  if (!mod) return; if (motionQueue.length > 0 || commitP || batching || flushT !== null) flushNow();
  motionQueue.clear(); lastT = 0; mod._begin_stroke(x, y, xt, yt, z, r, ba); mod._paint_begin_batch(); batching = true; scheduleFlush();
}
function pushState() {
  if (!mod) return;
  const layers: any[] = [], groups: any[] = [];
  const lc = mod._paint_get_layer_count(), active = mod._paint_get_active_layer();
  for (let i = 0; i < lc; i++) layers.push({ id: i, active: i === active, visible: mod._paint_get_layer_visible(i), opacity: mod._paint_get_layer_opacity?.(i) ?? 1, mode: mod._paint_get_layer_mode(i), group: mod._paint_get_layer_group(i) });
  const gc = mod._paint_get_group_count();
  for (let i = 0; i < gc; i++) groups.push({ id: i, alive: mod._paint_get_group_alive(i) !== 0, parent: mod._paint_get_group_parent(i), visible: mod._paint_get_group_visible(i), opacity: mod._paint_get_group_opacity(i), mode: mod._paint_get_group_mode(i), passThrough: mod._paint_get_group_pass_through(i), isolated: mod._paint_get_group_isolated(i) });
  post({ type: 'state', state: { layers, groups, activeLayer: active, canUndo: mod._paint_history_can_undo() !== 0, canRedo: mod._paint_history_can_redo() !== 0, width: docW, height: docH, displayScale: dispScale, mipLevel: dispMip, tilesWidth: mod._paint_get_tiles_width(), tilesHeight: mod._paint_get_tiles_height(), error: mod._paint_get_error_code() } });
}
function handleLayer(op: string, layer: number, value?: number) {
  if (!mod) return;
  switch (op) {
    case 'setActive': mod._paint_set_active_layer(layer); break;
    case 'create': mod._paint_create_layer(); break;
    case 'delete': mod._paint_delete_layer(layer); break;
    case 'move': mod._paint_move_layer(layer, value ?? 0); break;
    case 'setVisible': mod._paint_set_layer_visible(layer, value ? 1 : 0); break;
    case 'setOpacity': mod._paint_set_layer_opacity(layer, value ?? 1); break;
    case 'setMode': mod._paint_set_layer_mode(layer, value ?? 0); break;
    case 'setGroup': mod._paint_set_layer_group(layer, value ?? -1); break;
  } renderDirty(true); pushState();
}
function handleGroup(op: string, group: number, value?: number) {
  if (!mod) return;
  switch (op) {
    case 'create': mod._paint_create_group(); break;
    case 'delete': mod._paint_delete_group(group); break;
    case 'move': mod._paint_move_group(group, value ?? 0); break;
    case 'setParent': mod._paint_set_group_parent(group, value ?? -1); break;
    case 'setVisible': mod._paint_set_group_visible(group, value ? 1 : 0); break;
    case 'setOpacity': mod._paint_set_group_opacity(group, value ?? 1); break;
    case 'setMode': mod._paint_set_group_mode(group, value ?? 0); break;
    case 'setPassThrough': mod._paint_set_group_pass_through(group, value ? 1 : 0); break;
    case 'setIsolated': mod._paint_set_group_isolated(group, value ? 1 : 0); break;
  } renderDirty(true); pushState();
}
function exportTiles(layerId: number | null, id: number) {
  if (!mod) return; const sc = dispScale; const cols = Math.ceil(mod._paint_get_tiles_width() / sc);
  const rows = Math.ceil(mod._paint_get_tiles_height() / sc); const out: ArrayBuffer[] = [];
  for (let ty = 0; ty < rows; ty++) for (let tx = 0; tx < cols; tx++) {
    const ptr = layerId === null ? (mod._paint_render_rgba8_mip_tile_ptr ? mod._paint_render_rgba8_mip_tile_ptr(tx, ty, dispMip) : mod._paint_render_rgba8_tile_ptr(tx, ty)) : mod._paint_render_layer_rgba8_tile_ptr(layerId, tx * sc, ty * sc);
    const t = new Uint8Array(TILE_B); if (ptr) t.set(mod.HEAPU8.subarray(ptr, ptr + TILE_B)); out.push(t.buffer);
  }
  post({ type: 'tiles', id, data: out, scale: 1 }, out);
}
function writeTile(layer: number, tx: number, ty: number, data: ArrayBuffer) {
  if (!mod) return; mod._paint_set_active_layer(layer); const p = mod._malloc(TILE_B);
  try { mod.HEAPU8.set(new Uint8Array(data), p); mod._paint_write_rgba8_tile(tx, ty, p); } finally { mod._free(p); }
  renderDirty(); pushState();
}

let msgCount = 0;
const pending: Msg[] = [];

self.onmessage = async (e: MessageEvent<Msg & { canvas?: OffscreenCanvas }>) => {
  const m = e.data;
  if (m.cmd === 'init') {
    try {
    if (!mod) {
      const dynamicImport = new Function('url', 'return import(url)') as (u: string) => Promise<any>;
      mod = await (await dynamicImport('/wasm/brushlib.js')).default({ locateFile: (p: string) => `/wasm/${p}` });
    }
    docW = m.width; docH = m.height;
    if (m.canvas) { canvas = m.canvas; }
    if (canvas) {
      const ratio = Math.max(docW, docH) / 4096; dispScale = ratio <= 1 ? 1 : ratio <= 2 ? 2 : 4; dispMip = Math.round(Math.log2(dispScale));
      canvas.width = Math.ceil(docW / dispScale); canvas.height = Math.ceil(docH / dispScale);
      ctx = canvas.getContext('2d', { alpha: true });
    }
    if (!mod._init(docW, docH)) { post({ type: 'status', text: 'Init failed.' }); return; }
    rectPtr = mod._malloc(16); const dp = mod._paint_render_rgba8_tile_ptr(0, 0); rgba8 = mod.HEAPU8.subarray(dp, dp + TILE_B);
    mod._paint_set_eotf(EOTF); mod._paint_clear(); mod._paint_set_background_color(bgRGB[0], bgRGB[1], bgRGB[2]);
    renderedMip = -1; renderDirty(true); pushState(); post({ type: 'ready' });
    const q = pending.splice(0, pending.length);
    for (const msg of q) (self.onmessage as Function)({ data: msg });
    return;
    } catch (err) {
      post({ type: 'log', text: 'INIT ERROR: ' + (err as Error).message + ' ' + ((err as Error).stack || '').slice(0, 200) });
      post({ type: 'status', text: 'Engine init failed.' });
      return;
    }
  }
  if (!mod) { pending.push(m); return; }
  msgCount++;
  if (m.cmd === 'beginStroke' || m.cmd === 'commit') post({ type: 'log', text: `#${msgCount} ${m.cmd} q=${motionQueue.length} b=${batching} f=${flushT!==null} c=${commitP}` });
  switch (m.cmd) {
    case 'loadBrush': { const b = mod.lengthBytesUTF8(m.json) + 1; const p = mod._malloc(b); mod.stringToUTF8(m.json, p, b); mod._load_brush(p); mod._free(p); brushOk = true; break; }
    case 'config': m.settings.forEach(([n, v]) => setB(n, v)); break;
    case 'beginStroke': beginStroke(m.x, m.y, m.xtilt, m.ytilt, m.zoom, m.rotation, m.barrel); break;
    case 'strokeSample': motionQueue.push(m.time, m.x, m.y, m.pressure, m.xtilt, m.ytilt, m.zoom, m.rotation, m.barrel, Number.isFinite(m.pressure), true, true); scheduleFlush(); break;
    case 'commit': if (motionQueue.length > 0) { commitP = true; scheduleFlush(); } else doCommit(); break;
    case 'undo': flushNow(); if (mod._paint_history_undo()) renderDirty(true); pushState(); break;
    case 'redo': flushNow(); if (mod._paint_history_redo()) renderDirty(true); pushState(); break;
    case 'clear': flushNow(); mod._paint_clear(); renderDirty(true); pushState(); break;
    case 'clearBackground': flushNow(); mod._paint_clear_background(); renderDirty(true); break;
    case 'setBackground': flushNow(); bgRGB = [m.r, m.g, m.b]; mod._paint_set_background_color(m.r, m.g, m.b); renderDirty(true); break;
    case 'setView': viewMip = m.zoom < 0.75 ? Math.min(2, Math.max(1, Math.floor(Math.log2(1 / m.zoom)))) : 0; renderDirty(true); break;
    case 'layer': handleLayer(m.op, m.layer, m.value); break;
    case 'group': handleGroup(m.op, m.group, m.value); break;
    case 'exportTiles': exportTiles(m.layerId, m.id); break;
    case 'probe': {
      const data = ctx ? ctx.getImageData(0, 0, canvas!.width, canvas!.height).data : null;
      const y = Math.min(canvas!.height - 1, Math.max(0, Math.round(m.y * canvas!.height)));
      const runs: number[] = []; let rs = -1; const w = canvas!.width;
      const br = Math.round(bgRGB[0] * 255), bgc = Math.round(bgRGB[1] * 255), bb = Math.round(bgRGB[2] * 255);
      if (data) { for (let x = 0; x < w; x++) { const o = (y * w + x) * 4; const p = Math.abs(data[o] - br) + Math.abs(data[o + 1] - bgc) + Math.abs(data[o + 2] - bb) > 60; if (p && rs < 0) rs = x; if (!p && rs >= 0) { runs.push(rs, x - 1); rs = -1; } } if (rs >= 0) runs.push(rs, w - 1); }
      post({ type: 'probeResult', id: m.id, y, w, runs, dirtyCount: mod._paint_get_dirty_count(), usedTiles: mod._paint_get_used_tile_count(), rects: lastRects });
      break; }
    case 'writeTile': writeTile(m.layer, m.tx, m.ty, m.data); break;
    case 'requestState': pushState(); break;
  }
};
export {};
