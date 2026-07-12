// Tests for AssetStore — progressive mip streaming, format detection,
// Basis transcoding pipeline, generation handles.
//
//   node --test crates/afterglow-web/tests/asset-store.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const testDir = fileURLToPath(new URL('.', import.meta.url));
const crateDir = testDir + '../';

function transpile(tsRelPath) {
  const srcPath = crateDir + tsRelPath;
  const outPath = `/tmp/${tsRelPath.split('/').pop().replace('.ts', '.test.js')}`;
  execSync(`bun build ${srcPath} --outfile ${outPath} --target browser --external three`, { stdio: 'pipe' });
  let src = readFileSync(outPath, 'utf8');
  // Replace ALL THREE imports (bun renames duplicates to THREE2, THREE3, etc).
  src = src.replace(/import \* as THREE\w* from "three";/g, '');
  // Also replace any THREE2/THREE3 references with THREE.
  src = src.replace(/\bTHREE\d+\b/g, 'THREE');
  // Prepend mock.
  const mock = `const THREE={Texture:class{constructor(b){this.bitmap=b;this.needsUpdate=false;this.colorSpace=null}dispose(){}},DataTexture:class{constructor(d,w,h,f){this.image={data:d,width:w,height:h};this.mipmaps=[];this.generateMipmaps=false;this.needsUpdate=false;this.colorSpace=null;this.minFilter=0;this.magFilter=0}dispose(){}},CanvasTexture:class{constructor(c){this.canvas=c;this.needsUpdate=false;this.colorSpace=null;this.wrapS=0;this.wrapT=0;this.repeat={set(){}}}dispose(){}},BoxGeometry:class{constructor(){}dispose(){}},MeshBasicMaterial:class{constructor(o){Object.assign(this,o)}dispose(){}},Mesh:class{constructor(){}},Group:class{constructor(){this.name=''}add(){}clone(){return new THREE.Group()}},RGBAFormat:1023,SRGBColorSpace:'srgb',RepeatWrapping:1000,LinearMipmapLinearFilter:1008,LinearFilter:1006};globalThis.document=globalThis.document||{createElement:function(){return{width:0,height:0,getContext:function(){return{fillStyle:'',fillRect:function(){}}}}}};globalThis.createImageBitmap=globalThis.createImageBitmap||(async function(){return{}});\n`;
  const finalPath = outPath.replace('.test.js', '.mocked.mjs');
  writeFileSync(finalPath, mock + src);
  return import('file://' + finalPath);
}

// --- Fakes ---

class FakeLoader {
  constructor() { this.files = new Map(); this.pollCalls = 0; }
  addFile(path, bytes) { this.files.set(path, bytes); }
  async load(path) { const f = this.files.get(path); if (!f) throw new Error(`not found: ${path}`); return f; }
  async size(path) { return this.files.get(path)?.byteLength ?? 0; }
  async read(path, off, len) { return this.files.get(path)?.subarray(off, off + len); }
  poll() { this.pollCalls++; }
}

class FakeTranscoder {
  constructor() { this.pollCalls = 0; this.transcodeCalls = 0; this.mipData = null; }
  /** Set the serialized mip data to return from transcode(). */
  setMips(mips) {
    // Serialize: [count(u32)][w0(u32)][h0(u32)][len0(u32)][data0...]...
    const parts = [];
    const countBuf = new Uint8Array(4);
    new DataView(countBuf.buffer).setUint32(0, mips.length, true);
    parts.push(countBuf);
    for (const m of mips) {
      const hdr = new Uint8Array(12);
      const dv = new DataView(hdr.buffer);
      dv.setUint32(0, m.width, true);
      dv.setUint32(4, m.height, true);
      dv.setUint32(8, m.data.length, true);
      parts.push(hdr, m.data);
    }
    this.mipData = Buffer.concat(parts.map(p => Buffer.from(p)));
  }
  async transcode(data, format) {
    this.transcodeCalls++;
    if (this.mipData) return this.mipData;
    // Return raw bytes if no mips configured.
    return data;
  }
  async generateMips(data, w, h) { return data; }
  async downscale(data, w, h, tw, th) { return new Uint8Array(tw * th * 4); }
  poll() { this.pollCalls++; }
}

// --- Helper: drive async operations ---

function flushPromises() {
  return new Promise(r => setTimeout(r, 10));
}

async function drive(store, frames = 20) {
  for (let i = 0; i < frames; i++) {
    store.poll();
    await flushPromises();
  }
}

// --- Tests ---

test('AssetStore: loadTexture returns handle immediately with fallback', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('sky.png', new Uint8Array([0xFF])); // add file to avoid rejection
  const store = new AssetStore(loader);

  const handle = store.loadTexture('sky.png');
  assert.ok(handle, 'handle returned');
  assert.equal(handle.generation, 0, 'generation starts at 0');
  assert.equal(handle.state, 'loading', 'state is loading');
  assert.ok(handle.asset, 'fallback asset is set');
});

test('AssetStore: regular PNG texture loads and increments generation', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  // Minimal valid PNG (1×1 red pixel).
  const png = Buffer.from([
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1×1
    0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, // 8-bit RGB
    0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, // IDAT chunk
    0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01,
    0x62, 0xB1, 0x36, 0x37, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
  ]);
  loader.addFile('sky.png', png);
  const store = new AssetStore(loader);

  const handle = store.loadTexture('sky.png');
  assert.equal(handle.generation, 0);
  assert.equal(handle.state, 'loading');

  await drive(store, 30);

  assert.equal(handle.state, 'ready', 'should be ready after loading');
  assert.ok(handle.generation > 0, 'generation should have incremented');
});

