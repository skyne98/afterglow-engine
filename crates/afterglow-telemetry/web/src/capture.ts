import { TelemetryCaptureState, TelemetryRecorder, type TelemetryProducerIdentity } from './telemetry.ts';

/** Cold application adapter. The generated native client uses this same shape. */
export interface CaptureClient {
  start(): Promise<string>;
  ingest(epoch: number, opcode: number, payload: Uint8Array): Promise<number>;
  finish(): Promise<string>;
}
export interface CaptureSource {
  register(epoch: number, session: readonly [number, number, number, number], origin: number, reference: number, uncertainty: number): Uint8Array;
  drain(): Uint8Array;
  stop(): void;
}

/** One fixed window and one stable encoding buffer. Writes use the supplied recorder. */
export class RecorderSource implements CaptureSource {
  private readonly output: Uint8Array;
  private identity: TelemetryProducerIdentity | null = null;
  constructor(readonly id: number, readonly name: string, readonly recorder: TelemetryRecorder) {
    if (!Number.isInteger(id) || id < 1 || id > 0xffff_ffff || name.length > 128 || !name.length ||
        recorder.capacity > 1024 || recorder.descriptors.length > 256 || recorder.ticksPerSecond !== 1_000_000_000)
      throw new RangeError('Invalid profiling source capacity or clock');
    this.output = new Uint8Array(96 + recorder.capacity * 40);
  }
  register(epoch: number, session: readonly [number, number, number, number], origin: number, reference: number, uncertainty: number): Uint8Array {
    this.stop();
    const kinds = ['', 'Instant', 'Span', 'AsyncSpan', 'Flow'];
    const arg = (name?: string) => ({ name: name ?? '', kind: name ? 'Unsigned' : 'None', unit: 'None' });
    const bytes = new TextEncoder().encode(JSON.stringify({ source_id: this.id, producer_generation: 1,
      clock_generation: 1, process_id: 0, name: this.name,
      clock: { clock_domain: this.id, origin_tick: origin, origin_reference_ns: reference,
        rate_numerator: 1, rate_denominator: 1, uncertainty_ns: uncertainty },
      descriptors: this.recorder.descriptors.map(d => ({ category: d.category, category_name: d.categoryName,
        name: d.name, kind: kinds[d.kind], argument0: arg(d.argument0), argument1: arg(d.argument1), severity: 'Trace' })),
      metric_descriptors: [] }));
    if (bytes.length >= 65536) throw new RangeError('Profiling metadata exceeds capacity');
    this.identity = { session, sourceId: this.id, generation: 1, clockDomain: this.id, clockGeneration: 1 };
    if (!this.recorder.arm(epoch)) throw new Error('Profiling recorder is unavailable');
    return bytes;
  }
  drain(): Uint8Array {
    if (!this.identity || !this.recorder.stop()) throw new Error('Profiling recorder is not active');
    const length = this.recorder.encodeBatchInto(this.output, this.identity);
    this.recorder.resume();
    if (length <= 0) throw new Error('Invalid profiling batch');
    return this.output.subarray(0, length === 96 ? 0 : length);
  }
  stop(): void {
    if (this.recorder.state === TelemetryCaptureState.Armed) this.recorder.stop();
    if (this.recorder.state === TelemetryCaptureState.Frozen) this.recorder.reset();
    this.identity = null;
  }
}

/** At most eight sources, one RPC in flight, and one batch per step. No frame queue. */
export class CapturePump {
  private readonly sources: CaptureSource[] = [];
  private busy = false;
  private epoch = 0;
  private registered = 0;
  private cursor = 0;
  connected = false;
  failures = 0;
  batches = 0;
  bytes = 0;
  constructor(private readonly client: CaptureClient, private readonly clock = () => Math.floor(performance.now() * 1_000_000)) {}
  add(source: CaptureSource): boolean {
    if (this.sources.length === 8 || this.sources.includes(source)) return false;
    this.sources.push(source);
    return true;
  }
  private stop(): void {
    for (const source of this.sources) {
      try { source.stop(); } catch { this.failures++; }
    }
    this.epoch = 0;
    this.registered = 0;
    this.cursor = 0;
    this.connected = false;
  }
  async step(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    try {
      const before = this.clock();
      const status = JSON.parse(await this.client.start());
      const after = this.clock();
      if (!status.connected) { if (this.epoch) this.stop(); return; }
      if (!Number.isInteger(status.epoch) || status.epoch < 1 || status.epoch > 0xffff_ffff ||
          !Array.isArray(status.session) || status.session.length !== 4 ||
          !status.session.every((v: number) => Number.isInteger(v) && v >= 0 && v <= 0xffff_ffff) ||
          !status.session.some((v: number) => v !== 0) || status.maxFrameBytes !== 65536)
        throw new Error('Invalid profiling connection');
      const reference = Number(status.nativeTick);
      if (!Number.isSafeInteger(reference) || reference < 0) throw new Error('Invalid profiling clock');
      if (this.epoch !== status.epoch) { this.stop(); this.epoch = status.epoch; }
      this.connected = true;
      // Registration is cold. One source per step bounds work after attachment.
      if (this.registered < this.sources.length) {
        const data = this.sources[this.registered]!.register(this.epoch, status.session,
          after, reference, Math.max(0, after - before));
        if (await this.client.ingest(this.epoch, 1, data) !== 0) { this.stop(); return; }
        this.registered++;
        return;
      }
      if (this.registered) {
        const data = this.sources[this.cursor]!.drain();
        if (data.length > 65535) throw new RangeError('Profiling batch exceeds capacity');
        if (data.length) {
          if (await this.client.ingest(this.epoch, 2, data) !== 0) { this.stop(); return; }
          this.batches++;
          this.bytes += data.length;
        }
        this.cursor = (this.cursor + 1) % this.registered;
      }
    } catch {
      this.failures++;
      this.stop();
      try { await this.client.finish(); } catch { /* The application continues without capture. */ }
    } finally { this.busy = false; }
  }
}
