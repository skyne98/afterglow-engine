import { loadBrushModule } from './maipo-wasm.ts';
/* The brush worker owns WASM, input, display, and engine state.
 * The page only sends input and configuration messages.
 */
import { FixedRing } from './fixed-ring.ts';
import { FixedTaskWake } from './fixed-task-wake.ts';
import { PaintHistoryQueue, type PaintHistoryOperation } from './paint-history.ts';
import { MotionQueue } from './paint-input.ts';
import { paintMemoryLimitMiB, paintMemoryPolicy, paintTileWorkerCount } from './paint-memory.ts';
import { spawnTilePool, type TilePool } from './paint-tile-pool.ts';
import { PaintTileStore, paintEvictionRankBefore, paintTileKey, protectedTileRegionContains, type PaintEvictionRank, type PaintHistoryKey, type PaintHistoryRecord, type PaintTileKey, type PaintTileRecord } from './paint-tile-store.ts';

type Msg =
  | { cmd: 'init'; width: number; height: number; canvas?: OffscreenCanvas; hardwareConcurrency?: number; memoryLimitMiB?: number; documentId?: string }
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
  | { cmd: 'pickColor'; id: number; x: number; y: number }
  | { cmd: 'requestState' };

const TILE = 64, TILE_B = TILE * TILE * 4, TILE16_B = TILE_B * 2, EOTF = 2.2;
const BUDGET = 8;
const PAGE_RADIUS = 1024, PAGE_PREDICTION_MS = 100, PAGE_PREDICTION_MAX = 512;
const PAGE_GUARD_TILES = 8, PAGE_REGION_LIMIT = 1536, PAGE_WRITE_BATCH = 64;
const PAGE_ALLOCATION_HEADROOM = 512;
const INPUT_CAPACITY = 8192, COMMAND_CAPACITY = 8192;
let mod: any = null, motionQueue = new MotionQueue(INPUT_CAPACITY);
const deferredCommands = new FixedRing<Msg>(COMMAND_CAPACITY);
let tileStore: PaintTileStore | null = null;
let tilePager: PaintTilePager | null = null;
let diskHistory: PaintDiskHistory | null = null;
let tileDocumentId = '';
let ctx: OffscreenCanvasRenderingContext2D | null = null, canvas: OffscreenCanvas | null = null;
let rgba8: Uint8Array | null = null, rectPtr = 0, jobInfoPtr = 0, renderedMip = -1, lastT = 0;
let docW = 2048, docH = 2048, dispScale = 1, dispMip = 0, viewMip = 0;
let flushT: number | null = null, commitP = false, batching = false;
let n0 = 0, a0 = 0;
let batchInFlight = false, drainInFlight = false, storageInFlight = false, strokeContinuation = false;
let strokeOpen = false, strokePreparing = false, hasStrokePoint = false;
let lastStrokeX = 0, lastStrokeY = 0;
let bgRGB: [number, number, number] = [0xA8 / 255, 0xA4 / 255, 0x98 / 255];
let statsS = 0, lastBR = 0, lastRR = 0, lastStats = 0, lastDisplay = 0;
const DISPLAY_INTERVAL_MS = 16;
let lastRects: number[] = [];


class PaintTilePager {
  private engine: any = null;
  private store: PaintTileStore | null = null;
  private documentId = '';
  private scratchPtr = 0;
  private infoPtr = 0;
  private coldMask = 0;
  private prefetchRegion: [number, number, number, number] | null = null;
  private maximumTiles = 0;
  private growTiles = 0;
  private focusX = 0;
  private focusY = 0;
  private directionX = 0;
  private directionY = 0;
  private requestGeneration = 0;

  configure(
    engine: any,
    store: PaintTileStore | null,
    documentId: string,
    maximumTiles: number,
    growTiles: number,
  ): boolean {
    this.engine = engine;
    this.store = store;
    this.documentId = documentId;
    this.maximumTiles = maximumTiles;
    this.growTiles = growTiles;
    this.coldMask = 0;
    this.prefetchRegion = null;
    this.directionX = 0;
    this.directionY = 0;
    this.requestGeneration++;
    if (!store) return true;
    if (!this.scratchPtr) this.scratchPtr = engine._malloc(TILE16_B);
    if (!this.infoPtr) this.infoPtr = engine._malloc(8);
    if (!this.scratchPtr || !this.infoPtr) {
      this.store = null;
      return false;
    }
    return true;
  }

  private totalUsed(): number {
    return this.engine?._paint_get_resident_tile_count() ?? 0;
  }

  private growForHeadroom(): boolean {
    const used = this.totalUsed();
    const limit = this.engine._paint_get_resident_tile_limit();
    if (used + PAGE_ALLOCATION_HEADROOM < limit || limit >= this.maximumTiles) return false;
    const next = Math.min(this.maximumTiles, limit + this.growTiles);
    return this.engine._paint_set_resident_tile_limit(next) !== 0;
  }

