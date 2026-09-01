import { loadBrushModule } from './maipo-wasm';
/* The brush worker owns WASM, input, display, and engine state.
 * The page only sends input and configuration messages.
 */
import { FixedRing } from './fixed-ring.ts';
import { FixedTaskWake } from './fixed-task-wake.ts';
import { MotionQueue } from './paint-input.ts';
import { spawnTilePool, type TilePool } from './paint-tile-pool.ts';
import { PaintTileStore, paintTileKey, type PaintTileRecord } from './paint-tile-store.ts';

type Msg =
  | { cmd: 'init'; width: number; height: number; canvas?: OffscreenCanvas; hardwareConcurrency?: number; documentId?: string }
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

const TILE = 64, TILE_B = TILE * TILE * 4, TILE16_B = TILE_B * 2, EOTF = 2.2;
const BUDGET = 8;
const RESIDENT_TILES = 4096;
const PAGE_TRIGGER = 3584, PAGE_TARGET = 2048, PAGE_RADIUS = 1024;
const PAGE_REGION_LIMIT = 1536, PAGE_WRITE_BATCH = 64;
const INPUT_CAPACITY = 8192, COMMAND_CAPACITY = 8192;
let mod: any = null, motionQueue = new MotionQueue(INPUT_CAPACITY);
const deferredCommands = new FixedRing<Msg>(COMMAND_CAPACITY);
let tileStore: PaintTileStore | null = null;
let tilePager: PaintTilePager | null = null;
let tileDocumentId = '';
let ctx: OffscreenCanvasRenderingContext2D | null = null, canvas: OffscreenCanvas | null = null;
let rgba8: Uint8Array | null = null, rectPtr = 0, jobInfoPtr = 0, renderedMip = -1, lastT = 0;
let docW = 2048, docH = 2048, dispScale = 1, dispMip = 0, viewMip = 0;
let flushT: number | null = null, commitP = false, batching = false;
let n0 = 0, a0 = 0;
let batchInFlight = false, drainInFlight = false, storageInFlight = false, strokeContinuation = false;
let strokeOpen = false, strokePreparing = false;
let bgRGB: [number, number, number] = [0xA8 / 255, 0xA4 / 255, 0x98 / 255];
let statsS = 0, lastBR = 0, lastRR = 0, lastStats = 0;
let lastRects: number[] = [];


class PaintTilePager {
  private engine: any = null;
  private store: PaintTileStore | null = null;
  private documentId = '';
  private scratchPtr = 0;
  private infoPtr = 0;
  private coldMask = 0;
  private prefetchRegion: [number, number, number, number] | null = null;

  configure(engine: any, store: PaintTileStore | null, documentId: string): boolean {
    this.engine = engine;
    this.store = store;
    this.documentId = documentId;
    this.coldMask = 0;
    this.prefetchRegion = null;
    if (!store) return false;
    if (!this.scratchPtr) this.scratchPtr = engine._malloc(TILE16_B);
    if (!this.infoPtr) this.infoPtr = engine._malloc(8);
    if (!this.scratchPtr || !this.infoPtr) {
      this.store = null;
      return false;
    }
    return true;
  }

  private totalUsed(): number {
    if (!this.engine) return 0;
    let total = 0;
    const count = this.engine._paint_get_layer_count();
    for (let layer = 0; layer < count; layer++) {
      total += this.engine._paint_get_layer_used_tile_count(layer);
    }
    return total;
  }

  private bounds(x: number, y: number): [number, number, number, number] | null {
    const tw = this.engine._paint_get_tiles_width();
    const th = this.engine._paint_get_tiles_height();
    const minX = x - PAGE_RADIUS;
    const maxX = x + PAGE_RADIUS;
    const minY = y - PAGE_RADIUS;
    const maxY = y + PAGE_RADIUS;
    const tx0 = Math.max(0, Math.min(tw - 1, Math.floor(minX / TILE)));
    const ty0 = Math.max(0, Math.min(th - 1, Math.floor(minY / TILE)));
    const tx1 = Math.max(0, Math.min(tw - 1, Math.floor(maxX / TILE)));
    const ty1 = Math.max(0, Math.min(th - 1, Math.floor(maxY / TILE)));
    if (tx1 < tx0 || ty1 < ty0 || (tx1 - tx0 + 1) * (ty1 - ty0 + 1) > PAGE_REGION_LIMIT) return null;
    return [tx0, ty0, tx1, ty1];
  }

  private contains(region: [number, number, number, number], x: number, y: number): boolean {
    const tx = Math.floor(x / TILE), ty = Math.floor(y / TILE);
    return tx >= region[0] && tx <= region[2] && ty >= region[1] && ty <= region[3];
  }

  canSample(x: number, y: number): boolean {
    return this.coldMask === 0 || this.prefetchRegion === null ||
      this.contains(this.prefetchRegion, x, y);
  }

  clearColdLayer(layer: number): void {
    this.coldMask &= ~(1 << layer);
    if (this.coldMask === 0) this.prefetchRegion = null;
  }

