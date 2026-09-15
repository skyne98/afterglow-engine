import { PaintClient } from '../../workers/paint.client.ts';
import { NativeRpcTransport, nativeWorkerIds } from '../workers/native-transport.ts';
import { attachProfiling } from '../telemetry/profiling.ts';
import { TelemetryDescriptorKind, TelemetryRecorder } from '../telemetry/telemetry.ts';

const PAINT_TRACE = [
  { category: 0, categoryName: 'paint', name: 'paint.stroke_sample', kind: TelemetryDescriptorKind.Instant, argument0: 'queue_items', argument1: 'queue_bytes' },
  { category: 0, categoryName: 'paint', name: 'paint.publish', kind: TelemetryDescriptorKind.Instant, argument0: 'tiles', argument1: 'rgba_bytes' },
] as const;

/** Native paint owns one OS worker. No browser worker or WASM is permitted. */
export class NativePaint {
  onmessage: ((event: { data: any }) => void) | null = null;
  onerror: ((event: { message: string }) => void) | null = null;
  private readonly client: PaintClient;
  private readonly trace = new TelemetryRecorder(PAINT_TRACE, new ArrayBuffer(1024 * 40));
  private readonly queue: (string | { layer: number; tx: number; ty: number; data: Uint8Array } | undefined)[] = new Array(4096);
  private head = 0;
  private count = 0;
  private queuedBytes = 0;
  private running = false;
  private failed = false;
  private failure: Error | null = null;
  private completion: { resolve: () => void; reject: (error: Error) => void } | null = null;
  private state: any = null;
  private awaitingRecovery = false;
  private dirty: number[] | null = null;
  private readonly context: CanvasRenderingContext2D & { commit?: (x?: number, y?: number, width?: number, height?: number) => void };
  private readonly image = new ImageData(64, 64);

  constructor(private readonly canvas: HTMLCanvasElement) {
    const ids = nativeWorkerIds('paint');
    if (ids.length !== 1) throw new Error('Native paint needs exactly one worker.');
    this.client = new PaintClient(new NativeRpcTransport(ids[0]!));
    const context = canvas.getContext('2d', { alpha: true });
    if (!context) throw new Error('Canvas2D is unavailable.');
    this.context = context;
    attachProfiling('afterglow-engine.paint', this.trace);
  }

  send(message: any): void {
    if (this.failure) throw this.failure;
    if (this.count === this.queue.length) {
      this.fail(new Error('Native paint command queue is full.'));
      return;
    }
    // Capture input now: the pointer code reuses its sample object.
    const entry = message.cmd === 'writeTile'
      ? { layer: message.layer, tx: message.tx, ty: message.ty, data: new Uint8Array(message.data).slice() }
      : JSON.stringify(message);
    if (typeof entry === 'string' ? entry.length > 65536 : entry.data.length !== 16384) {
      this.fail(new Error('Native paint command exceeds its byte limit.'));
      return;
    }
    const bytes = typeof entry === 'string' ? entry.length * 2 : entry.data.byteLength;
    if (this.queuedBytes + bytes > 8 * 1024 * 1024) {
      this.fail(new Error('Native paint queue exceeds 8 MiB.'));
      return;
    }
    this.queuedBytes += bytes;
    this.queue[(this.head + this.count) % this.queue.length] = entry;
    this.count++;
    if (message.cmd === 'strokeSample') this.trace?.instant(0, 0, this.count, this.queuedBytes);
    if (!this.running) void this.drain();
  }

  flush(): Promise<void> {
    if (this.failure) return Promise.reject(this.failure);
    if (!this.running && this.count === 0) return Promise.resolve();
    if (this.completion) return Promise.reject(new Error('A paint flush is already active.'));
    return new Promise((resolve, reject) => { this.completion = { resolve, reject }; });
  }

  private emit(data: any): void { this.onmessage?.({ data }); }
  private fail(error: unknown): void {
    this.failed = true;
    this.queue.fill(undefined);
    this.count = 0;
    this.queuedBytes = 0;
    const failure = error instanceof Error ? error : new Error(String(error));
    this.failure = failure;
    this.completion?.reject(failure);
    this.completion = null;
    this.onerror?.({ message: failure.message });
  }