  private bounds(
    x: number,
    y: number,
    previousX = x,
    previousY = y,
  ): [number, number, number, number] | null {
    const tw = this.engine._paint_get_tiles_width();
    const th = this.engine._paint_get_tiles_height();
    const minX = Math.min(x, previousX) - PAGE_RADIUS;
    const maxX = Math.max(x, previousX) + PAGE_RADIUS;
    const minY = Math.min(y, previousY) - PAGE_RADIUS;
    const maxY = Math.max(y, previousY) + PAGE_RADIUS;
    const tx0 = Math.max(0, Math.min(tw - 1, Math.floor(minX / TILE)));
    const ty0 = Math.max(0, Math.min(th - 1, Math.floor(minY / TILE)));
    const tx1 = Math.max(0, Math.min(tw - 1, Math.floor(maxX / TILE)));
    const ty1 = Math.max(0, Math.min(th - 1, Math.floor(maxY / TILE)));
    if (tx1 < tx0 || ty1 < ty0 || (tx1 - tx0 + 1) * (ty1 - ty0 + 1) > PAGE_REGION_LIMIT) return null;
    return [tx0, ty0, tx1, ty1];
  }

  private containsProtected(region: [number, number, number, number], x: number, y: number): boolean {
    return protectedTileRegionContains(
      region,
      Math.floor(x / TILE),
      Math.floor(y / TILE),
      this.engine._paint_get_tiles_width(),
      this.engine._paint_get_tiles_height(),
      PAGE_GUARD_TILES,
    );
  }

