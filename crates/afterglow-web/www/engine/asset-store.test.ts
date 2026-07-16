import { describe, expect, test } from 'bun:test';
import * as THREE from 'three';
import { AssetAdmission, AssetRequestState, AssetStore, parseGLTFAsset } from './asset-store.ts';

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

describe('stable glTF material identity', () => {
  test('recovers parser material indices independent of names', async () => {
    const scene = new THREE.Group();
    const first = new THREE.MeshStandardMaterial({ name: 'duplicate' });
    const second = new THREE.MeshStandardMaterial({ name: 'duplicate' });
    scene.add(
      new THREE.Mesh(new THREE.BoxGeometry(), first),
      new THREE.Mesh(new THREE.BoxGeometry(), second),
    );
    const associations = new Map<object, { materials?: number }>([
      [first, { materials: 4 }], [second, { materials: 9 }],
    ]);
    const parsed = await parseGLTFAsset(new Uint8Array(8), {
      parse(_data, _path, onLoad) { onLoad({ scene, animations: [], parser: { associations } }); },
    });
    expect(parsed.materialIndices.get(first)).toBe(4);
    expect(parsed.materialIndices.get(second)).toBe(9);
  });
});

describe('rig-preserving runtime mesh optimization', () => {
  test('reorders only triangle indices and retains every skin/morph attribute and skeleton', async () => {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute('position', new THREE.Float32BufferAttribute([
      0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0,
    ], 3));
    geometry.setAttribute('normal', new THREE.Float32BufferAttribute(new Array(12).fill(0), 3));
    geometry.setAttribute('uv', new THREE.Float32BufferAttribute(new Array(8).fill(0), 2));
    geometry.setAttribute('skinIndex', new THREE.Uint16BufferAttribute(new Array(16).fill(0), 4));
    geometry.setAttribute('skinWeight', new THREE.Float32BufferAttribute([
      1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0,
    ], 4));
    geometry.morphAttributes.position = [new THREE.Float32BufferAttribute(new Array(12).fill(0.1), 3)];
    geometry.setIndex([0, 1, 2, 0, 2, 3]);
    const attributes = geometry.attributes;
    const morph = geometry.morphAttributes.position[0];
    const root = new THREE.Bone(), child = new THREE.Bone();
    root.add(child);
    const skeleton = new THREE.Skeleton([root, child]);
    const mesh = new THREE.SkinnedMesh(geometry, new THREE.MeshBasicMaterial());
    mesh.name = 'rig';
    mesh.add(root);
    mesh.bind(skeleton);
    const scene = new THREE.Group();
    scene.add(mesh);
    const meshopt = {
      async optimizeVertexCache(indices: Uint32Array) { return indices.slice().reverse(); },
      async optimizeOverdraw(indices: Uint32Array) { return indices.slice(); },
      async simplifyWithUvs() { throw new Error('skinned scene must not be simplified'); },
      async analyzeVertexCache() { return new Float32Array([0.75]); },
      async encodeIndexBuffer(indices: Uint32Array) { return new Uint8Array(indices.byteLength / 2); },
      poll() {},
    };
    const loader = { async size() { return 0; }, async load() { return new Uint8Array(); },
      async read() { return new Uint8Array(); }, poll() {} };
    const store = new AssetStore(loader, meshopt);
    const stats = await store.optimizeGltfScene(scene);
    expect(stats).toHaveLength(1);
    expect(stats[0].skinned).toBe(true);
    expect(stats[0].preservedAttributes).toEqual(['position', 'normal', 'uv', 'skinIndex', 'skinWeight']);
    expect(geometry.attributes).toBe(attributes);
    expect(geometry.morphAttributes.position[0]).toBe(morph);
    expect(mesh.skeleton).toBe(skeleton);
    expect(mesh.skeleton.bones).toEqual([root, child]);
    expect(Array.from(geometry.index!.array)).toEqual([3, 2, 0, 2, 1, 0]);
  });
});

