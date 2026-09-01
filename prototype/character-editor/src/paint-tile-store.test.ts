import { describe, expect, test } from 'bun:test';
import { paintTileKey } from './paint-tile-store.ts';

describe('paintTileKey', () => {
  test('keeps document, layer, and tile coordinates in key order', () => {
    expect(paintTileKey('doc-1', 3, -2, 7)).toEqual(['doc-1', 3, -2, 7]);
  });
});
