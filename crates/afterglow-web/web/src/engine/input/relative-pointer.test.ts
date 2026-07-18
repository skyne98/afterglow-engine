import { describe, expect, test } from 'bun:test';
import { RelativePointerInput } from './relative-pointer.ts';

class FakeDocument extends EventTarget {
  pointerLockElement: EventTarget | null = null;
  defaultView = {} as Window;
}

class FakeElement extends EventTarget {
  readonly ownerDocument: FakeDocument;
  requests: Array<PointerLockOptions | undefined> = [];
  rejectRaw = false;
  legacyVoid = false;

  constructor(ownerDocument: FakeDocument) {
    super();
    this.ownerDocument = ownerDocument;
  }

  requestPointerLock(options?: PointerLockOptions): Promise<void> {
    this.requests.push(options);
    if (options?.unadjustedMovement && this.rejectRaw) return Promise.reject(new Error('unsupported'));
    this.ownerDocument.pointerLockElement = this;
    this.ownerDocument.dispatchEvent(new Event('pointerlockchange'));
    if (this.legacyVoid) return undefined as unknown as Promise<void>;
    return Promise.resolve();
  }
}

function movementEvent(type: string, x: number, y: number): Event {
  const event = new Event(type);
  Object.defineProperty(event, 'movementX', { value: x });
  Object.defineProperty(event, 'movementY', { value: y });
  return event;
}

function pointerDown(button: number): Event {
  const event = new Event('pointerdown');
  Object.defineProperty(event, 'button', { value: button });
  return event;
}

describe('RelativePointerInput', () => {
  test('locks from a primary canvas gesture and removes activation on dispose', async () => {
    const document = new FakeDocument();
    const element = new FakeElement(document);
    const input = new RelativePointerInput(
      element as unknown as HTMLElement,
      () => {},
      { document: document as unknown as Document, rawEventSupported: true },
    );

    element.dispatchEvent(pointerDown(2));
    expect(element.requests).toEqual([]);
    element.dispatchEvent(pointerDown(0));
    await Promise.resolve();
    expect(element.requests).toEqual([{ unadjustedMovement: true }]);
    expect(input.getStatus().locked).toBe(true);

    document.pointerLockElement = null;
    input.dispose();
    element.dispatchEvent(pointerDown(0));
    expect(element.requests).toHaveLength(1);
  });

  test('uses raw updates and unadjusted pointer lock when available', async () => {
    const document = new FakeDocument();
    const element = new FakeElement(document);
    let x = 0, y = 0;
    const input = new RelativePointerInput(
      element as unknown as HTMLElement,
      (dx, dy) => { x += dx; y += dy; },
      { document: document as unknown as Document, rawEventSupported: true },
    );

    element.dispatchEvent(movementEvent('pointerrawupdate', 9, -4));
    expect([x, y]).toEqual([0, 0]);
    input.requestLock();
    await Promise.resolve();
    element.dispatchEvent(movementEvent('mousemove', 100, 100));
    element.dispatchEvent(movementEvent('pointerrawupdate', 9, -4));

    expect(element.requests).toEqual([{ unadjustedMovement: true }]);
    expect([x, y]).toEqual([9, -4]);
    expect(input.getStatus()).toEqual({
      eventType: 'pointerrawupdate', locked: true, unadjustedMovement: true,
    });
    input.dispose();
  });

  test('does not duplicate a legacy void-returning lock request', () => {
    const document = new FakeDocument();
    const element = new FakeElement(document);
    element.legacyVoid = true;
    const input = new RelativePointerInput(
      element as unknown as HTMLElement,
      () => {},
      { document: document as unknown as Document, rawEventSupported: true },
    );
    input.requestLock();
    expect(element.requests).toEqual([{ unadjustedMovement: true }]);
    expect(input.getStatus().unadjustedMovement).toBe(true);
  });

  test('falls back to accelerated lock and mousemove deterministically', async () => {
    const document = new FakeDocument();
    const element = new FakeElement(document);
    element.rejectRaw = true;
    let movement = 0;
    const input = new RelativePointerInput(
      element as unknown as HTMLElement,
      dx => { movement += dx; },
      { document: document as unknown as Document, rawEventSupported: false },
    );

    input.requestLock();
    await Promise.resolve();
    await Promise.resolve();
    element.dispatchEvent(movementEvent('mousemove', 3, 0));

    expect(element.requests).toEqual([{ unadjustedMovement: true }, undefined]);
    expect(movement).toBe(3);
    expect(input.getStatus()).toEqual({
      eventType: 'mousemove', locked: true, unadjustedMovement: false,
    });
  });
});