describe('AssetStore structural continuations', () => {
  test('attaches one continuation at load time and deduplicates handles', async () => {
    let resolveLoad!: (bytes: Uint8Array) => void;
    let loadCalls = 0;
    let parserCalls = 0;
    let polls = 0;
    const loader = {
      async size() { return 4; },
      load() {
        loadCalls++;
        return new Promise<Uint8Array>(resolve => { resolveLoad = resolve; });
      },
      async read() { return new Uint8Array(); },
      poll() { polls++; },
    };
    const store = new AssetStore(loader);
    const first = store.load('value.bin', bytes => {
      parserCalls++;
      return bytes[0];
    }, 0);
    const second = store.load('value.bin', () => 99, 0);
    expect(second).toBe(first);
    expect(loadCalls).toBe(0); // size resolves before load starts
    await flush();
    expect(loadCalls).toBe(1);
    store.poll();
    store.poll();
    expect(polls).toBe(2);
    expect(parserCalls).toBe(0);
    resolveLoad(new Uint8Array([7, 0, 0, 0]));
    await flush(); await flush();
    expect(parserCalls).toBe(1);
    expect(store.stateTable[0]).toBe(AssetRequestState.ReadyToPublish);
    expect(first.state).toBe('loading');
    expect(store.drainCompletions(1)).toBe(1);
    expect(first.asset).toBe(7);
    expect(first.state).toBe('ready');
    expect(store.isLoading('value.bin')).toBe(false);
    expect(store.size).toBe(1);
    expect(store.getHandle('value.bin')).toBe(first);
  });

  test('uses fixed numeric registration/state tables with deterministic exhaustion', () => {
    const loader = {
      async size() { return 0; }, async load() { return new Uint8Array(); },
      async read() { return new Uint8Array(); }, poll() {},
    };
    const store = new AssetStore(loader, undefined, 2);
    const states = store.stateTable;
    expect(store.registerAsset('a')).toBe(0);
    expect(store.registerAsset('b')).toBe(1);
    expect(store.registerAsset('a')).toBe(0);
    expect(store.registerAsset('c')).toBe(-1);
    expect(store.registeredAssetCount).toBe(2);
    expect(store.capacity).toBe(2);
    expect(store.stateTable).toBe(states);
    expect(states[0]).toBe(AssetRequestState.Idle);
    expect(store.tryLoad('c', bytes => bytes).status).toBe(AssetAdmission.CapacityExceeded);
  });

  test('invalidates an evicted parsing generation without publishing stale data', async () => {
    let resolveLoad!: (bytes: Uint8Array) => void;
    let resolveParse!: (asset: { dispose(): void }) => void;
    let disposed = 0;
    const loader = {
      async size() { return 1; },
      load() { return new Promise<Uint8Array>(resolve => { resolveLoad = resolve; }); },
      async read() { return new Uint8Array(); }, poll() {},
    };
    const store = new AssetStore(loader, undefined, 1);
    const id = store.registerAsset('stale');
    const result = store.tryLoadAsset(id, () => new Promise(resolve => { resolveParse = resolve; }));
    expect(result.status).toBe(AssetAdmission.Started);
    await flush();
    resolveLoad(new Uint8Array([1]));
    await flush();
    expect(store.stateTable[id]).toBe(AssetRequestState.Parsing);
    resolveParse({ dispose() { disposed++; } });
    await flush();
    expect(store.stateTable[id]).toBe(AssetRequestState.ReadyToPublish);
    store.evict('stale');
    expect(store.drainCompletions(1)).toBe(1);
    expect(disposed).toBe(1);
    expect(store.stateTable[id]).toBe(AssetRequestState.Idle);
    expect(store.size).toBe(0);
    expect(store.getHandle('stale')).toBeUndefined();
  });

  test('publishes only the configured number of fixed-ring completions per poll', async () => {
    const resolvers: Array<(bytes: Uint8Array) => void> = [];
    const loader = {
      async size() { return 1; },
      load() { return new Promise<Uint8Array>(resolve => resolvers.push(resolve)); },
      async read() { return new Uint8Array(); }, poll() {},
    };
    const store = new AssetStore(loader, undefined, 2, 1);
    const a = store.load('a', bytes => bytes[0], 0);
    const b = store.load('b', bytes => bytes[0], 0);
    await flush();
    resolvers[0](new Uint8Array([1]));
    resolvers[1](new Uint8Array([2]));
    await flush(); await flush();
    expect(store.pendingCompletionCount).toBe(2);
    expect(store.completionQueueHighWater).toBe(2);
    expect(store.completionQueueOverflows).toBe(0);
    store.poll();
    expect(store.pendingCompletionCount).toBe(1);
    expect([a.state, b.state].filter(state => state === 'ready')).toHaveLength(1);
    store.poll();
    expect(store.pendingCompletionCount).toBe(0);
    expect(a.asset).toBe(1);
    expect(b.asset).toBe(2);
  });
});