  deleteLayer(layer: number): void {
    let mask = 0;
    for (let old = 0; old < 32; old++) {
      if (old === layer || !(this.coldMask & (1 << old))) continue;
      const next = old > layer ? old - 1 : old;
      mask |= 1 << next;
    }
    this.coldMask = mask;
    this.prefetchRegion = null;
  }

  prepareStrokePoint(x: number, y: number): Promise<boolean> | null {
    if (!this.engine || !this.store) return null;
    if (this.coldMask === 0 && this.totalUsed() < RESIDENT_TILES) return null;
    const region = this.bounds(x, y);
    if (!region) return Promise.resolve(false);
    if (this.prefetchRegion && this.contains(this.prefetchRegion, x, y) &&
        this.totalUsed() < RESIDENT_TILES) return null;
    return this.prepareAsync(region);
  }

  prepare(x: number, y: number): Promise<boolean> | null {
    if (!this.engine || !this.store) return null;
    if (this.totalUsed() < PAGE_TRIGGER && this.coldMask === 0) return null;
    const region = this.bounds(x, y);
    if (!region) return Promise.resolve(false);
    return this.prepareAsync(region);
  }

  private async prepareAsync(region: [number, number, number, number]): Promise<boolean> {
    try {
      while (this.totalUsed() > PAGE_TARGET) {
        if (!(await this.pageBatch(region))) return false;
      }
      const loaded = await this.loadRegion(region, true);
      if (loaded) this.prefetchRegion = region;
      return loaded;
    } catch (error) {
      post({ type: 'log', text: `IndexedDB tile paging failed: ${(error as Error)?.message ?? error}` });
      return false;
    }
  }

  private findVictim(
    region: [number, number, number, number],
    planned: readonly [number, number, number][],
  ): [number, number, number] | null {
    const [tx0, ty0, tx1, ty1] = region;
    const count = this.engine._paint_get_layer_count();
    for (let layer = 0; layer < count; layer++) {
      const used = this.engine._paint_get_layer_used_tile_count(layer);
      for (let index = 0; index < used; index++) {
        this.engine._paint_get_layer_used_tile_info(layer, index, this.infoPtr);
        const slot = this.infoPtr >> 2;
        const tx = this.engine.HEAP32[slot], ty = this.engine.HEAP32[slot + 1];
        if (tx >= tx0 && tx <= tx1 && ty >= ty0 && ty <= ty1) continue;
        let alreadyPlanned = false;
        for (const item of planned) {
          if (item[0] === layer && item[1] === tx && item[2] === ty) {
            alreadyPlanned = true;
            break;
          }
        }
        if (!alreadyPlanned) return [layer, tx, ty];
      }
    }
    return null;
  }

  private async pageBatch(region: [number, number, number, number]): Promise<boolean> {
    const candidates: { key: PaintTileRecord['key']; layer: number; tx: number; ty: number; data: ArrayBuffer }[] = [];
    const planned: [number, number, number][] = [];
    while (this.totalUsed() - candidates.length > PAGE_TARGET && candidates.length < PAGE_WRITE_BATCH) {
      const victim = this.findVictim(region, planned);
      if (!victim) break;
      const [layer, tx, ty] = victim;
      const ptr = this.engine._paint_get_layer_tile_ptr(layer, tx, ty);
      if (!ptr) break;
      const bytes = new Uint8Array(TILE16_B);
      bytes.set(this.engine.HEAPU8.subarray(ptr, ptr + TILE16_B));
      candidates.push({ key: paintTileKey(this.documentId, layer, tx, ty), layer, tx, ty, data: bytes.buffer });
      planned.push(victim);
    }
    if (candidates.length === 0) {
      post({ type: 'log', text: 'IndexedDB found no evictable resident tile in the brush area.' });
      return false;
    }
    await this.store!.putMany(candidates);
    for (const candidate of candidates) {
      if (!this.engine._paint_remove_layer_tile(candidate.layer, candidate.tx, candidate.ty)) {
        post({ type: 'log', text: `IndexedDB could not evict tile (${candidate.tx}, ${candidate.ty}) from layer ${candidate.layer}.` });
        return false;
      }
      this.coldMask |= 1 << candidate.layer;
    }
    return true;
  }

  private async pageOne(region: [number, number, number, number]): Promise<boolean> {
    const victim = this.findVictim(region, []);
    if (!victim) {
      post({ type: 'log', text: 'IndexedDB found no evictable resident tile in the brush area.' });
      return false;
    }
    const [layer, tx, ty] = victim;
    const ptr = this.engine._paint_get_layer_tile_ptr(layer, tx, ty);
    if (!ptr) return false;
    const bytes = new Uint8Array(TILE16_B);
    bytes.set(this.engine.HEAPU8.subarray(ptr, ptr + TILE16_B));
    await this.store!.put(paintTileKey(this.documentId, layer, tx, ty), bytes.buffer);
    if (!this.engine._paint_remove_layer_tile(layer, tx, ty)) {
      post({ type: 'log', text: `IndexedDB could not evict tile (${tx}, ${ty}) from layer ${layer}.` });
      return false;
    }
    this.coldMask |= 1 << layer;
    return true;
  }

