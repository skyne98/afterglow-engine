// Lowest-latency relative pointer input available to authored web engine code.
// Browser-created event objects are unavoidable; this handler allocates nothing.

export type RelativePointerSink = (movementX: number, movementY: number) => void;

export interface RelativePointerStatus {
  eventType: 'pointerrawupdate' | 'mousemove';
  locked: boolean;
  unadjustedMovement: boolean;
}

export interface RelativePointerOptions {
  document?: Document;
  rawEventSupported?: boolean;
}

export class RelativePointerInput {
  private readonly element: HTMLElement;
  private readonly sink: RelativePointerSink;
  private readonly ownerDocument: Document;
  private readonly status: RelativePointerStatus;
  private requestedUnadjustedMovement = false;

  private readonly onMovement = (event: Event): void => {
    if (this.ownerDocument.pointerLockElement !== this.element) return;
    const movement = event as MouseEvent;
    this.sink(movement.movementX, movement.movementY);
  };

  private readonly onPointerLockChange = (): void => {
    this.status.locked = this.ownerDocument.pointerLockElement === this.element;
    this.status.unadjustedMovement = this.status.locked && this.requestedUnadjustedMovement;
    if (!this.status.locked) this.requestedUnadjustedMovement = false;
  };

  private readonly onRawLockAcquired = (): void => {
    this.status.locked = this.ownerDocument.pointerLockElement === this.element;
    this.status.unadjustedMovement = this.status.locked;
  };

  private readonly onRawLockRejected = (): void => {
    this.requestedUnadjustedMovement = false;
    this.requestFallbackLock();
  };

  private readonly onFallbackLockAcquired = (): void => {
    this.status.locked = this.ownerDocument.pointerLockElement === this.element;
    this.status.unadjustedMovement = false;
  };

  private readonly onFallbackLockRejected = (): void => {
    this.status.locked = false;
    this.status.unadjustedMovement = false;
  };

  constructor(element: HTMLElement, sink: RelativePointerSink, options: RelativePointerOptions = {}) {
    this.element = element;
    this.sink = sink;
    this.ownerDocument = options.document ?? element.ownerDocument;
    const view = this.ownerDocument.defaultView;
    const rawEventSupported = options.rawEventSupported ?? (view !== null && 'onpointerrawupdate' in view);
    this.status = {
      eventType: rawEventSupported ? 'pointerrawupdate' : 'mousemove',
      locked: false,
      unadjustedMovement: false,
    };
    this.element.addEventListener(this.status.eventType, this.onMovement, { passive: true });
    this.ownerDocument.addEventListener('pointerlockchange', this.onPointerLockChange, { passive: true });
  }

  requestLock(): void {
    if (this.ownerDocument.pointerLockElement === this.element) return;
    this.requestedUnadjustedMovement = true;
    try {
      const pending = this.element.requestPointerLock({ unadjustedMovement: true });
      // Chromium returns a Promise. The void branch retains compatibility with
      // the original Pointer Lock API without issuing a duplicate request.
      if (pending) void pending.then(this.onRawLockAcquired, this.onRawLockRejected);
    } catch {
      this.requestedUnadjustedMovement = false;
      this.requestFallbackLock();
    }
  }

  private requestFallbackLock(): void {
    this.requestedUnadjustedMovement = false;
    try {
      const pending = this.element.requestPointerLock();
      if (pending) void pending.then(this.onFallbackLockAcquired, this.onFallbackLockRejected);
    } catch {
      this.onFallbackLockRejected();
    }
  }

  getStatus(): Readonly<RelativePointerStatus> {
    return this.status;
  }

  dispose(): void {
    this.element.removeEventListener(this.status.eventType, this.onMovement);
    this.ownerDocument.removeEventListener('pointerlockchange', this.onPointerLockChange);
  }
}
