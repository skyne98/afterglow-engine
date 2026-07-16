import { describe, expect, test } from 'bun:test';
import { AssetAdmission, AssetRequestState, AssetStore } from './asset-store.ts';

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

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