  private async loadRegion(
    region: [number, number, number, number],
    allowEvict: boolean,
  ): Promise<boolean> {
    const [tx0, ty0, tx1, ty1] = region;
    const layerCount = this.engine._paint_get_layer_count();
    for (let layer = 0; layer < layerCount; layer++) {
      if ((this.coldMask & (1 << layer)) === 0) continue;
      const records = await this.store!.getRegion(
        this.documentId, layer, tx0, ty0, tx1, ty1,
      );
      for (const record of records) {
        const tx = record.key[2], ty = record.key[3];
        if (this.engine._paint_get_layer_tile_ptr(layer, tx, ty)) continue;
        if (record.data.byteLength !== TILE16_B) {
          throw new Error(`Invalid tile size at (${tx}, ${ty}).`);
        }
        while (this.totalUsed() >= RESIDENT_TILES) {
          if (!allowEvict || !(await this.pageOne(region))) return false;
        }
        this.engine.HEAPU8.set(new Uint8Array(record.data), this.scratchPtr);
        if (!this.engine._paint_write_layer_rgba16_tile(layer, tx, ty, this.scratchPtr)) {
          throw new Error(`Tile restore failed at (${tx}, ${ty}).`);
        }
      }
    }
    return true;
  }
}

function runDrainWake() {
  if (!mod || batchInFlight || drainInFlight || strokePreparing || motionQueue.length === 0) return;
  startDrain(true);
}
const drainWake = new FixedTaskWake(runDrainWake);
let tilePool: TilePool | null = null;

/// Parallel drain of the published blend jobs: each job (one dirty tile)
/// is serialized — the op bytes + the tile bytes — and blended off-thread
/// by the tile pool. Falls back to inline draining without a pool.
/// Returns the job count (0 when the batch had no dirty tiles).
async function drainBlendJobs(): Promise<number> {
  const n = mod._paint_end_batch_parallel();
  if (n <= 0) return n;
  const opSize = mod._paint_draw_dab_op_size();

  if (tilePool?.ready()) {
    const jobs: Promise<Uint8Array>[] = [];
    const info = new Int32Array(mod.memory.buffer, jobInfoPtr, 5);
    const opArenaPtr = mod._paint_ops_arena_ptr();
    const heap = new Uint8Array(mod.memory.buffer);
    for (let i = 0; i < n; i++) {
      mod._paint_get_job_info(i, jobInfoPtr);
      const opsOff = info[0], opCount = info[1], tileAddr = info[2], tx = info[3], ty = info[4];
      const opsStart = opArenaPtr + opsOff * opSize;
      const ops = heap.slice(opsStart, opsStart + opCount * opSize);
      const tile = heap.slice(tileAddr, tileAddr + 64 * 64 * 4 * 2);
      jobs.push(tilePool.blend({ id: i, tx, ty, opCount, ops, tile }));
    }
    try {
      const blended = await Promise.all(jobs);
      const resultHeap = new Uint8Array(mod.memory.buffer);
      for (let i = 0; i < n; i++) {
        mod._paint_get_job_info(i, jobInfoPtr);
        const tileAddr2 = info[2];
        resultHeap.set(blended[i], tileAddr2);
      }
      return n;
    } catch (err) {
      post({ type: 'log', text: `tile pool fallback: ${(err as Error)?.message ?? err}` });
      for (let i = 0; i < n; i++) mod._paint_process_tile_job(i, 0);
      return n;
    }
  }
  for (let i = 0; i < n; i++) mod._paint_process_tile_job(i, 0);
  return n;
}

const post = (m: any, t?: Transferable[]) => { if (t && t.length > 0) (self as unknown as Worker).postMessage(m, t); else (self as unknown as Worker).postMessage(m); };

function reportEngineError(what: string, status = 'Brush engine error. See the log.') {
  post({ type: 'log', text: 'ENGINE ERROR: ' + what });
  post({ type: 'status', text: status });
}
function finishStorageFailure() {
  if (!mod) return;
  motionQueue.clear();
  strokeContinuation = false;
  commitP = false;
  mod._paint_cancel_stroke();
  if (batching) {
    batching = false;
    try { mod._paint_end_batch(); } catch {}
  }
  mod._paint_history_commit();
  strokeOpen = false;
  pushState();
  reportEngineError('IndexedDB tile storage could not make room.', 'Paint storage is full.');
}
self.onerror = (e: any) => { reportEngineError('WORKER ERROR: ' + ((e && (e.message || e.error || e)) ?? String(e))); };
self.onunhandledrejection = (e: any) => { reportEngineError('WORKER UNHANDLED REJECTION: ' + ((e && e.reason && (e.reason.message || e.reason)) || String(e))); };

