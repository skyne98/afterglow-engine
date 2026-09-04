import { describe, expect, test } from 'bun:test';
import { resolvePaintShortcut } from './paint-shortcuts.ts';

const key = (code: string, value = code.replace(/^Key/, '').replace(/^Digit/, '')) => ({ code, key: value });

describe('Photoshop paint shortcuts', () => {
  test('maps tools and brush controls', () => {
    expect(resolvePaintShortcut(key('KeyB'))).toBe('brush-tool');
    expect(resolvePaintShortcut(key('KeyH'))).toBe('hand-tool');
    expect(resolvePaintShortcut(key('KeyI'))).toBe('eyedropper-tool');
    expect(resolvePaintShortcut(key('KeyR'))).toBe('rotate-tool');
    expect(resolvePaintShortcut(key('KeyZ'))).toBe('zoom-tool');
    expect(resolvePaintShortcut(key('BracketLeft', '['))).toBe('brush-smaller');
    expect(resolvePaintShortcut(key('BracketRight', ']'))).toBe('brush-larger');
    expect(resolvePaintShortcut({ key: '{', code: 'BracketLeft', shiftKey: true })).toBe('hardness-softer');
    expect(resolvePaintShortcut({ key: '}', code: 'BracketRight', shiftKey: true })).toBe('hardness-harder');
    expect(resolvePaintShortcut(key('Digit7'))).toBe('opacity-70');
    expect(resolvePaintShortcut(key('Digit0'))).toBe('opacity-100');
  });

  test('maps document and history commands on Control or Command', () => {
    expect(resolvePaintShortcut({ key: 'z', ctrlKey: true })).toBe('undo');
    expect(resolvePaintShortcut({ key: 'z', metaKey: true, shiftKey: true })).toBe('redo');
    expect(resolvePaintShortcut({ key: 'n', ctrlKey: true })).toBe('new-document');
    expect(resolvePaintShortcut({ key: 'n', ctrlKey: true, shiftKey: true })).toBe('new-layer');
    expect(resolvePaintShortcut({ key: 'o', ctrlKey: true })).toBe('open-document');
    expect(resolvePaintShortcut({ key: 's', metaKey: true })).toBe('save-document');
    expect(resolvePaintShortcut({ key: 'w', ctrlKey: true, altKey: true, shiftKey: true })).toBe('export-png');
  });

  test('maps view, color, brush, and panel commands', () => {
    expect(resolvePaintShortcut({ key: '0', ctrlKey: true })).toBe('fit-view');
    expect(resolvePaintShortcut({ key: '1', metaKey: true })).toBe('actual-pixels');
    expect(resolvePaintShortcut({ key: '+', ctrlKey: true, shiftKey: true })).toBe('zoom-in');
    expect(resolvePaintShortcut({ key: '-', metaKey: true })).toBe('zoom-out');
    expect(resolvePaintShortcut(key('KeyD'))).toBe('default-colors');
    expect(resolvePaintShortcut(key('KeyX'))).toBe('switch-colors');
    expect(resolvePaintShortcut(key('Comma', ','))).toBe('previous-brush');
    expect(resolvePaintShortcut(key('Period', '.'))).toBe('next-brush');
    expect(resolvePaintShortcut({ key: 'Tab', code: 'Tab' })).toBe('toggle-panels');
  });

  test('does not change fields or browser commands', () => {
    expect(resolvePaintShortcut({ ...key('KeyB'), editable: true })).toBeUndefined();
    expect(resolvePaintShortcut({ key: 'b', code: 'KeyB', ctrlKey: true })).toBeUndefined();
    expect(resolvePaintShortcut({ key: 'w', ctrlKey: true })).toBeUndefined();
    expect(resolvePaintShortcut({ key: '[', code: 'BracketLeft', altKey: true })).toBeUndefined();
    expect(resolvePaintShortcut({ key: 'i', code: 'KeyI', altKey: true })).toBeUndefined();
  });
});