test('AssetStore: Basis texture creates streaming texture with empty mipmaps', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  const transcoder = new FakeTranscoder();
  loader.addFile('sky.basis', new Uint8Array([0xAB, 0xCD]));
  const store = new AssetStore(loader, undefined, transcoder);

  const handle = store.loadTexture('sky.basis');
  assert.ok(handle.asset, 'texture created immediately');
  assert.ok(handle.asset.mipmaps, 'texture has mipmaps array');
  assert.equal(handle.asset.mipmaps.length, 0, 'mipmaps array starts empty');
  assert.equal(handle.asset.generateMipmaps, false, 'auto-mipmaps disabled');
  assert.equal(handle.generation, 0, 'generation starts at 0');
});

test('AssetStore: Basis texture streams mips progressively', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  const transcoder = new FakeTranscoder();

  // Configure transcoder to return 3 mip levels.
  transcoder.setMips([
    { width: 8, height: 8, data: new Uint8Array(8 * 8 * 4).fill(0xAA) },
    { width: 4, height: 4, data: new Uint8Array(4 * 4 * 4).fill(0xBB) },
    { width: 2, height: 2, data: new Uint8Array(2 * 2 * 4).fill(0xCC) },
  ]);

  loader.addFile('sky.basis', new Uint8Array([0x01, 0x02, 0x03]));
  const store = new AssetStore(loader, undefined, transcoder);

  const handle = store.loadTexture('sky.basis');
  assert.equal(handle.asset.mipmaps.length, 0, 'no mips yet');

  await drive(store, 30);

  // After driving, mips should have been uploaded.
  assert.ok(handle.asset.mipmaps.length > 0, 'mipmaps should have been uploaded');
  assert.ok(handle.generation > 0, 'generation should have incremented');
  assert.equal(handle.state, 'ready', 'should be ready after all mips uploaded');
  assert.equal(transcoder.transcodeCalls, 1, 'transcode called once');
});

test('AssetStore: poll drives both loader and transcoder', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  const transcoder = new FakeTranscoder();
  const store = new AssetStore(loader, undefined, transcoder);

  store.poll();
  store.poll();
  assert.equal(loader.pollCalls, 2, 'loader polled');
  assert.equal(transcoder.pollCalls, 2, 'transcoder polled');
});

test('AssetStore: duplicate load returns same handle', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('a.png', new Uint8Array([0xFF]));
  const store = new AssetStore(loader);

  const h1 = store.loadTexture('a.png');
  const h2 = store.loadTexture('a.png');
  assert.strictEqual(h1, h2, 'same handle returned');
});

test('AssetStore: isLoading and has', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('a.png', new Uint8Array([0xFF]));
  const store = new AssetStore(loader);

  assert.equal(store.has('a.png'), false, 'not cached yet');
  assert.equal(store.isLoading('a.png'), false, 'not loading yet');

  store.loadTexture('a.png');
  assert.equal(store.isLoading('a.png'), true, 'now loading');
  assert.equal(store.has('a.png'), false, 'not cached yet');

  await drive(store, 20);
  assert.equal(store.has('a.png'), true, 'cached after load');
});

test('AssetStore: cachedPaths and size', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('a.png', new Uint8Array([0xFF]));
  loader.addFile('b.png', new Uint8Array([0xFF]));
  const store = new AssetStore(loader);

  store.loadTexture('a.png');
  store.loadTexture('b.png');
  await drive(store, 20);

  assert.equal(store.size, 2, '2 cached assets');
  assert.ok(store.cachedPaths.includes('a.png'), 'has a.png');
  assert.ok(store.cachedPaths.includes('b.png'), 'has b.png');
});

test('AssetStore: evict removes from cache', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('a.png', new Uint8Array([0xFF]));
  const store = new AssetStore(loader);

  store.loadTexture('a.png');
  await drive(store, 20);
  assert.equal(store.has('a.png'), true);

  store.evict('a.png');
  assert.equal(store.has('a.png'), false, 'evicted');
});

test('AssetStore: large file chunked loading', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  // Create a file larger than 1 MiB.
  const big = new Uint8Array(2 * 1024 * 1024);
  for (let i = 0; i < big.length; i++) big[i] = i % 256;
  loader.addFile('big.bin', big);
  const store = new AssetStore(loader);

  const handle = store.load('big.bin', (bytes) => bytes, new Uint8Array());
  await drive(store, 30);

  assert.equal(handle.state, 'ready', 'large file loaded');
  assert.ok(handle.asset.length === 2 * 1024 * 1024, 'full data loaded');
  // Verify data integrity.
  for (let i = 0; i < 100; i++) {
    assert.equal(handle.asset[i], i % 256, `byte ${i} matches`);
  }
});

test('AssetStore: Basis texture without transcoder falls back to regular load', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('sky.basis', new Uint8Array([0xFF]));
  // No transcoder passed.
  const store = new AssetStore(loader);

  // Should not throw — just loads as regular file.
  const handle = store.loadTexture('sky.basis');
  assert.ok(handle, 'handle returned');
});

test('AssetStore: loadBasisTexture requires transcoder', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  const store = new AssetStore(loader);

  assert.throws(() => store.loadBasisTexture('sky.basis'), /No texture worker/);
});

test('AssetStore: loadJSON parses correctly', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('data.json', new TextEncoder().encode('{"hello":"world","n":42}'));
  const store = new AssetStore(loader);

  const handle = store.loadJSON('data.json');
  await drive(store, 20);

  assert.equal(handle.state, 'ready');
  assert.deepEqual(handle.asset, { hello: 'world', n: 42 });
});
