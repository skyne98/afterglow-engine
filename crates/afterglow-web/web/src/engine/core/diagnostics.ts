export const enum DiagnosticStatus {
  Recorded = 0,
  CapacityExceeded = 1,
}

export const enum DiagnosticSource {
  Runtime = 0,
  Renderer = 1,
  Worker = 2,
  Asset = 3,
  VirtualTexture = 4,
  Game = 5,
}

export const enum DiagnosticCode {
  Unknown = 0,
  RuntimeState = 1,
  DeviceLost = 2,
  UncapturedGpuError = 3,
  WorkerFailure = 4,
  CapacityExceeded = 5,
  PipelineAfterSeal = 6,
}

export interface DiagnosticRecord {
  sequence: number;
  code: DiagnosticCode;
  source: DiagnosticSource;
  detail: unknown;
}

/** Fixed-capacity diagnostic ring. Full rings drop newest events visibly. */
export class EngineDiagnostics {
  private readonly sequences: Uint32Array;
  private readonly codes: Uint16Array;
  private readonly sources: Uint8Array;
  private readonly details: unknown[];
  private head = 0;
  private nextSequence = 1;
  count = 0;
  highWater = 0;
  dropped = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity <= 0)
      throw new RangeError('diagnostic capacity must be a positive integer');
    this.sequences = new Uint32Array(capacity);
    this.codes = new Uint16Array(capacity);
    this.sources = new Uint8Array(capacity);
    this.details = new Array<unknown>(capacity).fill(null);
  }

  // @hot-no-alloc-begin EngineDiagnostics.tryRecord
  tryRecord(code: DiagnosticCode, source: DiagnosticSource, detail: unknown): DiagnosticStatus {
    if (this.count === this.capacity) {
      this.dropped++;
      return DiagnosticStatus.CapacityExceeded;
    }
    const slot = (this.head + this.count) % this.capacity;
    this.sequences[slot] = this.nextSequence++;
    this.codes[slot] = code;
    this.sources[slot] = source;
    this.details[slot] = detail;
    this.count++;
    if (this.count > this.highWater) this.highWater = this.count;
    return DiagnosticStatus.Recorded;
  }
  // @hot-no-alloc-end EngineDiagnostics.tryRecord

  // @hot-no-alloc-begin EngineDiagnostics.readInto
  readInto(index: number, out: DiagnosticRecord): boolean {
    if (!Number.isInteger(index) || index < 0 || index >= this.count) return false;
    const slot = (this.head + index) % this.capacity;
    out.sequence = this.sequences[slot] ?? 0;
    out.code = (this.codes[slot] ?? 0) as DiagnosticCode;
    out.source = (this.sources[slot] ?? 0) as DiagnosticSource;
    out.detail = this.details[slot];
    return true;
  }
  // @hot-no-alloc-end EngineDiagnostics.readInto

  // @hot-no-alloc-begin EngineDiagnostics.shiftInto
  shiftInto(out: DiagnosticRecord): boolean {
    if (this.count === 0) return false;
    const slot = this.head;
    out.sequence = this.sequences[slot] ?? 0;
    out.code = (this.codes[slot] ?? 0) as DiagnosticCode;
    out.source = (this.sources[slot] ?? 0) as DiagnosticSource;
    out.detail = this.details[slot];
    this.details[slot] = null;
    this.head = (slot + 1) % this.capacity;
    this.count--;
    return true;
  }
  // @hot-no-alloc-end EngineDiagnostics.shiftInto

  clear(): void {
    while (this.count !== 0) {
      this.details[this.head] = null;
      this.head = (this.head + 1) % this.capacity;
      this.count--;
    }
  }
}
