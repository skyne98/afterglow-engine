// Tests for the ECS Resource concept and AssetStore cache behavior.
//
//   node --test crates/afterglow-web/tests/engine-resource.test.mjs
//
// Transpiles the TS source via `bun build` (same tool xtask uses), then
// imports the JS via data: URL (no npm deps needed).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const testDir = fileURLToPath(new URL('.', import.meta.url));
const crateDir = testDir + '../';  // crates/afterglow-web/

// Transpile TS → JS via bun, then import via data: URL.
// For modules that import `three`, the import is replaced with a mock.
function transpile(tsRelPath) {
  const srcPath = crateDir + tsRelPath;
  const outPath = `/tmp/${tsRelPath.split('/').pop().replace('.ts', '.test.js')}`;
  execSync(`bun build ${srcPath} --outfile ${outPath} --target browser --external three`, {
    stdio: 'pipe',
  });
  let src = readFileSync(outPath, 'utf8');
  // Replace `import * as THREE from "three"` with a mock THREE object.
  // The AssetStore cache logic doesn't use THREE — only the parsers do.
  src = src.replace(
    /import \* as THREE from "three";/g,
    'const THREE = { Texture: class { constructor(b){this.b=b} }, SRGBColorSpace: "srgb", GLTFLoader: undefined };',
  );
  return import('data:text/javascript;base64,' + Buffer.from(src, 'utf8').toString('base64'));
}

// --- Fake asset loader ---

class FakeLoader {
  constructor() { this.files = new Map(); this.pollCalls = 0; this.loadCalls = 0; this.sizeCalls = 0; this.readCalls = 0; }
  addFile(path, bytes) { this.files.set(path, bytes); }
  async load(path) { this.loadCalls++; const f = this.files.get(path); if (!f) throw new Error(`not found: ${path}`); return f; }
  async size(path) { this.sizeCalls++; return this.files.get(path)?.byteLength ?? 0; }
  async read(path, offset, len) { this.readCalls++; return this.files.get(path)?.subarray(offset, offset + len); }
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
  assert.equal(c1.count, 0);

  const c2 = Counter.get(world);
  assert.equal(created, 1); // not re-created
  assert.strictEqual(c1, c2);

  Counter.set(world, { count: 42, created: 99 });
  const c3 = Counter.get(world);
  assert.equal(c3.count, 42);

  Counter.remove(world);
  assert.equal(Counter.has(world), false);
});

test('AssetStore: load caches, get returns sync, no duplicate loads', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');

  const loader = new FakeLoader();
  loader.addFile('data.json', new TextEncoder().encode('{"hello":"world"}'));
  const store = new AssetStore(loader);

  assert.equal(store.has('data.json'), false);
  assert.equal(store.get('data.json'), undefined);

  const result = await store.load('data.json', (b) => JSON.parse(new TextDecoder().decode(b)));
  assert.deepEqual(result, { hello: 'world' });
  assert.equal(store.has('data.json'), true);

  const cached = store.get('data.json');
  assert.deepEqual(cached, { hello: 'world' });

  // Second load returns cached — no new worker call.
  const prevCalls = loader.loadCalls;
  await store.load('data.json', (b) => JSON.parse(new TextDecoder().decode(b)));
  assert.equal(loader.loadCalls, prevCalls, 'should not re-load from worker');
});

test('AssetStore: concurrent loads of same path share one promise', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');

  const loader = new FakeLoader();
  loader.addFile('x.json', new TextEncoder().encode('[1,2,3]'));
  const store = new AssetStore(loader);

  const p1 = store.load('x.json', (b) => JSON.parse(new TextDecoder().decode(b)));
  const p2 = store.load('x.json', (b) => JSON.parse(new TextDecoder().decode(b)));

  const [r1, r2] = await Promise.all([p1, p2]);
  assert.deepEqual(r1, [1, 2, 3]);
  assert.deepEqual(r2, [1, 2, 3]);
  assert.equal(loader.loadCalls, 1, 'only one worker call for concurrent loads');
});

test('AssetStore: large assets auto-chunk via size + read', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');

  const loader = new FakeLoader();
  // 3 KB file — small, but we'll make the store think it's large by
  // checking that size() is called and read() is used when > MAX_SINGLE_LOAD.
  // Since we can't easily mock the 1 MiB constant, just verify the
  // size-then-load path works (size is called for every load).
  const full = new Uint8Array(3072);
  for (let i = 0; i < 3072; i++) full[i] = i % 256;
  loader.addFile('big.bin', full);
  const store = new AssetStore(loader);

  const result = await store.load('big.bin', (b) => b);
  assert.equal(result.byteLength, 3072);
  for (let i = 0; i < 3072; i++) assert.equal(result[i], i % 256);
});

test('AssetStore: 2 MiB model loads via chunked read (not single load)', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');

  const loader = new FakeLoader();

  // 2 MiB of deterministic data — larger than the 1 MiB response ring.
  const size = 2 * 1024 * 1024;
  const full = new Uint8Array(size);
  for (let i = 0; i < size; i++) full[i] = (i * 7 + 13) % 256;
  loader.addFile('model.glb', full);

  const store = new AssetStore(loader);

  // Track progress.
  const progress = [];
  const result = await store.load('model.glb', (bytes) => {
    // Parser receives the fully reassembled bytes.
    assert.equal(bytes.byteLength, size, 'parser received full 2 MiB');
    return { byteLength: bytes.byteLength, first: bytes[0], last: bytes[size - 1] };
  }, (loaded, total) => progress.push({ loaded, total }));

  // The store must have used the chunked path: size() + read(), not load().
  assert.equal(loader.sizeCalls, 1, 'size() called once');
  assert.equal(loader.loadCalls, 0, 'load() NOT called for >1 MiB asset');
  assert.ok(loader.readCalls > 1, `read() called ${loader.readCalls} times (chunked)`);

  // Progress was reported.
  assert.ok(progress.length > 0, 'progress callback fired');
  assert.equal(progress[progress.length - 1].loaded, size, 'progress reached 100%');
  assert.equal(progress[0].total, size, 'total reported correctly');

  // Data integrity: every byte matches.
  assert.equal(result.byteLength, size);
  assert.equal(result.first, full[0]);
  assert.equal(result.last, full[size - 1]);

  // Cached — second get is sync.
  assert.equal(store.has('model.glb'), true);
  const cached = store.get('model.glb');
  assert.equal(cached.byteLength, size);
});

test('AssetStore: small asset uses single load (not chunked)', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');

  const loader = new FakeLoader();
  const small = new TextEncoder().encode('small file');
  loader.addFile('small.txt', small);

  const store = new AssetStore(loader);
  const result = await store.load('small.txt', (b) => new TextDecoder().decode(b));

  assert.equal(result, 'small file');
  assert.equal(loader.loadCalls, 1, 'load() called once');
  assert.equal(loader.readCalls, 0, 'read() NOT called for small asset');
});

test('AssetStore: poll forwards to loader', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');

  const loader = new FakeLoader();
  const store = new AssetStore(loader);

  store.poll();
  store.poll();
  assert.equal(loader.pollCalls, 2);
});