  private accept(encoded: string): void {
    const result = JSON.parse(encoded);
    // Only the owner can confirm restoration of the last completed document.
    if (result.error && result.documentRolledBack !== true) throw new Error(result.error);
    if (result.recoveryRequired === true) {
      this.state = null;
      this.dirty = null;
      this.awaitingRecovery = true;
      this.emit({ type: 'recoveryRequired' });
      return;
    }
    if (result.error) {
      this.emit({ type: 'status', text: result.error });
      this.emit({ type: 'log', text: result.error });
    }
    if (result.state) { this.state = result.state; this.awaitingRecovery = false; }
    if (result.dirty) {
      const [x, y, w, h] = result.dirty as number[];
      if (this.dirty) {
        const [a, b, c, d] = this.dirty;
        this.dirty = [Math.min(a, x), Math.min(b, y), Math.max(a + c, x + w) - Math.min(a, x), Math.max(b + d, y + h) - Math.min(b, y)];
      } else this.dirty = [x, y, w, h];
    }
  }

  private async drain(): Promise<void> {
    this.running = true;
    try {
      while (this.count && !this.failed) {
        let changed = false;
        for (let batch = 0; batch < 32 && this.count; batch++) {
          const entry = this.queue[this.head]!;
          this.queue[this.head] = undefined;
          this.head = (this.head + 1) % this.queue.length;
          this.count--;
          this.queuedBytes -= typeof entry === 'string' ? entry.length * 2 : entry.data.byteLength;
          if (typeof entry !== 'string') {
            if (this.awaitingRecovery) throw new Error('Select Restore or Discard before a tile write.');
            this.accept(await this.client.writeTile(entry.layer, entry.tx, entry.ty, entry.data));
            changed = true;
            continue;
          }
          const command = JSON.parse(entry);
          if (!this.state && (command.cmd === 'setView' || command.cmd === 'config' || command.cmd === 'loadBrush')) continue;
          if (this.awaitingRecovery && command.cmd !== 'init') throw new Error('Select Restore or Discard before a paint command.');
          if (command.cmd === 'exportTiles') {
            await this.exportTiles(command);
          } else if (command.cmd === 'pickColor' || command.cmd === 'probe') {
            await this.render();
            this.sample(command);
          } else {
            let encoded = entry;
            if (command.cmd === 'strokeSample') {
              const samples = [entry];
              // Three UTF-8 bytes per UTF-16 code unit is a safe upper bound.
              let bytes = entry.length * 3 + 40;
              while (samples.length < 32 - batch && this.count) {
                const next = this.queue[this.head];
                if (typeof next !== 'string' || bytes + next.length * 3 + 1 > 65536 || JSON.parse(next).cmd !== 'strokeSample') break;
                this.queue[this.head] = undefined;
                this.head = (this.head + 1) % this.queue.length;
                this.count--;
                this.queuedBytes -= next.length * 2;
                samples.push(next);
                bytes += next.length * 3 + 1;
              }
              batch += samples.length - 1;
              if (samples.length > 1) encoded = `{"cmd":"strokeBatch","samples":[${samples.join(',')}]}`;
            }
            this.accept(await this.client.command(encoded));
            changed = true;
            if (command.cmd === 'init' && !this.awaitingRecovery) {
              await this.render();
              this.emit({ type: 'state', state: this.state });
              this.emit({ type: 'ready' });
            }
            // Publish each available sample group before more pointer input.
            if (command.cmd === 'strokeSample') break;
          }
        }
        if (changed && !this.awaitingRecovery) {
          await this.render();
          this.emit({ type: 'state', state: this.state });
        }
      }
    } catch (error) { this.fail(error); }
    finally {
      this.running = false;
      this.completion?.resolve();
      this.completion = null;
    }
  }

