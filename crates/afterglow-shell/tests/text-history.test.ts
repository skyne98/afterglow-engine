import { test, expect } from 'bun:test';
import { TextHistory } from '../text-history.ts';
const state = value => ({ value, anchor: value.length, focus: value.length });

test('text history rejects invalid capacity limits', () => {
  for (const limit of [0, -1, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
    expect(() => new TextHistory(limit, 100)).toThrow(RangeError);
    expect(() => new TextHistory(10, limit)).toThrow(RangeError);
  }
});

test('text history retains independent controls and removes redo after an edit', () => {
  const history = new TextHistory(2, 4096), a = {}, b = {};
  history.record(a, state(''), state('a'));
  history.record(b, state(''), state('b'));
  history.record(a, state('a'), state('ab'));
  const token = history.peek(a, false);
  expect(token.value.value).toBe('a');
  expect(history.accept(a, token, false)).toBe(true);
  expect(history.peek(a, true).value.value).toBe('ab');
  expect(history.peek(b, false).value.value).toBe('');
  history.record(a, state('a'), state('ac'));
  expect(history.peek(a, true)).toBeNull();
  expect(history.accept(a, token, true)).toBe(false);
  history.clear(a);
  expect(history.peek(a, false)).toBeNull();
  expect(history.peek(b, false)).not.toBeNull();
});

test('text history enforces edit and document byte limits without changing text', () => {
  const history = new TextHistory(2, 550), control = {}, second = {};
  for (let i = 0; i < 1000; i++) {
    history.record(control, state(String(i)), state(String(i + 1)));
    expect(history.bytes).toBeLessThanOrEqual(550);
  }
  let count = 0;
  for (let token; (token = history.peek(control, false)); count++) history.accept(control, token, false);
  expect(count).toBe(2);
  history.record(second, state(''), state('x'));
  expect(history.bytes).toBeLessThanOrEqual(550);
  history.record(second, state('x'), state('x'.repeat(1000)));
  expect(history.peek(second, false)).toBeNull();
  history.clear(control);
  expect(history.bytes).toBe(0);
});

test('text history retains Unicode values and backward selection', () => {
  const history = new TextHistory(), control = {};
  const before = { value: 'a😀é', anchor: 4, focus: 1 };
  history.record(control, before, state('x'));
  expect(history.peek(control, false).value).toEqual(before);
});
