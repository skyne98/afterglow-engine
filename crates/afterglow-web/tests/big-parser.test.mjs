// Tests for the .big format parser (JavaScript side).
// Run: node --test crates/afterglow-web/tests/big-parser.test.mjs

import { test, describe } from 'node:test';
import assert from 'node:assert';
import { readFileSync } from 'node:fs';

// We need to transpile the TS — use bun to build first
import { parseBigHeader, findVTPageChunk, BIG_MAGIC, BIG_VERSION } from '../www/engine/big-parser.js';

describe('BigParser', () => {
  test('BIG_MAGIC is "BIG1" as u32 LE', () => {
    // "BIG1" = B=0x42, I=0x49, G=0x47, 1=0x31 → LE u32 = 0x31474942
    assert.strictEqual(BIG_MAGIC, 0x31474942);
  });

  test('BIG_VERSION is 2', () => {
    assert.strictEqual(BIG_VERSION, 2);
  });

  test('parseBigHeader rejects empty data', () => {
    assert.throws(() => parseBigHeader(new Uint8Array(0)), /file too small/);
  });

  test('parseBigHeader rejects bad magic', () => {
    const data = new Uint8Array(16);
    data[0] = 0x58; // 'X'
    assert.throws(() => parseBigHeader(data), /bad magic/);
  });

  test('parseBigHeader rejects wrong version', () => {
    const data = new Uint8Array(16);
    const view = new DataView(data.buffer);
    view.setUint32(0, BIG_MAGIC, true);
    view.setUint32(4, 99, true); // wrong version
    assert.throws(() => parseBigHeader(data), /version 99/);
  });
});
