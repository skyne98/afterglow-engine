// Tests for the .big format parser (JavaScript side).
// Run: node --test crates/afterglow-web/tests/big-parser.test.mjs

import { test, describe } from 'node:test';
import assert from 'node:assert';
import { readFileSync } from 'node:fs';

// We need to transpile the TS — use bun to build first
import { createPageDataProvider, parseBigHeader, findVTPageChunk, BIG_MAGIC, BIG_VERSION } from '../www/engine/big-parser.js';

describe('BigParser', () => {
  test('BIG_MAGIC is "BIG1" as u32 LE', () => {
    // "BIG1" = B=0x42, I=0x49, G=0x47, 1=0x31 → LE u32 = 0x31474942
    assert.strictEqual(BIG_MAGIC, 0x31474942);
  });

  test('BIG_VERSION is 3', () => {
    assert.strictEqual(BIG_VERSION, 3);
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

  test('raw RGBA pages bypass Basis only for the RGBA atlas', async () => {
    const page = new Uint8Array(136 * 136 * 4).fill(7);
    const chunk = { offset: 0n, compressedSize: BigInt(page.length), meta: {
      type: 'VirtualTexturePage', mip: 0, pageX: 0, pageY: 0, encoding: 'RawRgba8',
    }};
    const header = { assets: [{ name: 'terrain', chunks: [chunk] }] };
    const loader = { read: async () => page };
    const worker = { transcode: async () => { throw new Error('must not transcode raw'); }, poll() {} };
    const rgbaProvider = createPageDataProvider(loader, header, worker, 4);
    assert.strictEqual(await rgbaProvider('terrain', { mip: 0, x: 0, y: 0 }), page);
    const bc7Provider = createPageDataProvider(loader, header, worker, 0);
    await assert.rejects(() => bc7Provider('terrain', { mip: 0, x: 0, y: 0 }), /requires Basis/);
  });

  test('Basis provider strips the texture-worker mip envelope', async () => {
    const payload = new Uint8Array([1, 2, 3, 4]);
    const envelope = new Uint8Array(16 + payload.length);
    const view = new DataView(envelope.buffer);
    view.setUint32(0, 1, true); view.setUint32(4, 136, true);
    view.setUint32(8, 136, true); view.setUint32(12, payload.length, true);
    envelope.set(payload, 16);
    const chunk = { offset: 0n, compressedSize: 10n, meta: {
      type: 'VirtualTexturePage', mip: 0, pageX: 0, pageY: 0, encoding: 'Basis',
    }};
    const header = { assets: [{ name: 'terrain', chunks: [chunk] }] };
    const provider = createPageDataProvider(
      { read: async () => new Uint8Array(10) }, header,
      { transcode: async () => envelope, poll() {} }, 0,
    );
    assert.deepStrictEqual(await provider('terrain', { mip: 0, x: 0, y: 0 }), payload);
  });
});