  canSample(x: number, y: number): boolean {
    return this.coldMask === 0 || this.prefetchRegion === null ||
      this.containsProtected(this.prefetchRegion, x, y);
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

  prepareStrokePoint(
    x: number,
    y: number,
    time: number,
    previousX = x,
    previousY = y,
    previousTime = time,
  ): Promise<boolean> | null {
    if (!this.engine) return null;
    const coldPoint = this.coldMask !== 0 && (!this.prefetchRegion ||
      !this.containsProtected(this.prefetchRegion, x, y) ||
      !this.containsProtected(this.prefetchRegion, previousX, previousY));
    if (!coldPoint && this.growForHeadroom()) return null;
    const limit = this.engine._paint_get_resident_tile_limit();
    if (!coldPoint && this.totalUsed() + PAGE_ALLOCATION_HEADROOM < limit) return null;
    const dx = x - previousX;
    const dy = y - previousY;
    const distance = Math.hypot(dx, dy);
    if (distance > 0) {
      this.directionX = dx / distance;
      this.directionY = dy / distance;
    }
    this.focusX = x;
    this.focusY = y;
    const predictedDistance = Math.min(
      PAGE_PREDICTION_MAX,
      distance * PAGE_PREDICTION_MS / Math.max(1, time - previousTime),
    );
    const predictedX = x + this.directionX * predictedDistance;
    const predictedY = y + this.directionY * predictedDistance;
    const region = this.bounds(predictedX, predictedY, previousX, previousY) ?? this.bounds(x, y);
    if (!region) return Promise.resolve(false);
    return this.prepareAsync(region, ++this.requestGeneration);
  }

  async prepareHistory(maximumTiles: number): Promise<boolean> {
    if (!this.engine) return false;
    try {
      let limit = this.engine._paint_get_resident_tile_limit();
      while (limit - this.totalUsed() < maximumTiles && limit < this.maximumTiles) {
        const next = Math.min(this.maximumTiles, limit + this.growTiles);
        if (!this.engine._paint_set_resident_tile_limit(next)) return false;
        limit = next;
      }
      if (limit - this.totalUsed() >= maximumTiles) return true;
      if (!this.store) return false;
      const generation = ++this.requestGeneration;
      const target = Math.max(0, limit - maximumTiles);
      const noProtection: [number, number, number, number] = [1, 1, 0, 0];
      while (this.totalUsed() > target) {
        if (!(await this.pageBatch(noProtection, target, generation))) return false;
        if (generation !== this.requestGeneration) return true;
      }
      return true;
    } catch (error) {
      post({ type: 'log', text: `IndexedDB history paging failed: ${(error as Error)?.message ?? error}` });
      return false;
    }
  }

  prepare(x: number, y: number): Promise<boolean> | null {
    if (!this.engine) return null;
    this.focusX = x;
    this.focusY = y;
    this.directionX = 0;
    this.directionY = 0;
    const coldPoint = this.coldMask !== 0 && (!this.prefetchRegion ||
      !this.containsProtected(this.prefetchRegion, x, y));
    if (!coldPoint && this.growForHeadroom()) return null;
    const limit = this.engine._paint_get_resident_tile_limit();
    if (!coldPoint && this.totalUsed() + PAGE_ALLOCATION_HEADROOM < limit) return null;
    const region = this.bounds(x, y);
    if (!region) return Promise.resolve(false);
    return this.prepareAsync(region, ++this.requestGeneration);
  }

  private async prepareAsync(
    region: [number, number, number, number],
    generation: number,
  ): Promise<boolean> {
    if (!this.store) {
      post({ type: 'log', text: 'IndexedDB is necessary after the paint memory limit is full.' });
      return false;
    }
    try {
      const limit = this.engine._paint_get_resident_tile_limit();
      const target = Math.max(0, limit - this.growTiles);
      while (this.totalUsed() + PAGE_ALLOCATION_HEADROOM >= limit && this.totalUsed() > target) {
        if (!(await this.pageBatch(region, target, generation))) return false;
        if (generation !== this.requestGeneration) return true;
      }
      const loaded = await this.loadRegion(region, true, generation);
      if (generation !== this.requestGeneration) return true;
      if (loaded) this.prefetchRegion = region;
      return loaded;
    } catch (error) {
      post({ type: 'log', text: `IndexedDB tile paging failed: ${(error as Error)?.message ?? error}` });
      return false;
    }
  }

  private findVictims(
    region: [number, number, number, number],
    maximum: number,
  ): PaintEvictionRank[] {
    const [tx0, ty0, tx1, ty1] = region;
    const result: PaintEvictionRank[] = [];
    const count = this.engine._paint_get_layer_count();
    for (let layer = 0; layer < count; layer++) {
      const used = this.engine._paint_get_layer_used_tile_count(layer);
      for (let index = 0; index < used; index++) {
        this.engine._paint_get_layer_used_tile_info(layer, index, this.infoPtr);
        const slot = this.infoPtr >> 2;
        const tx = this.engine.HEAP32[slot], ty = this.engine.HEAP32[slot + 1];
        if (tx >= tx0 && tx <= tx1 && ty >= ty0 && ty <= ty1) continue;
        if (this.engine._paint_layer_tile_is_captured(layer, tx, ty)) continue;
        const x = tx * TILE + TILE * 0.5 - this.focusX;
        const y = ty * TILE + TILE * 0.5 - this.focusY;
        const projection = x * this.directionX + y * this.directionY;
        const candidate: PaintEvictionRank = [
          layer, index, tx, ty, projection < 0 ? 1 : 0, projection, x * x + y * y,
        ];
        let position: number;
        if (result.length < maximum) {
          result.push(candidate);
          position = result.length - 1;
        } else {
          position = result.length - 1;
          if (!paintEvictionRankBefore(candidate, result[position])) continue;
          result[position] = candidate;
        }
        while (position > 0 && paintEvictionRankBefore(result[position], result[position - 1])) {
          const swap = result[position - 1];
          result[position - 1] = result[position];
          result[position] = swap;
          position--;
        }
      }
    }
    return result;
  }

  private async pageBatch(
    region: [number, number, number, number],
    target: number,
    generation: number,
  ): Promise<boolean> {
    const candidates: { layer: number; tx: number; ty: number }[] = [];
    const writes: PaintTileRecord[] = [];
    const victims = this.findVictims(
      region,
      Math.min(PAGE_WRITE_BATCH, this.totalUsed() - target),
    );
    for (const [layer, index, tx, ty] of victims) {
      if (this.engine._paint_get_layer_used_tile_is_storage_dirty(layer, index)) {
        const ptr = this.engine._paint_get_layer_tile_ptr(layer, tx, ty);
        if (!ptr) break;
        const bytes = new Uint8Array(TILE16_B);
        bytes.set(this.engine.HEAPU8.subarray(ptr, ptr + TILE16_B));
        writes.push({ key: paintTileKey(this.documentId, layer, tx, ty), data: bytes.buffer });
      }
      candidates.push({ layer, tx, ty });
    }
    if (candidates.length === 0) {
      post({ type: 'log', text: 'IndexedDB found no evictable resident tile in the brush area.' });
      return false;
    }
    if (writes.length > 0) await this.store!.putMany(writes);
    if (generation !== this.requestGeneration) return true;
    for (const candidate of candidates) {
      if (!this.engine._paint_remove_layer_tile(candidate.layer, candidate.tx, candidate.ty)) {
        post({ type: 'log', text: `IndexedDB could not evict tile (${candidate.tx}, ${candidate.ty}) from layer ${candidate.layer}.` });
        return false;
      }
      this.coldMask |= 1 << candidate.layer;
    }
    return true;
  }

  private async pageOne(
    region: [number, number, number, number],
    generation: number,
  ): Promise<boolean> {
    const victim = this.findVictims(region, 1)[0];
    if (!victim) {
      post({ type: 'log', text: 'IndexedDB found no evictable resident tile in the brush area.' });
      return false;
    }
    const [layer, index, tx, ty] = victim;
    if (this.engine._paint_get_layer_used_tile_is_storage_dirty(layer, index)) {
      const ptr = this.engine._paint_get_layer_tile_ptr(layer, tx, ty);
      if (!ptr) return false;
      const bytes = new Uint8Array(TILE16_B);
      bytes.set(this.engine.HEAPU8.subarray(ptr, ptr + TILE16_B));
      await this.store!.put(paintTileKey(this.documentId, layer, tx, ty), bytes.buffer);
    }
    if (generation !== this.requestGeneration) return true;
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
    generation: number,
  ): Promise<boolean> {
    const [tx0, ty0, tx1, ty1] = region;
    const layerCount = this.engine._paint_get_layer_count();
    for (let layer = 0; layer < layerCount; layer++) {
      if ((this.coldMask & (1 << layer)) === 0) continue;
      const records = await this.store!.getRegion(
        this.documentId, layer, tx0, ty0, tx1, ty1,
      );
      if (generation !== this.requestGeneration) return true;
      let needed = 0;
      for (const record of records) {
        if (!this.engine._paint_get_layer_tile_ptr(layer, record.key[2], record.key[3])) needed++;
      }
      const limit = this.engine._paint_get_resident_tile_limit();
      if (needed > limit) return false;
      const target = limit - needed;
      while (allowEvict && this.totalUsed() > target) {
        if (!(await this.pageBatch(region, target, generation))) return false;
        if (generation !== this.requestGeneration) return true;
      }
      for (const record of records) {
        const tx = record.key[2], ty = record.key[3];
        if (this.engine._paint_get_layer_tile_ptr(layer, tx, ty)) continue;
        if (record.data.byteLength !== TILE16_B) {
          throw new Error(`Invalid tile size at (${tx}, ${ty}).`);
        }
        while (this.totalUsed() >= limit) {
          if (!allowEvict || !(await this.pageOne(region, generation))) return false;
          if (generation !== this.requestGeneration) return true;
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

const HISTORY_WRITE_BATCH = 64;

class PaintDiskHistory {
  private engine: any = null;
  private store: PaintTileStore | null = null;
  private pager: PaintTilePager | null = null;
  private documentId = '';
  private queue = new PaintHistoryQueue();
  private infoPtr = 0;
  private scratchPtr = 0;
  private activeTiles = 0;
  private readonly zeroTile = new ArrayBuffer(TILE16_B);

  configure(
    engine: any,
    store: PaintTileStore | null,
    pager: PaintTilePager,
    documentId: string,
  ): boolean {
    this.engine = engine;
    this.store = store;
    this.pager = pager;
    this.documentId = documentId;
    this.queue = new PaintHistoryQueue();
    this.activeTiles = 0;
    if (!store) return false;
    if (!this.infoPtr) this.infoPtr = engine._malloc(12);
    if (!this.scratchPtr) this.scratchPtr = engine._malloc(TILE16_B);
    return this.infoPtr !== 0 && this.scratchPtr !== 0;
  }

  canUndo(): boolean { return this.queue.canUndo(); }
  canRedo(): boolean { return this.queue.canRedo(); }

  private async deleteOperations(operations: readonly PaintHistoryOperation[]): Promise<void> {
    for (const operation of operations) {
      await this.store!.deleteHistorySide(this.documentId, operation.id);
    }
  }

  async begin(layer: number): Promise<boolean> {
    if (!this.store) return false;
    const { discarded } = this.queue.begin(layer);
    await this.deleteOperations(discarded);
    this.activeTiles = 0;
    return true;
  }

  private async writeWithEviction(
    records: readonly PaintHistoryRecord[],
    protectedOperation = -1,
  ): Promise<void> {
    for (;;) {
      try {
        await this.store!.putHistoryMany(records);
        return;
      } catch (error) {
        if ((error as Error)?.name !== 'QuotaExceededError') throw error;
        const oldest = this.queue.removeOldest(protectedOperation);
        if (!oldest) throw error;
        await this.store!.deleteHistorySide(this.documentId, oldest.id);
      }
    }
  }

  async flush(force: boolean): Promise<boolean> {
    const operation = this.queue.activeOperation();
    if (!operation || !this.store) return false;
    const count = this.engine._paint_external_history_capture_count();
    if (this.engine._paint_get_error_code() === 2) {
      throw new Error('Paint history capture failed.');
    }
    if (count === 0 || (!force && count < HISTORY_WRITE_BATCH)) return true;
    for (let base = 0; base < count; base += HISTORY_WRITE_BATCH) {
      const records: PaintHistoryRecord[] = [];
      const end = Math.min(count, base + HISTORY_WRITE_BATCH);
      for (let index = base; index < end; index++) {
        this.engine._paint_external_history_capture_info(index, this.infoPtr);
        const info = this.infoPtr >> 2;
        const layer = this.engine.HEAP32[info];
        const tx = this.engine.HEAP32[info + 1];
        const ty = this.engine.HEAP32[info + 2];
        const pointer = this.engine._paint_external_history_capture_ptr(index);
        if (!pointer) throw new Error('A paint history capture has no data.');
        const bytes = new Uint8Array(TILE16_B);
        bytes.set(this.engine.HEAPU8.subarray(pointer, pointer + TILE16_B));
        records.push({
          key: [this.documentId, operation.id, 0, layer, tx, ty],
          data: bytes.buffer,
        });
      }
      await this.writeWithEviction(records);
    }
    this.activeTiles += count;
    this.engine._paint_external_history_clear_captures();
    return true;
  }

  async commit(): Promise<boolean> {
    this.engine._paint_external_history_finish();
    if (!(await this.flush(true))) return false;
    const discarded = this.queue.commit(this.activeTiles);
    this.activeTiles = 0;
    await this.deleteOperations(discarded);
    return true;
  }

  async cancel(): Promise<void> {
    this.engine?._paint_external_history_cancel();
    const operation = this.queue.cancel();
    this.activeTiles = 0;
    if (operation && this.store) {
      await this.store.deleteHistorySide(this.documentId, operation.id);
    }
  }

  async rollbackActive(): Promise<void> {
    const operation = this.queue.activeOperation();
    if (!operation || !this.store || !this.pager) {
      await this.cancel();
      return;
    }
    this.engine._paint_external_history_finish();
    await this.restoreSide(operation, 0);
    const count = this.engine._paint_external_history_capture_count();
    for (let base = 0; base < count; base += HISTORY_WRITE_BATCH) {
      const end = Math.min(count, base + HISTORY_WRITE_BATCH);
      let needed = 0;
      for (let index = base; index < end; index++) {
        this.engine._paint_external_history_capture_info(index, this.infoPtr);
        const info = this.infoPtr >> 2;
        if (!this.engine._paint_get_layer_tile_ptr(
          this.engine.HEAP32[info], this.engine.HEAP32[info + 1], this.engine.HEAP32[info + 2],
        )) needed++;
      }
      if (needed > 0 && !(await this.pager.prepareHistory(needed))) {
        throw new Error('IndexedDB could not make room to cancel a paint operation.');
      }
      for (let index = base; index < end; index++) {
        this.engine._paint_external_history_capture_info(index, this.infoPtr);
        const info = this.infoPtr >> 2;
        const pointer = this.engine._paint_external_history_capture_ptr(index);
        if (!pointer) throw new Error('A paint history capture has no data.');
        this.engine.HEAPU8.set(
          this.engine.HEAPU8.subarray(pointer, pointer + TILE16_B), this.scratchPtr,
        );
        if (!this.engine._paint_write_layer_rgba16_tile_modified(
          this.engine.HEAP32[info], this.engine.HEAP32[info + 1],
          this.engine.HEAP32[info + 2], this.scratchPtr,
        )) {
          throw new Error('A paint history rollback failed.');
        }
      }
    }
    await this.cancel();
    if (this.engine._paint_get_error_code() === 2) this.engine._paint_clear_error();
  }

  async clear(): Promise<void> {
    const active = this.queue.cancel();
    const operations = this.queue.reset();
    this.engine?._paint_external_history_cancel();
    this.activeTiles = 0;
    if (active) operations.push(active);
    if (this.store && operations.length > 0) {
      await this.store.deleteHistoryDocument(this.documentId);
    }
  }

  private async captureCurrent(
    operation: PaintHistoryOperation,
    records: readonly PaintHistoryRecord[],
    side: 0 | 1,
  ): Promise<void> {
    const swap: PaintHistoryRecord[] = new Array(records.length);
    const coldKeys: PaintTileKey[] = [];
    const coldIndexes: number[] = [];
    for (let index = 0; index < records.length; index++) {
      const target = records[index];
      const layer = target.key[3], tx = target.key[4], ty = target.key[5];
      const pointer = this.engine._paint_get_layer_tile_ptr(layer, tx, ty);
      let data: ArrayBuffer;
      if (pointer) {
        const bytes = new Uint8Array(TILE16_B);
        bytes.set(this.engine.HEAPU8.subarray(pointer, pointer + TILE16_B));
        data = bytes.buffer;
      } else {
        data = this.zeroTile;
        coldIndexes.push(index);
        coldKeys.push(paintTileKey(this.documentId, layer, tx, ty));
      }
      swap[index] = {
        key: [this.documentId, operation.id, side, layer, tx, ty],
        data,
      };
    }
    const cold = await this.store!.getMany(coldKeys);
    for (let index = 0; index < coldIndexes.length; index++) {
      swap[coldIndexes[index]].data = cold[index]?.data ?? this.zeroTile;
    }
    await this.writeWithEviction(swap, operation.id);
  }

  private async restoreSide(operation: PaintHistoryOperation, side: 0 | 1): Promise<number> {
    let after: PaintHistoryKey | null = null;
    let restored = 0;
    for (;;) {
      const records = await this.store!.getHistoryBatch(
        this.documentId, operation.id, side, after, HISTORY_WRITE_BATCH,
      );
      if (records.length === 0) return restored;
      let needed = 0;
      for (const record of records) {
        if (!this.engine._paint_get_layer_tile_ptr(record.key[3], record.key[4], record.key[5])) {
          needed++;
        }
      }
      if (needed > 0 && !(await this.pager!.prepareHistory(needed))) {
        throw new Error('IndexedDB could not make room for a history tile.');
      }
      for (const record of records) {
        if (record.data.byteLength !== TILE16_B) {
          throw new Error('A paint history tile has an incorrect size.');
        }
        this.engine.HEAPU8.set(new Uint8Array(record.data), this.scratchPtr);
        if (!this.engine._paint_write_layer_rgba16_tile_modified(
          record.key[3], record.key[4], record.key[5], this.scratchPtr,
        )) {
          throw new Error('A paint history tile restore failed.');
        }
      }
      restored += records.length;
      after = records[records.length - 1].key;
    }
  }

  async toggle(redo: boolean): Promise<boolean> {
    if (!this.store || !this.pager) return false;
    const operation = redo ? this.queue.redoTarget() : this.queue.undoTarget();
    if (!operation) return false;
    const source = operation.side;
    const opposite = (source === 0 ? 1 : 0) as 0 | 1;
    await this.store.deleteHistorySide(this.documentId, operation.id, opposite);

    try {
      let after: PaintHistoryKey | null = null;
      let captured = 0;
      for (;;) {
        const records = await this.store.getHistoryBatch(
          this.documentId, operation.id, source, after, HISTORY_WRITE_BATCH,
        );
        if (records.length === 0) break;
        await this.captureCurrent(operation, records, opposite);
        captured += records.length;
        after = records[records.length - 1].key;
      }
      if (captured !== operation.tiles) {
        throw new Error('A paint history operation is incomplete.');
      }
    } catch (error) {
      try { await this.store.deleteHistorySide(this.documentId, operation.id, opposite); }
      catch {}
      throw error;
    }

    try {
      const restored = await this.restoreSide(operation, source);
      if (restored !== operation.tiles) {
        throw new Error('A paint history operation is incomplete.');
      }
    } catch (error) {
      await this.restoreSide(operation, opposite);
      await this.store.deleteHistorySide(this.documentId, operation.id, opposite);
      throw error;
    }
    operation.side = opposite;
    this.engine._paint_set_active_layer(operation.layer);
    if (redo) this.queue.completeRedo();
    else this.queue.completeUndo();
    await this.store.deleteHistorySide(this.documentId, operation.id, source);
    return true;
  }
}

function runDrainWake() {
  if (!mod || batchInFlight || drainInFlight || strokePreparing || motionQueue.length === 0) return;
  startDrain(true);
}
const drainWake = new FixedTaskWake(runDrainWake);
let tilePool: TilePool | null = null;

const INLINE_JOB_ROWS = 4;
async function drainInlineJobs(count: number): Promise<void> {
  let job = 0;
  while (job < count) {
    const start = performance.now();
    do {
      if (mod._paint_process_tile_job_work(job, 0, INLINE_JOB_ROWS)) job++;
    } while (job < count && performance.now() - start < BUDGET);
    if (job < count) await new Promise<void>(resolve => self.setTimeout(resolve, 0));
  }
}

/// Drain one published group of blend jobs.
async function drainPreparedJobs(count: number): Promise<void> {
  const opSize = mod._paint_draw_dab_op_size();
  if (tilePool?.ready()) {
    const jobs: Promise<Uint8Array>[] = [];
    const info = new Int32Array(mod.memory.buffer, jobInfoPtr, 5);
    const opArenaPtr = mod._paint_ops_arena_ptr();
    const heap = new Uint8Array(mod.memory.buffer);
    for (let index = 0; index < count; index++) {
      mod._paint_get_job_info(index, jobInfoPtr);
      const opsOff = info[0], opCount = info[1], tileAddr = info[2], tx = info[3], ty = info[4];
      const opsStart = opArenaPtr + opsOff * opSize;
      const ops = heap.slice(opsStart, opsStart + opCount * opSize);
      const tile = heap.slice(tileAddr, tileAddr + TILE16_B);
      jobs.push(tilePool.blend({ id: index, tx, ty, opCount, ops, tile }));
    }
    try {
      const blended = await Promise.all(jobs);
      const resultHeap = new Uint8Array(mod.memory.buffer);
      for (let index = 0; index < count; index++) {
        mod._paint_get_job_info(index, jobInfoPtr);
        resultHeap.set(blended[index], info[2]);
      }
      return;
    } catch (error) {
      post({ type: 'log', text: `tile pool fallback: ${(error as Error)?.message ?? error}` });
    }
  }
  await drainInlineJobs(count);
}

/// Drain all fixed-size groups. This leaves no operation for a later stroke.
async function drainBlendJobs(): Promise<number> {
  let total = 0;
  for (;;) {
    const count = mod._paint_end_batch_parallel();
    if (count <= 0) return total;
    await drainPreparedJobs(count);
    total += count;
  }
}

const post = (m: any, t?: Transferable[]) => { if (t && t.length > 0) (self as unknown as Worker).postMessage(m, t); else (self as unknown as Worker).postMessage(m); };

function reportEngineError(what: string, status = 'Brush engine error. See the log.') {
  post({ type: 'log', text: 'ENGINE ERROR: ' + what });
  post({ type: 'status', text: status });
}
function finishStorageFailure(rollback = false) {
  if (!mod) return;
  motionQueue.clear();
  strokeContinuation = false;
  hasStrokePoint = false;
  commitP = false;
  mod._paint_cancel_stroke();
  if (batching) {
    batching = false;
    try { mod._paint_end_batch(); } catch {}
  }
  strokeOpen = false;
  storageInFlight = true;
  reportEngineError('IndexedDB tile storage could not make room.', 'Paint storage is full.');
  let restored = rollback;
  const settle = async () => {
    if (rollback) {
      await diskHistory?.rollbackActive();
      return;
    }
    try {
      await diskHistory?.commit();
    } catch {
      restored = true;
      await diskHistory?.rollbackActive();
    }
  };
  void settle().then(() => {
    if (restored) renderDirty(true);
  }).catch(error => {
    reportEngineError(`Paint history recovery failed: ${(error as Error)?.message ?? error}`);
  }).finally(() => {
    storageInFlight = false;
    pushState();
    replayDeferredCommands();
  });
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
  else if (c === 2) reportEngineError('Paint history capture allocation failed.', 'Paint history storage failed.');
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
  lastStrokeX = x;
  lastStrokeY = y;
  hasStrokePoint = true;
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
    residentTiles: mod?._paint_get_resident_tile_count() ?? 0,
    residentTileLimit: mod?._paint_get_resident_tile_limit() ?? 0,
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
  const now = performance.now();
  if (motionQueue.length === 0 || now - lastDisplay >= DISPLAY_INTERVAL_MS) {
    const renderStart = now;
    renderDirty();
    lastRR = performance.now() - renderStart;
    lastDisplay = performance.now();
  }
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

function startDrain(bounded: boolean) {
  if (!mod || batchInFlight || drainInFlight || strokePreparing || motionQueue.length === 0) return;
  const wait = strokeOpen && tilePager
    ? tilePager.prepareStrokePoint(
      motionQueue.peekX(), motionQueue.peekY(), motionQueue.peekTime(),
      hasStrokePoint ? lastStrokeX : motionQueue.peekX(),
      hasStrokePoint ? lastStrokeY : motionQueue.peekY(),
      lastT > 0 ? lastT : motionQueue.peekTime(),
    )
    : null;
  if (!wait) {
    drainProcess(bounded);
    return;
  }
  const renderStart = performance.now();
  renderDirty();
  lastRR = performance.now() - renderStart;
  lastDisplay = performance.now();
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
    void drainBlendJobs().then(async () => {
      mod._paint_end_batch_finish();
      if (!diskHistory || !(await diskHistory.flush(false))) {
        throw new Error('Paint history storage is not available.');
      }
      batchInFlight = false;
      afterBatch();
    }).catch((err) => {
      reportEngineError('Batch drain failed: ' + ((err as Error)?.stack ?? err));
      batchInFlight = false;
      finishStorageFailure(true);
    });
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
  batchInFlight = true;
  const finish = async () => {
    if (batching) {
      batching = false;
      await drainBlendJobs();
      mod._paint_end_batch_finish();
    }
    if (!diskHistory || !(await diskHistory.commit())) {
      throw new Error('Paint history storage is not available.');
    }
  };
  void finish().catch(async (error) => {
    reportEngineError('Commit failed: ' + ((error as Error)?.message ?? error));
    try {
      await diskHistory?.rollbackActive();
      renderDirty(true);
    } catch (recoveryError) {
      reportEngineError(`Paint history recovery failed: ${(recoveryError as Error)?.message ?? recoveryError}`);
    }
  }).finally(() => {
    batchInFlight = false;
    strokeOpen = false;
    pushState();
    replayDeferredCommands();
  });
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
      hasStrokePoint = false;
      strokeOpen = false;
      reportEngineError('IndexedDB tile storage could not make room.', 'Paint storage is full.');
      pushState();
      replayDeferredCommands();
      return;
    }
    if (!diskHistory || !(await diskHistory.begin(mod._paint_get_active_layer()))) {
      throw new Error('Paint history storage is not available.');
    }
    mod._begin_stroke(x, y, xt, yt, z, r, ba);
    lastStrokeX = x;
    lastStrokeY = y;
    hasStrokePoint = true;
    mod._paint_begin_batch();
    batching = true;
    scheduleFlush();
  } catch (error) {
    motionQueue.clear();
    commitP = false;
    hasStrokePoint = false;
    strokeOpen = false;
    await diskHistory?.cancel();
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
  post({ type: 'state', state: { layers, groups, activeLayer: active, canUndo: diskHistory?.canUndo() ?? false, canRedo: diskHistory?.canRedo() ?? false, width: docW, height: docH, displayScale: dispScale, mipLevel: dispMip, tilesWidth: mod._paint_get_tiles_width(), tilesHeight: mod._paint_get_tiles_height(), residentTiles: mod._paint_get_resident_tile_count(), residentTileLimit: mod._paint_get_resident_tile_limit(), maximumResidentTiles: mod._paint_get_maximum_resident_tile_limit(), error: mod._paint_get_error_code() } });
}
async function changeHistory(redo: boolean) {
  if (!mod || storageInFlight) return;
  storageInFlight = true;
  try {
    renderDirty();
    mod._reset_brush();
    const changed = await diskHistory?.toggle(redo);
    if (changed) renderDirty(true);
    pushState();
  } catch (error) {
    reportEngineError(`History change failed: ${(error as Error)?.message ?? error}`);
  } finally {
    storageInFlight = false;
    replayDeferredCommands();
  }
}

async function clearPaint() {
  if (!mod || storageInFlight) return;
  storageInFlight = true;
  try {
    const layer = mod._paint_get_active_layer();
    await diskHistory?.clear();
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
    await diskHistory?.clear();
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
    case 'undo': flushNow(); void changeHistory(false); break;
    case 'redo': flushNow(); void changeHistory(true); break;
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
    case 'pickColor': {
      const px = Math.max(0, Math.min((canvas?.width ?? 1) - 1, Math.floor(m.x / dispScale)));
      const py = Math.max(0, Math.min((canvas?.height ?? 1) - 1, Math.floor(m.y / dispScale)));
      const color = ctx?.getImageData(px, py, 1, 1).data;
      if (color) post({ type: 'colorPicked', id: m.id, r: color[0], g: color[1], b: color[2] });
      break;
    }
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
      const logicalProcessors = m.hardwareConcurrency ?? 8;
      const workerCount = paintTileWorkerCount(logicalProcessors);
      const memory = paintMemoryPolicy(
        paintMemoryLimitMiB(undefined, m.memoryLimitMiB),
        workerCount,
      );
      if (memory.maximumTiles === 0) {
        reportEngineError('The selected paint memory is too small for the fixed reserve.', 'Increase the paint memory limit.');
        return;
      }
      if (!mod) {
        mod = await loadBrushModule();
        void spawnTilePool(
          mod._paint_draw_dab_op_size(),
          (t) => post({ type: 'log', text: t }),
          logicalProcessors,
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
      if (tileStore && tileDocumentId) {
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
      if (!mod._paint_init_with_tile_limits(
        docW,
        docH,
        memory.initialTiles,
        memory.maximumTiles,
      )) {
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
      tilePager.configure(mod, tileStore, tileDocumentId, memory.maximumTiles, memory.growTiles);
      if (!diskHistory) diskHistory = new PaintDiskHistory();
      if (!diskHistory.configure(mod, tileStore, tilePager, tileDocumentId)) {
        reportEngineError('IndexedDB history is not available.', 'Paint history is not available.');
        return;
      }
      mod._paint_set_external_history(1);
      void navigator.storage?.persist?.().catch(() => {});
      post({ type: 'log', text: `paint memory ${memory.totalMiB} MiB, ${memory.maximumTiles} tile maximum` });
      mod._paint_clear();
      mod._paint_set_background_color(bgRGB[0], bgRGB[1], bgRGB[2]);
      motionQueue.clear();
      deferredCommands.clear();
      strokeContinuation = false;
      hasStrokePoint = false;
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