function setB(n: string, v: number) { const b = mod.lengthBytesUTF8(n) + 1; const p = mod._malloc(b); if (!p) throw new Error(`No memory for brush setting ${n}.`); mod.stringToUTF8(n, p, b); mod._set_brush_base_value(p, v); mod._free(p); }
function mipLevel() { return dispMip > viewMip ? dispMip : viewMip; }
function fillBg() { if (!ctx || !canvas) return; const [r, g, b] = bgRGB; ctx.fillStyle = `rgb(${Math.round(r * 255)},${Math.round(g * 255)},${Math.round(b * 255)})`; ctx.fillRect(0, 0, canvas.width, canvas.height); }
let tileImg: ImageData | null = null;
let mipImg: ImageData | null = null;
let mipImgSz = 0;
function drawTile(tx: number, ty: number, scale: number) {
  if (!ctx) return; const s = rgba8!;
  if (scale === 1) {
    if (!tileImg) tileImg = new ImageData(TILE, TILE);
    tileImg.data.set(s);
    ctx.putImageData(tileImg, tx * TILE, ty * TILE);
    return;
  }
  const sz = TILE * scale;
  if (!mipImg || mipImgSz !== sz) { mipImg = new ImageData(sz, sz); mipImgSz = sz; }
  const img = mipImg;
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
  const mw = ml > 0 ? Math.ceil(tw / sc) : tw, mh = ml > 0 ? Math.ceil(th / sc) : th;
  lastRects = [];
  for (let i = 0; i < cnt; i++) {
    mod._paint_get_dirty_rect(i, rectPtr); const b = rectPtr >> 2;
    lastRects.push(mod.HEAP32[b], mod.HEAP32[b+1], mod.HEAP32[b+2], mod.HEAP32[b+3]);
  }
  const dirtyTiles = mod._paint_get_dirty_tile_count();
  if (dirtyTiles > 0) {
    const seenMip = new Set<number>();
    for (let i = 0; i < dirtyTiles; i++) {
      mod._paint_get_dirty_tile_info(i, rectPtr); const b = rectPtr >> 2;
      const tx = mod.HEAP32[b] >> ml, ty = mod.HEAP32[b+1] >> ml;
      if (tx < 0 || ty < 0 || tx >= mw || ty >= mh) continue;
      const key = ty * mw + tx;
      if (seenMip.has(key)) continue;
      seenMip.add(key);
      renderTile(tx, ty, ml);
    }
  } else {
    for (let i = 0; i < cnt; i++) {
      const x = lastRects[i * 4], y = lastRects[i * 4 + 1], w = lastRects[i * 4 + 2], h = lastRects[i * 4 + 3];
      const ts = TILE * sc;
      const tx0 = Math.max(0, Math.floor(x / ts)), ty0 = Math.max(0, Math.floor(y / ts));
      const tx1 = Math.min(mw - 1, Math.floor((x + w) / ts)), ty1 = Math.min(mh - 1, Math.floor((y + h) / ts));
      for (let ty = ty0; ty <= ty1; ty++) for (let tx = tx0; tx <= tx1; tx++) renderTile(tx, ty, ml);
    }
  }
  mod._paint_clear_dirty(); checkErr();
}
function checkErr() {
  const c = mod._paint_get_error_code();
  if (!c) return;
  if (c === 1) reportEngineError('Paint tile allocation failed.', 'Paint storage allocation failed.');
  else if (c === 2) reportEngineError('The undo history reached its fixed capacity.', 'Undo history capacity reached.');
  else if (c === 3) reportEngineError('The libmypaint dab loop made no progress.');
  else if (c === 4) reportEngineError('The brush operation queue reached its fixed capacity.');
  else reportEngineError(`Unknown paint error code ${c}.`);
  mod._paint_clear_error();
}
let stopBatch = false;
function strokeSample(_t: number, x: number, y: number, p: number, xt: number, yt: number, z: number, r: number, ba: number): boolean {
  if (stopBatch) return false;
  if (!strokeContinuation && tilePager && !tilePager.canSample(x, y)) return false;
  const result = strokeContinuation
    ? mod._paint_continue_stroke_to()
    : mod._stroke_to(x, y, p, xt, yt, lastT > 0 ? Math.max(0, (_t - lastT) * 0.001) : 0.016, z, r, ba, 0);
  if (result === 0) {
    strokeContinuation = true;
    return false;
  }
  strokeContinuation = false;
  lastT = _t;
  if (result < 0) {
    if (mod._paint_get_error_code()) checkErr();
    else reportEngineError('The brush continuation state is incorrect.');
  } else if (mod._paint_get_error_code()) {
    checkErr();
  }
  // ponytail: one input sample per batch keeps the fixed operation queue bounded.
  stopBatch = true;
  return true;
}
function emitStats(force = false) {
  const now = performance.now();
  if (!force && now - lastStats < 150) return;
  const wall = now - lastStats;
  lastStats = now;
  post({
    type: 'stats',
    queued: motionQueue.length,
    deferred: deferredCommands.length,
    brushMs: lastBR,
    renderMs: lastRR,
    sps: wall > 0 ? Math.round((statsS / wall) * 1000) : 0,
  });
  statsS = 0;
}

