import { describe, expect, test } from 'bun:test';
import { decodeFeedback, encodeFeedback } from './virtual-texture-feedback.ts';

describe('RG32Uint VT feedback encoding', () => {
  test('round trips 256K texture page coordinates', () => {
    const encoded = encodeFeedback(0xfedcba98, 11, 2047, 2047, 7);
    expect(decodeFeedback(...encoded)).toEqual({
      textureId: 0xfedcba98, mip: 11, x: 2047, y: 2047, cameraCloseness: 7,
    });
  });

  test('keeps texture identity separate from coordinates', () => {
    expect(decodeFeedback(...encodeFeedback(1, 3, 9, 17))?.textureId).toBe(1);
    expect(decodeFeedback(...encodeFeedback(2, 3, 9, 17))?.textureId).toBe(2);
  });

  test('recognizes cleared pixels and rejects overflow', () => {
    expect(decodeFeedback(0, 123)).toBeNull();
    expect(() => encodeFeedback(0, 0, 2048, 0)).toThrow();
    expect(() => encodeFeedback(0, 64, 0, 0)).toThrow();
    expect(() => encodeFeedback(0, 0, 0, 0, 8)).toThrow();
  });
});
