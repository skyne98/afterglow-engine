import { describe, expect, test } from 'bun:test';
import { PaintPointerState } from './paint-pointer-state.ts';

describe('PaintPointerState', () => {
  test('finishes an open stroke before a view change', () => {
    const state = new PaintPointerState();
    expect(state.beginStroke(4)).toBe(false);
    expect(state.finishForViewChange()).toBe(true);
    expect(state.strokeActive).toBe(false);
    expect(state.strokePointer).toBeNull();
    expect(state.panPointer).toBeNull();
    expect(state.strokeOpen).toBe(false);
    expect(state.finishForViewChange()).toBe(false);
  });

  test('replaces a stale stroke with a new stroke', () => {
    const state = new PaintPointerState();
    state.beginStroke(2);
    expect(state.beginStroke(2)).toBe(true);
    expect(state.strokeActive).toBe(true);
    expect(state.strokePointer).toBe(2);
    expect(state.beginStroke(3)).toBe(true);
    expect(state.strokeActive).toBe(true);
    expect(state.strokePointer).toBe(3);
    expect(state.strokeOpen).toBe(true);
    expect(state.finishStroke(2)).toBe(false);
    expect(state.finishStroke(3)).toBe(true);
  });

  test('finishes a stroke before pan input', () => {
    const state = new PaintPointerState();
    state.beginStroke(5);
    expect(state.beginPan(6)).toBe(true);
    expect(state.strokeActive).toBe(false);
    expect(state.strokePointer).toBeNull();
    expect(state.panPointer).toBe(6);
    expect(state.finishPan(6)).toBe(true);
    expect(state.panPointer).toBeNull();
  });

  test('clears stale pan input before a view change', () => {
    const state = new PaintPointerState();
    expect(state.beginPan(9)).toBe(false);
    expect(state.finishForViewChange()).toBe(false);
    expect(state.panPointer).toBeNull();
  });

  test('finishes a stroke when pointer capture is lost', () => {
    const state = new PaintPointerState();
    state.beginStroke(7);
    expect(state.losePointer(8)).toBe(false);
    expect(state.losePointer(7)).toBe(true);
    expect(state.strokeActive).toBe(false);
    expect(state.strokeOpen).toBe(false);
  });
});
