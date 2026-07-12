// Tests for AssetStore handle-based API + generation number pattern.
//
//   node --test crates/afterglow-web/tests/engine-resource.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const testDir = fileURLToPath(new URL('.', import.meta.url));
const crateDir = testDir + '../';

function transpile(tsRelPath) {
  const srcPath = crateDir + tsRelPath;
  const outPath = `/tmp/${tsRelPath.split('/').pop().replace('.ts', '.test.js')}`;
  execSync(`bun build ${srcPath} --outfile ${outPath} --target browser --external three`, {
    stdio: 'pipe',
  });
  let src = readFileSync(outPath, 'utf8');
  // Replace all THREE imports (bun renames duplicates to THREE2, etc).
  src = src.replace(/import \* as THREE\w* from "three";/g, '');
  src = 'const THREE = { Texture: class { constructor(b){this.b=b} }, SRGBColorSpace: "srgb", CanvasTexture: class { constructor(c){this.c=c} }, RepeatWrapping: 1000, GLTFLoader: undefined, BoxGeometry: class {}, MeshBasicMaterial: class {}, Mesh: class {}, Group: class { add(){} clone(){return new THREE.Group()} } };\n' + src;
  src = src.replace(/THREE2\./g, 'THREE.');
  return import('data:text/javascript;base64,' + Buffer.from(src, 'utf8').toString('base64'));
}

// --- Fake asset loader ---

class FakeLoader {
  constructor() { this.files = new Map(); this.pollCalls = 0; this.loadCalls = 0; this.sizeCalls = 0; this.readCalls = 0; this.virtualSize = 0; this.virtualRead = null; }
  addFile(path, bytes) { this.files.set(path, bytes); }
  async load(path) { this.loadCalls++; const f = this.files.get(path); if (!f) throw new Error(`not found: ${path}`); return f; }
  async size(path) { this.sizeCalls++; if (this.files.get(path) === null && this.virtualSize) return this.virtualSize; return this.files.get(path)?.byteLength ?? 0; }
  async read(path, offset, len) { this.readCalls++; if (this.virtualRead) return this.virtualRead(offset, len); return this.files.get(path)?.subarray(offset, offset + len); }
  poll() { this.pollCalls++; }
}

// --- Tests ---

test('Resource: lazy creation, get returns same instance, set overwrites', async () => {
  const { defineResource } = await transpile('www/engine/resource.ts');
  let created = 0;
  const Counter = defineResource('counter', () => ({ count: 0, created: ++created }));
  const world = {};
  assert.equal(Counter.has(world), false);
  const c1 = Counter.get(world);
  assert.equal(created, 1);
  const c2 = Counter.get(world);
  assert.equal(created, 1);
  assert.strictEqual(c1, c2);
  Counter.set(world, { count: 42, created: 99 });
  const c3 = Counter.get(world);
  assert.equal(c3.count, 42);
  Counter.remove(world);
  assert.equal(Counter.has(world), false);
});

test('AssetStore: load returns handle immediately with fallback', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('data.json', new TextEncoder().encode('{"hello":"world"}'));
  const store = new AssetStore(loader);

  const handle = store.load('data.json', (b) => JSON.parse(new TextDecoder().decode(b)), 'fallback');

  // Handle is returned immediately — fallback is set, generation is 0.
  assert.equal(handle.asset, 'fallback');
  assert.equal(handle.generation, 0);
  assert.equal(handle.state, 'loading');
  assert.equal(handle.path, 'data.json');
});

test('AssetStore: poll swaps handle.asset + increments generation', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('data.json', new TextEncoder().encode('{"hello":"world"}'));
  const store = new AssetStore(loader);

  const handle = store.load('data.json', (b) => JSON.parse(new TextDecoder().decode(b)), 'fallback');
  assert.equal(handle.asset, 'fallback');
  assert.equal(handle.generation, 0);

  // Drive the store — async load completes, handle swaps.
  // Need to poll + let microtasks resolve.
  store.poll();
  await new Promise(r => setTimeout(r, 50)); // let async resolve

  assert.equal(handle.state, 'ready');
  assert.equal(handle.generation, 1);
  assert.deepEqual(handle.asset, { hello: 'world' });
});

