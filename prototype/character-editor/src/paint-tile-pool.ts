/* Tile-parallel blend pool: `physical cores - 1` workers, each with an
 * isolated maipointo module instance (own linear memory — message passing,
 * no shared Rust state). The paint worker serializes each blend job
 * (the ops bytes + the tile bytes), dispatches round-robin, and collects
 * the blended tiles. Falls back to inline draining when the pool cannot
 * start. */

const POOL_CAP = 16;

export interface TilePool {
  workerCount: number;
  /** True once every pool worker finished instantiating. */
  ready(): boolean;
  /** Blend one job off-thread. Resolves with the blended tile bytes. */
  blend(job: {
    id: number;
    tx: number;
    ty: number;
    opCount: number;
    ops: Uint8Array;
    tile: Uint8Array;
  }): Promise<Uint8Array>;
}

export async function spawnTilePool(
  memoryOpSize: number,
  onLog?: (text: string) => void,
  logical = navigator.hardwareConcurrency || 8,
): Promise<TilePool> {
  const count = Math.max(1, Math.min(POOL_CAP, Math.floor(logical / 2) - 1));
  const readyFlag = new SharedArrayBuffer(4);
  const readyView = new Int32Array(readyFlag);
  const workers: Worker[] = [];
  let dispatched = 0;
  let failed = false;
  type Pending = {
    resolve: (value: Uint8Array) => void;
    reject: (reason: Error) => void;
  };
  const pending = new Map<number, Pending>();
  const fail = (message: string) => {
    if (failed) return;
    failed = true;
    onLog?.(message);
    for (const entry of pending.values()) entry.reject(new Error(message));
    pending.clear();
    for (const worker of workers) worker.terminate();
  };

  for (let i = 0; i < count; i++) {
    const w = new Worker(new URL('./paint-tile-pool-worker.ts', import.meta.url), { type: 'module' });
    w.onmessage = (e: MessageEvent) => {
      const d = e.data;
      if (d?.type === 'log') { onLog?.(d.text); return; }
      if (d?.type === 'bootFailed') { fail(d.text); return; }
      if (d?.type === 'jobDone') {
        const entry = pending.get(d.id);
        if (entry) {
          pending.delete(d.id);
          entry.resolve(new Uint8Array(d.tile));
        }
      }
    };
    w.onerror = (e: ErrorEvent) => {
      fail(`tile pool worker ${i} error: ${(e as ErrorEvent)?.message ?? 'unknown'}`);
    };
    w.postMessage({ cmd: 'boot', workerId: i + 1, readyFlag, opSize: memoryOpSize });
    workers.push(w);
  }

  // The boot completes in the background (15 module instantiations take
  // tens of seconds); the drain falls back to inline until then.

  return {
    workerCount: count,
    ready() {
      return !failed && Atomics.load(readyView, 0) >= count;
    },
    blend(job): Promise<Uint8Array> {
      if (failed) return Promise.reject(new Error('The tile pool is unavailable.'));
      const id = dispatched++;
      const promise = new Promise<Uint8Array>((resolve, reject) => {
        pending.set(id, { resolve, reject });
      });
      const worker = workers[id % workers.length];
      worker.postMessage({ cmd: 'job', id, tx: job.tx, ty: job.ty, opCount: job.opCount, ops: job.ops, tile: job.tile }, [job.ops.buffer, job.tile.buffer]);
      return promise;
    },
  };
}
