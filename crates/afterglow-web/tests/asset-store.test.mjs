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
  src = src.replace(/import \* as THREE\w* from "three";/g, '');
  src = src.replace(/\bTHREE\d+\b/g, 'THREE');
  const mock = `const THREE={Texture:class{constructor(b){this.bitmap=b;this.needsUpdate=false;this.colorSpace=null}dispose(){}},DataTexture:class{constructor(d,w,h,f){this.image={data:d,width:w,height:h};this.mipmaps=[];this.generateMipmaps=false;this.needsUpdate=false;this.colorSpace=null;this.minFilter=0;this.magFilter=0}dispose(){}},CanvasTexture:class{constructor(c){this.canvas=c;this.needsUpdate=false;this.colorSpace=null;this.wrapS=0;this.wrapT=0;this.repeat={set(){}}}dispose(){}},BoxGeometry:class{constructor(){}dispose(){}},MeshBasicMaterial:class{constructor(o){Object.assign(this,o)}dispose(){}},Mesh:class{constructor(){}},Group:class{constructor(){this.name=''}add(){}clone(){return new THREE.Group()}},RGBAFormat:1023,SRGBColorSpace:'srgb',RepeatWrapping:1000,LinearMipmapLinearFilter:1008,LinearFilter:1006};globalThis.document=globalThis.document||{createElement:function(){return{width:0,height:0,getContext:function(){return{fillStyle:'',fillRect:function(){}}}}}};globalThis.createImageBitmap=globalThis.createImageBitmap||(async function(){return{}});\n`;
  const finalPath = outPath.replace('.test.js', '.mocked.mjs');
  writeFileSync(finalPath, mock + src);
  return import('file://' + finalPath);
}

class FakeLoader {
  constructor() { this.files = new Map(); this.pollCalls = 0; }
  addFile(name, data) { this.files.set(name, data); }
  async load(path) { this.pollCalls++; return this.files.get(path); }
  async size(path) { return this.files.get(path)?.byteLength ?? 0; }
  async read(path, off, len) { return this.files.get(path)?.subarray(off, off + len); }
  poll() { this.pollCalls++; }
}

test('loadTexture returns handle immediately (no VT store)', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('sky.png', new Uint8Array([0xFF, 0x00, 0xFF]));
  const store = new AssetStore(loader);
  const handle = store.loadTexture('sky.png');
  assert.ok(handle, 'handle returned');
  assert.equal(handle.state, 'loading');
});

test('regular PNG texture loads and increments generation', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('a.png', new Uint8Array([0xFF]));
  const store = new AssetStore(loader);
  const handle = store.loadTexture('a.png');
  for (let i = 0; i < 30 && handle.state === 'loading'; i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 10));
  }
  assert.equal(handle.state, 'ready');
  assert.ok(handle.generation > 0, 'generation incremented');
});

test('duplicate load returns same handle', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('a.png', new Uint8Array([0xFF]));
  const store = new AssetStore(loader);
  const h1 = store.loadTexture('a.png');
  const h2 = store.loadTexture('a.png');
  assert.equal(h1, h2, 'same handle returned');
});

test('isLoading and has', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('a.png', new Uint8Array([0xFF]));
  const store = new AssetStore(loader);
  store.loadTexture('a.png');
  assert.equal(store.isLoading('a.png'), true);
  assert.equal(store.has('a.png'), false);
  for (let i = 0; i < 30 && store.isLoading('a.png'); i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 10));
  }
  assert.equal(store.has('a.png'), true);
  assert.equal(store.isLoading('a.png'), false);
});

test('cachedPaths and size', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('a.png', new Uint8Array([0xFF]));
  loader.addFile('b.png', new Uint8Array([0xFF]));
  const store = new AssetStore(loader);
  store.loadTexture('a.png');
  store.loadTexture('b.png');
  for (let i = 0; i < 30 && store.isLoading('a.png'); i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 10));
  }
  assert.equal(store.size, 2);
  assert.deepEqual(store.cachedPaths.sort(), ['a.png', 'b.png']);
});

test('evict removes from cache', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('a.png', new Uint8Array([0xFF]));
  const store = new AssetStore(loader);
  store.loadTexture('a.png');
  for (let i = 0; i < 30 && store.isLoading('a.png'); i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 10));
  }
  assert.equal(store.has('a.png'), true);
  store.evict('a.png');
  assert.equal(store.has('a.png'), false);
});

test('large file chunked loading', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  const bigData = new Uint8Array(2 * 1024 * 1024);
  bigData.fill(0xAA);
  loader.addFile('big.png', bigData);
  const store = new AssetStore(loader);
  const handle = store.load('big.png', (bytes) => bytes);
  for (let i = 0; i < 30 && handle.state === 'loading'; i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 10));
  }
  assert.equal(handle.state, 'ready');
  assert.equal(handle.asset.length, 2 * 1024 * 1024);
});

test('loadJSON parses correctly', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  loader.addFile('data.json', new TextEncoder().encode('{"name":"test","value":42}'));
  const store = new AssetStore(loader);
  const handle = store.loadJSON('data.json');
  for (let i = 0; i < 30 && handle.state === 'loading'; i++) {
    store.poll();
    await new Promise(r => setTimeout(r, 10));
  }
  assert.equal(handle.state, 'ready');
  assert.equal(handle.asset.name, 'test');
  assert.equal(handle.asset.value, 42);
});

test('poll drives loader', async () => {
  const { AssetStore } = await transpile('www/engine/asset-store.ts');
  const loader = new FakeLoader();
  const store = new AssetStore(loader);
  store.poll();
  store.poll();
  assert.ok(loader.pollCalls >= 2, 'loader polled');
});