function deferCommand(command: Msg) {
  if (!deferredCommands.push(command)) {
    reportEngineError('The deferred command queue reached its fixed capacity.');
  }
}

function replayDeferredCommands() {
  while (deferredCommands.length > 0) {
    if (storageInFlight || drainInFlight || strokePreparing) return;
    const next = deferredCommands.peek();
    if (!next) return;
    if (motionQueue.length > 0 &&
        next.cmd !== 'strokeSample' && next.cmd !== 'commit') return;
    deferredCommands.shift();
    handleReadyCommand(next);
    if (batchInFlight || commitP || storageInFlight || drainInFlight || strokePreparing) return;
  }
}

function afterBatch() {
  batchInFlight = false;
  const renderStart = performance.now();
  renderDirty();
  lastRR = performance.now() - renderStart;
  statsS += Math.max(0, n0 - motionQueue.length);
  emitStats(
    motionQueue.length === 0 && deferredCommands.length === 0 &&
    !strokeContinuation,
  );
  if (motionQueue.length > 0) {
    if (flushT !== null) {
      clearTimeout(flushT);
      flushT = null;
    }
    drainWake.schedule();
    return;
  }
  if (commitP) {
    commitP = false;
    doCommit();
    if (batchInFlight || commitP) return;
  }
  replayDeferredCommands();
  if (!batchInFlight && !commitP && motionQueue.length === 0 &&
      deferredCommands.length === 0 && !strokeContinuation) {
    emitStats(true);
  }
}

function pollBatch() {
  if (!mod) return;
  if (mod._paint_is_batch_done()) {
    try {
      mod._paint_end_batch_finish();
    } catch (err) {
      reportEngineError('Batch finish failed: ' + ((err as Error)?.message ?? err));
      return;
    }
    afterBatch();
    return;
  }
  self.setTimeout(pollBatch, 2);
}