  private async render(): Promise<void> {
    if (!this.state || !this.dirty) return;
    const scale = this.state.displayScale;
    const width = Math.ceil(this.state.width / scale), height = Math.ceil(this.state.height / scale);
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
      this.dirty = [0, 0, this.state.width, this.state.height];
    }
    const [x, y, w, h] = this.dirty;
    this.dirty = null;
    const stride = 64 * scale;
    const x0 = Math.max(0, Math.floor(x / stride)), y0 = Math.max(0, Math.floor(y / stride));
    const x1 = Math.min(Math.ceil(this.state.width / stride), Math.ceil((x + w) / stride));
    const y1 = Math.min(Math.ceil(this.state.height / stride), Math.ceil((y + h) / stride));
    // One serial RPC per tile costs one host poll per tile. Keep at most
    // eight reads in flight (128 KiB of tile data) on the existing transport.
    const columns = x1 - x0;
    const count = columns * (y1 - y0);
    for (let start = 0; start < count; start += 8) {
      const reads: Promise<Uint8Array>[] = [];
      const end = Math.min(start + 8, count);
      for (let index = start; index < end; index++) {
        reads.push(this.client.tile(-1, x0 + index % columns, y0 + Math.floor(index / columns), scale));
      }
      const tiles = await Promise.all(reads);
      for (let index = start; index < end; index++) {
        const bytes = tiles[index - start];
        if (bytes.length !== this.image.data.length) throw new Error('Invalid native paint tile.');
        this.image.data.set(bytes);
        this.context.putImageData(this.image, (x0 + index % columns) * 64, (y0 + Math.floor(index / columns)) * 64);
      }
    }
    if (count > 0) this.context.commit?.(x0 * 64, y0 * 64,
      Math.min(width, x1 * 64) - x0 * 64, Math.min(height, y1 * 64) - y0 * 64);
    this.trace?.instant(1, 0, count, count * 16384);
  }

  private async exportTiles(command: any): Promise<void> {
    if (!this.state) throw new Error('No paint document.');
    // The editor retains one full RGBA export. Reject it before allocation.
    const cols = Math.ceil(this.state.width / 64), rows = Math.ceil(this.state.height / 64);
    if (cols * rows * 16384 > 256 * 1024 * 1024) throw new Error('Paint export exceeds 256 MiB.');
    const data: ArrayBuffer[] = [];
    for (let ty = 0; ty < rows; ty++) for (let tx = 0; tx < cols; tx++) {
      const bytes = await this.client.tile(command.layerId ?? -1, tx, ty, 1);
      if (bytes.length !== 16384) throw new Error('Invalid native paint export tile.');
      data.push(bytes.slice().buffer as ArrayBuffer);
    }
    this.emit({ type: 'tiles', id: command.id, data, scale: 1 });
  }

  private sample(command: any): void {
    if (!this.state) return;
    const scale = this.state.displayScale;
    if (command.cmd === 'pickColor') {
      const x = Math.floor(command.x / scale), y = Math.floor(command.y / scale);
      if (!Number.isFinite(x) || !Number.isFinite(y) || x < 0 || y < 0 || x >= this.canvas.width || y >= this.canvas.height) return;
      const bytes = this.context.getImageData(x, y, 1, 1).data;
      this.emit({ type: 'colorPicked', id: command.id, r: bytes[0], g: bytes[1], b: bytes[2] });
    } else {
      const y = Math.max(0, Math.min(this.canvas.height - 1, Math.round(command.y * this.canvas.height)));
      const bytes = this.context.getImageData(0, y, this.canvas.width, 1).data;
      let transparent = 0, colored = 0;
      for (let x = 0; x < this.canvas.width; x++) {
        if (bytes[x * 4 + 3] === 0) transparent++;
        if (bytes[x * 4] !== bytes[0] || bytes[x * 4 + 1] !== bytes[1] || bytes[x * 4 + 2] !== bytes[2]) colored++;
      }
      this.emit({ type: 'probeResult', id: command.id, y, transparent, colored, width: this.canvas.width, w: this.canvas.width, alpha0: transparent, painted: colored, native: true,
        rgba: command.pixels === true ? Array.from(bytes) : undefined });
    }
  }
}