test('AssetStore: consumer checks generation (the core pattern)', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('data.json', new TextEncoder().encode('[1,2,3]'));
  const store = new AssetStore(loader);

  const handle = store.load('data.json', (b) => JSON.parse(new TextDecoder().decode(b)), null);
  let lastGen = -1;
  let boundAsset = null;

  // Frame 1 — not loaded yet.
  store.poll();
  await new Promise(r => setTimeout(r, 10));
  if (handle.generation !== lastGen) { boundAsset = handle.asset; lastGen = handle.generation; }
  // Either still fallback (gen 0) or loaded (gen 1) depending on timing.

  // Drive until loaded.
  for (let i = 0; i < 10 && handle.state === 'loading'; i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 20));
  }

  // Now loaded — generation should be 1.
  assert.equal(handle.state, 'ready');
  assert.equal(handle.generation, 1);

  // Consumer sees the generation change.
  if (handle.generation !== lastGen) { boundAsset = handle.asset; lastGen = handle.generation; }
  assert.deepEqual(boundAsset, [1, 2, 3]);
  assert.equal(lastGen, 1);

  // Next frame — no change, no swap.
  const prevBound = boundAsset;
  store.poll();
  if (handle.generation !== lastGen) { boundAsset = handle.asset; lastGen = handle.generation; }
  assert.strictEqual(boundAsset, prevBound); // no re-bind
});

test('AssetStore: duplicate load returns same handle', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('x.json', new TextEncoder().encode('{}'));
  const store = new AssetStore(loader);

  const h1 = store.load('x.json', (b) => JSON.parse(new TextDecoder().decode(b)), null);
  const h2 = store.load('x.json', (b) => JSON.parse(new TextDecoder().decode(b)), null);
  assert.strictEqual(h1, h2, 'same handle returned');
});

test('AssetStore: 2 MiB asset loads via chunked read', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  const size = 2 * 1024 * 1024;
  const full = new Uint8Array(size);
  for (let i = 0; i < size; i++) full[i] = (i * 7 + 13) % 256;
  loader.addFile('big.bin', full);
  const store = new AssetStore(loader);

  const handle = store.load('big.bin', (b) => b, null);
  assert.equal(handle.state, 'loading');

  for (let i = 0; i < 20 && handle.state === 'loading'; i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 50));
  }

  assert.equal(handle.state, 'ready');
  assert.equal(handle.generation, 1);
  assert.equal(handle.asset.byteLength, size);
  assert.equal(loader.loadCalls, 0, 'load() NOT called for >1 MiB');
  assert.ok(loader.readCalls > 1, `read() called ${loader.readCalls} times`);
});

test('AssetStore: small asset uses single load', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('small.txt', new TextEncoder().encode('hi'));
  const store = new AssetStore(loader);

  const handle = store.load('small.txt', (b) => new TextDecoder().decode(b), null);
  for (let i = 0; i < 10 && handle.state === 'loading'; i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 20));
  }
  assert.equal(handle.state, 'ready');
  assert.equal(handle.asset, 'hi');
  assert.equal(loader.loadCalls, 1);
  assert.equal(loader.readCalls, 0);
});

test('AssetStore: 500 MB synthetic asset', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  const totalSize = 500 * 1024 * 1024;
  loader.files.set('huge.glb', null);
  loader.virtualSize = totalSize;
  loader.virtualRead = (offset, len) => {
    const chunk = new Uint8Array(len);
    for (let i = 0; i < len; i++) chunk[i] = ((offset + i) * 7 + 13) % 256;
    return chunk;
  };
  const store = new AssetStore(loader);

  const handle = store.load('huge.glb', (bytes) => {
    assert.equal(bytes.byteLength, totalSize);
    return { size: bytes.byteLength };
  }, null);

  for (let i = 0; i < 100 && handle.state === 'loading'; i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 10));
  }

  assert.equal(handle.state, 'ready');
  assert.equal(handle.generation, 1);
  assert.equal(handle.asset.size, totalSize);
  assert.equal(loader.loadCalls, 0);
  assert.ok(loader.readCalls > 100, `read() called ${loader.readCalls} times`);
});

test('AssetStore: poll forwards to loader', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  const store = new AssetStore(loader);
  store.poll();
  store.poll();
  assert.equal(loader.pollCalls, 2);
});