function startDrain(bounded: boolean) {
  if (!mod || batchInFlight || drainInFlight || strokePreparing || motionQueue.length === 0) return;
  const wait = strokeOpen && tilePager
    ? tilePager.prepareStrokePoint(motionQueue.peekX(), motionQueue.peekY())
    : null;
  if (!wait) {
    drainProcess(bounded);
    return;
  }
  drainInFlight = true;
  let failed = false;
  void wait.then(ok => {
    if (ok) drainProcess(bounded);
    else {
      failed = true;
      finishStorageFailure();
    }
  }).catch(err => {
    failed = true;
    post({ type: 'log', text: `IndexedDB tile prefetch failed: ${(err as Error)?.message ?? err}` });
    finishStorageFailure();
  }).then(() => {
    drainInFlight = false;
    if (failed) replayDeferredCommands();
    else if (motionQueue.length > 0 && !batchInFlight && !strokePreparing) drainWake.schedule();
  });
}
function drainProcess(bounded: boolean) {
  if (!mod || batchInFlight || strokePreparing) return;
  if (!batching) { mod._paint_begin_batch(); batching = true; }
  n0 = motionQueue.length;
  a0 = performance.now();
  stopBatch = false;
  if (bounded) motionQueue.drainInterpolatedBounded(strokeSample, BUDGET);
  else motionQueue.drainInterpolated(strokeSample);
  lastBR = performance.now() - a0;
  if (batching) {
    batching = false;
    batchInFlight = true;
    void drainBlendJobs().then(() => {
      mod._paint_end_batch_finish();
      batchInFlight = false;
      afterBatch();
    }).catch((err) => {
      reportEngineError('Batch drain failed: ' + ((err as Error)?.stack ?? err));
      batchInFlight = false;
    });
    return;
  }
  if (!mod._paint_is_batch_done()) {
    batchInFlight = true;
    pollBatch();
    return;
  }
  afterBatch();
}
function scheduleFlush() {
  if (flushT !== null || batchInFlight || drainInFlight || storageInFlight || strokePreparing || drainWake.isPending) return;
  flushT = self.setTimeout(() => {
    flushT = null;
    if (!mod || batchInFlight || strokePreparing) return;
    startDrain(true);
  }, 8);
}
function flushNow() {
  if (flushT !== null) { clearTimeout(flushT); flushT = null; }
  if (!mod || batchInFlight || drainInFlight || strokePreparing) return;
  startDrain(false);
}
function doCommit() {
  if (!mod) return;
  if (batchInFlight || drainInFlight || strokePreparing || motionQueue.length > 0 || strokeContinuation) {
    commitP = true;
    return;
  }
  if (batching) {
    batching = false;
    batchInFlight = true;
    void drainBlendJobs().then(() => {
      mod._paint_end_batch_finish();
      mod._paint_history_commit();
      batchInFlight = false;
      strokeOpen = false;
      pushState();
      replayDeferredCommands();
    }).catch((err) => {
      reportEngineError('Commit failed: ' + ((err as Error)?.message ?? err));
      batchInFlight = false;
      strokeOpen = false;
      pushState();
      replayDeferredCommands();
    });
    return;
  }
  mod._paint_history_commit();
  strokeOpen = false;
  pushState();
}
async function beginStroke(x: number, y: number, xt: number, yt: number, z: number, r: number, ba: number) {
  if (!mod) return;
  if (strokeOpen || strokePreparing) {
    reportEngineError('A new stroke started before the current stroke ended.');
    return;
  }
  motionQueue.clear();
  lastT = 0;
  strokeOpen = true;
  strokePreparing = true;
  try {
    const wait = tilePager?.prepare(x, y);
    if (wait && !(await wait)) {
      motionQueue.clear();
      commitP = false;
      strokeOpen = false;
      reportEngineError('IndexedDB tile storage could not make room.', 'Paint storage is full.');
      pushState();
      replayDeferredCommands();
      return;
    }
    mod._begin_stroke(x, y, xt, yt, z, r, ba);
    mod._paint_begin_batch();
    batching = true;
    scheduleFlush();
  } catch (error) {
    motionQueue.clear();
    commitP = false;
    strokeOpen = false;
    reportEngineError(`Stroke start failed: ${(error as Error)?.message ?? error}`);
    pushState();
    replayDeferredCommands();
  } finally {
    strokePreparing = false;
    if (strokeOpen && batching) scheduleFlush();
    replayDeferredCommands();
  }
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
async function clearPaint() {
  if (!mod || storageInFlight) return;
  storageInFlight = true;
  try {
    const layer = mod._paint_get_active_layer();
    if (tileStore) await tileStore.dropLayer(tileDocumentId, layer);
    tilePager?.clearColdLayer(layer);
    mod._reset_brush();
    mod._paint_clear();
    renderDirty(true);
    pushState();
  } catch (error) {
    reportEngineError(`Paint clear failed: ${(error as Error)?.message ?? error}`);
  } finally {
    storageInFlight = false;
    replayDeferredCommands();
  }
}

async function deleteLayer(layer: number) {
  if (!mod || storageInFlight) return;
  const count = mod._paint_get_layer_count();
  if (layer < 0 || layer >= count || count <= 1) return;
  storageInFlight = true;
  try {
    if (tileStore) await tileStore.deleteLayer(tileDocumentId, layer);
    if (mod._paint_delete_layer(layer)) {
      tilePager?.deleteLayer(layer);
      renderDirty(true);
      pushState();
    }
  } catch (error) {
    reportEngineError(`Layer delete failed: ${(error as Error)?.message ?? error}`);
  } finally {
    storageInFlight = false;
    replayDeferredCommands();
  }
}

function handleLayer(op: string, layer: number, value?: number) {
  if (!mod) return;
  if (op === 'delete') {
    void deleteLayer(layer);
    return;
  }
  switch (op) {
    case 'setActive': mod._paint_set_active_layer(layer); break;
    case 'create': mod._paint_create_layer(); break;
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
  if (!mod) return;
  /* Export at full document resolution: one 64x64 tile per source tile.
   * The old mip-grid path truncated docs larger than the display canvas. */
  const tw = mod._paint_get_tiles_width(), th = mod._paint_get_tiles_height();
  const out: ArrayBuffer[] = [];
  for (let ty = 0; ty < th; ty++) for (let tx = 0; tx < tw; tx++) {
    const ptr = layerId === null
      ? mod._paint_render_rgba8_tile_ptr(tx, ty)
      : mod._paint_render_layer_rgba8_tile_ptr(layerId, tx, ty);
    const t = new Uint8Array(TILE_B); if (ptr) t.set(mod.HEAPU8.subarray(ptr, ptr + TILE_B)); out.push(t.buffer);
  }
  post({ type: 'tiles', id, data: out, scale: 1 }, out);
}
function writeTile(layer: number, tx: number, ty: number, data: ArrayBuffer) {
  if (!mod) return; mod._paint_set_active_layer(layer); const p = mod._malloc(TILE_B);
  if (!p) { reportEngineError('No memory is available for a tile write.'); return; }
  try {
    mod.HEAPU8.set(new Uint8Array(data), p);
    if (!mod._paint_write_rgba8_tile(tx, ty, p)) reportEngineError(`Tile write failed at (${tx}, ${ty}).`);
  } finally { mod._free(p); }
  renderDirty(); pushState();
}

const pendingInitCommands = new FixedRing<Msg>(COMMAND_CAPACITY);

function handleReadyCommand(m: Msg) {
  switch (m.cmd) {
    case 'loadBrush': {
      const bytes = mod.lengthBytesUTF8(m.json) + 1;
      const pointer = mod._malloc(bytes);
      if (!pointer) {
        reportEngineError('No memory is available for brush data.');
        break;
      }
      mod.stringToUTF8(m.json, pointer, bytes);
      const loaded = mod._load_brush(pointer);
      mod._free(pointer);
      if (!loaded) {
        reportEngineError('Brush load failed because the .myb data is incorrect.');
      }
      break;
    }
    case 'config': m.settings.forEach(([name, value]) => {
      try { setB(name, value); }
      catch (err) { post({ type: 'log', text: `config ${name} failed: ${(err as Error)?.message ?? err}` }); }
    }); break;
    case 'beginStroke': void beginStroke(m.x, m.y, m.xtilt, m.ytilt, m.zoom, m.rotation, m.barrel); break;
    case 'strokeSample':
      if (!strokeOpen) {
        reportEngineError('A stroke sample has no active stroke.');
        break;
      }
      if (!motionQueue.push(
        m.time, m.x, m.y, m.pressure, m.xtilt, m.ytilt, m.zoom,
        m.rotation, m.barrel, Number.isFinite(m.pressure), true, true,
      )) {
        reportEngineError('The motion queue reached its fixed capacity.');
      }
      scheduleFlush();
      break;
    case 'commit':
      if (!strokeOpen) break;
      if (batchInFlight || motionQueue.length > 0 || strokeContinuation) {
        commitP = true;
        scheduleFlush();
      } else doCommit();
      break;
    case 'undo': flushNow(); mod._reset_brush(); if (mod._paint_history_undo()) renderDirty(true); pushState(); break;
    case 'redo': flushNow(); mod._reset_brush(); if (mod._paint_history_redo()) renderDirty(true); pushState(); break;
    case 'clear': flushNow(); void clearPaint(); break;
    case 'clearBackground': flushNow(); mod._reset_brush(); mod._paint_clear_background(); renderDirty(true); break;
    case 'setBackground': flushNow(); bgRGB = [m.r, m.g, m.b]; mod._paint_set_background_color(m.r, m.g, m.b); renderDirty(true); break;
    case 'setView': {
      const before = mipLevel();
      viewMip = m.zoom < 0.75 ? Math.min(2, Math.max(1, Math.floor(Math.log2(1 / m.zoom)))) : 0;
      if (mipLevel() !== before) renderDirty(true);
      break;
    }
    case 'layer': handleLayer(m.op, m.layer, m.value); break;
    case 'group': handleGroup(m.op, m.group, m.value); break;
    case 'exportTiles': exportTiles(m.layerId, m.id); break;
    case 'probe': {
      const currentCanvas = canvas, currentCtx = ctx;
      const data = currentCtx && currentCanvas
        ? currentCtx.getImageData(0, 0, currentCanvas.width, currentCanvas.height).data
        : null;
      const w = currentCanvas?.width ?? 0;
      const h = currentCanvas?.height ?? 1;
      const y = Math.min(h - 1, Math.max(0, Math.round(m.y * h)));
      const runs: number[] = []; let rs = -1;
      const br = Math.round(bgRGB[0] * 255), bgc = Math.round(bgRGB[1] * 255), bb = Math.round(bgRGB[2] * 255);
      let alpha0 = 0, painted = 0;
      const samples: number[] = [];
      if (data) {
        for (let x = 0; x < w; x++) {
          const o = (y * w + x) * 4; const a = data[o + 3];
          if (a === 0) alpha0++;
          const p = a > 0 && Math.abs(data[o] - br) + Math.abs(data[o + 1] - bgc) + Math.abs(data[o + 2] - bb) > 60;
          if (p) painted++;
          if (p && rs < 0) rs = x;
          if (!p && rs >= 0) { runs.push(rs, x - 1); rs = -1; }
        }
        if (rs >= 0) runs.push(rs, w - 1);
        for (const sx of [Math.floor(w * 0.1), Math.floor(w * 0.5), Math.floor(w * 0.9)]) {
          const o = (y * w + sx) * 4;
          samples.push(sx, data[o], data[o + 1], data[o + 2], data[o + 3]);
        }
      }
      post({ type: 'probeResult', id: m.id, y, w, runs, alpha0, painted, samples, dirtyCount: mod._paint_get_dirty_count(), usedTiles: mod._paint_get_used_tile_count(), rects: lastRects });
      break; }
    case 'writeTile': writeTile(m.layer, m.tx, m.ty, m.data); break;
    case 'requestState': pushState(); break;
    case 'init': break;
  }
}

function routeReadyCommand(m: Msg) {
  const paintPending = batchInFlight || drainInFlight || storageInFlight || strokePreparing ||
    strokeContinuation || motionQueue.length > 0 || batching || commitP;
  const joinsCurrentStroke = strokeOpen && !batchInFlight &&
    (m.cmd === 'strokeSample' || m.cmd === 'commit');
  if (deferredCommands.length > 0 ||
      (m.cmd === 'beginStroke' && strokeOpen) ||
      (paintPending && !joinsCurrentStroke)) {
    deferCommand(m);
    scheduleFlush();
    return;
  }
  handleReadyCommand(m);
}

async function dispatchInput(e: MessageEvent<Msg & { canvas?: OffscreenCanvas }>) {
  const m = e.data;
  if (m.cmd === 'init') {
    try {
      if (!mod) {
        mod = await loadBrushModule();
        void spawnTilePool(
          mod._paint_draw_dab_op_size(),
          (t) => post({ type: 'log', text: t }),
          m.hardwareConcurrency,
        )
          .then((p) => { tilePool = p; })
          .catch((err) => {
            post({ type: "log", text: `tile pool unavailable: ${(err as Error)?.message ?? err}` });
          });
      }
      const nextDocumentId = m.documentId ?? `paint-${Date.now()}-${Math.random()}`;
      if (!tileStore) {
        const candidate = new PaintTileStore();
        try {
          await candidate.open();
          await candidate.clear();
          tileStore = candidate;
        } catch (err) {
          post({ type: 'log', text: `IndexedDB unavailable: ${(err as Error)?.message ?? err}` });
        }
      }
      if (tileStore && tileDocumentId && tileDocumentId !== nextDocumentId) {
        try { await tileStore.dropDocument(tileDocumentId); }
        catch (err) { post({ type: 'log', text: `IndexedDB cleanup failed: ${(err as Error)?.message ?? err}` }); }
      }
      tileDocumentId = nextDocumentId;
      docW = m.width; docH = m.height;
      if (m.canvas) canvas = m.canvas;
      if (canvas) {
        const ratio = Math.max(docW, docH) / 4096;
        dispScale = ratio <= 1 ? 1 : ratio <= 2 ? 2 : 4;
        dispMip = Math.round(Math.log2(dispScale));
        canvas.width = Math.ceil(docW / dispScale);
        canvas.height = Math.ceil(docH / dispScale);
        ctx = canvas.getContext('2d', { alpha: true });
      }
      if (!mod._init(docW, docH)) {
        reportEngineError('Brush engine initialization failed.', 'Engine initialization failed.');
        return;
      }
      if (!rectPtr) rectPtr = mod._malloc(16);
      if (!jobInfoPtr) jobInfoPtr = mod._malloc(20);
      if (!rectPtr || !jobInfoPtr) {
        reportEngineError('No memory is available for display data.');
        return;
      }
      const displayPointer = mod._paint_render_rgba8_tile_ptr(0, 0);
      rgba8 = mod.HEAPU8.subarray(displayPointer, displayPointer + TILE_B);
      mod._paint_set_eotf(EOTF);
      if (!tilePager) tilePager = new PaintTilePager();
      tilePager.configure(mod, tileStore, tileDocumentId);
      mod._paint_clear();
      mod._paint_set_background_color(bgRGB[0], bgRGB[1], bgRGB[2]);
      motionQueue.clear();
      deferredCommands.clear();
      strokeContinuation = false;
      strokeOpen = false;
      commitP = false;
      batching = false;
      batchInFlight = false;
      drainInFlight = false;
      storageInFlight = false;
      strokePreparing = false;
      renderedMip = -1;
      renderDirty(true);
      pushState();
      post({ type: 'ready' });
      while (pendingInitCommands.length > 0) {
        const command = pendingInitCommands.shift();
        if (command) routeReadyCommand(command);
      }
      return;
    } catch (err) {
      post({ type: 'log', text: 'INIT ERROR: ' + (err as Error).message + ' ' + ((err as Error).stack || '').slice(0, 200) });
      post({ type: 'status', text: 'Engine init failed.' });
      return;
    }
  }
  if (!mod) {
    if (!pendingInitCommands.push(m)) {
      reportEngineError('The initial command queue reached its fixed capacity.');
    }
    return;
  }
  routeReadyCommand(m);
}

async function waitForPaintIdle(): Promise<void> {
  if (!mod) return;
  if (strokeOpen) {
    commitP = true;
    if (!batchInFlight && !drainInFlight && !storageInFlight && !strokePreparing &&
        motionQueue.length === 0 && !strokeContinuation) {
      doCommit();
    } else {
      scheduleFlush();
    }
  }
  while (strokeOpen || strokePreparing || batching || batchInFlight ||
      drainInFlight || storageInFlight || motionQueue.length > 0 || strokeContinuation ||
      commitP || deferredCommands.length > 0) {
    if (!strokeOpen && !batchInFlight && !drainInFlight && !storageInFlight &&
        deferredCommands.length > 0) {
      replayDeferredCommands();
    }
    await new Promise<void>(resolve => self.setTimeout(resolve, 0));
  }
}

let initChain: Promise<void> = Promise.resolve();
async function handleInput(e: MessageEvent<Msg & { canvas?: OffscreenCanvas }>) {
  const m = e.data;
  if (m.cmd === 'init') {
    const task = initChain.then(async () => {
      await waitForPaintIdle();
      await dispatchInput(e);
    });
    initChain = task.catch(() => {});
    await task;
    return;
  }
  await initChain;
  await dispatchInput(e);
}

self.onmessage = (e: MessageEvent<Msg & { canvas?: OffscreenCanvas }>) => {
  handleInput(e)?.catch?.((err: unknown) => {
    const what = `Input command failed: ${(err as Error)?.message ?? err} ${((err as Error)?.stack || '').slice(0, 200)}`;
    reportEngineError(what);
  });
};
export {};
