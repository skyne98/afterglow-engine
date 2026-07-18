import {
  DiagnosticCode, DiagnosticSource, EngineDiagnostics,
} from './diagnostics.ts';

type BootstrapCleanup = () => void | Promise<void>;

/** Fixed-capacity reverse rollback for partially completed async bootstrap. */
export class BootstrapGuard {
  private readonly cleanups: Array<BootstrapCleanup | null>;
  private count = 0;
  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity <= 0)
      throw new RangeError('bootstrap cleanup capacity must be positive');
    this.cleanups = new Array<BootstrapCleanup | null>(capacity).fill(null);
  }
  defer(cleanup: BootstrapCleanup): void {
    if (this.count === this.capacity) throw new Error('bootstrap cleanup capacity exceeded');
    this.cleanups[this.count++] = cleanup;
  }
  release(): void {
    for (let index = 0; index < this.count; index++) this.cleanups[index] = null;
    this.count = 0;
  }
  async rollback(): Promise<void> {
    let firstError: unknown = null;
    for (let index = this.count - 1; index >= 0; index--) {
      const cleanup = this.cleanups[index];
      this.cleanups[index] = null;
      try { await cleanup?.(); }
      catch (error) { if (firstError === null) firstError = error; }
    }
    this.count = 0;
    if (firstError !== null) throw firstError;
  }
}

/** Fixed-capacity promises for out-of-band automation; never used by gameplay. */
export class FrameStepHarness {
  private readonly targets: Float64Array;
  private readonly resolvers: Array<(() => void) | null>;
  private count = 0;

  constructor(readonly capacity: number) {
    if (!Number.isInteger(capacity) || capacity <= 0)
      throw new RangeError('frame-step capacity must be positive');
    this.targets = new Float64Array(capacity);
    this.resolvers = new Array<(() => void) | null>(capacity).fill(null);
  }

  wait(currentFrame: number, count = 1): Promise<void> {
    if (!Number.isInteger(count) || count <= 0) throw new RangeError('frame-step count must be positive');
    if (this.count === this.capacity) throw new Error('frame-step capacity exceeded');
    const slot = this.count++;
    this.targets[slot] = currentFrame + count;
    return new Promise<void>((resolve) => { this.resolvers[slot] = resolve; });
  }

  /** @alloc-effect none */
  poll(frame: number): void {
    let write = 0;
    for (let read = 0; read < this.count; read++) {
      const resolver = this.resolvers[read];
      if (frame >= (this.targets[read] ?? Number.POSITIVE_INFINITY)) {
        this.resolvers[read] = null;
        resolver?.(); // @alloc-allowed reason=AutomationPromiseResolution issue=DME-034 expires=2026-10-01
        continue;
      }
      if (write !== read) {
        this.targets[write] = this.targets[read] ?? 0;
        this.resolvers[write] = resolver ?? null;
        this.resolvers[read] = null;
      }
      write++;
    }
    this.count = write;
  }
}

/** Owns global browser error listeners and writes only to bounded diagnostics. */
export class BrowserErrorCapture {
  private readonly record = { sequence: 0, code: DiagnosticCode.Unknown, source: DiagnosticSource.Game, detail: null as unknown };
  private readonly onError: (event: ErrorEvent) => void;
  private readonly onRejection: (event: PromiseRejectionEvent) => void;
  private disposed = false;

  constructor(private readonly diagnostics: EngineDiagnostics, private readonly target: Window = window) {
    this.onError = (event): void => {
      diagnostics.tryRecord(DiagnosticCode.Unknown, DiagnosticSource.Game, event.error ?? event);
    };
    this.onRejection = (event): void => {
      diagnostics.tryRecord(DiagnosticCode.Unknown, DiagnosticSource.Game, event.reason);
    };
    target.addEventListener('error', this.onError);
    target.addEventListener('unhandledrejection', this.onRejection);
  }

  snapshot(): unknown[] {
    const result: unknown[] = [];
    for (let index = 0; index < this.diagnostics.count; index++) {
      if (this.diagnostics.readInto(index, this.record)) result.push(this.record.detail);
    }
    return result;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.target.removeEventListener('error', this.onError);
    this.target.removeEventListener('unhandledrejection', this.onRejection);
  }
}

/** Owns page-teardown listener registration for demo bootstrap resources. */
export class PageShutdown {
  private readonly onUnload: () => void;
  private disposed = false;
  constructor(callback: () => void, private readonly target: Window = window) {
    this.onUnload = (): void => callback();
    target.addEventListener('beforeunload', this.onUnload, { once: true });
  }
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.target.removeEventListener('beforeunload', this.onUnload);
  }
}

export function publishDevHarness<T>(name: string, value: T): void {
  Object.defineProperty(window, name, { configurable: true, value });
}

/** Diagnostic-only DOM writer kept outside visual entrypoint architecture. */
export class TextHud {
  constructor(private readonly element: HTMLElement | null) {}
  setText(text: string): void { if (this.element) this.element.textContent = text; }
  setVisible(visible: boolean): void { if (this.element) this.element.style.display = visible ? '' : 'none'; }
}
