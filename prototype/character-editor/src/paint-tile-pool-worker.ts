/* Tile-pool worker: one isolated maipointo module instance per worker
 * (own linear memory — two Rust allocators never share state). The paint
 * worker serializes each blend job (the ops bytes + the tile bytes); this
 * worker blends in place and returns the tile. One tile is owned by
 * exactly one job. */

const TILE_PX = 64 * 64 * 4;
const MAX_OPS = 16384;

let mod: any = null;
let opsArena = 0;
let tileArena = 0;

self.onmessage = (e: MessageEvent) => {
  const d = e.data;
  if (d?.cmd === 'boot') {
    void (async () => {
      try {
        const dynamicImport = new Function('url', 'return import(url)') as (url: string) => Promise<any>;
        const js = await dynamicImport('/wasm/brushlib.js');
        const bytes = await (await fetch(js.brushlibWasm ?? '/wasm/brushlib.wasm')).arrayBuffer();
        // Own memory: the module requires shared memory, but this instance
        // is single-threaded and shares nothing with the paint worker.
        const memory = new WebAssembly.Memory({ initial: 128, maximum: 4096, shared: true });
        const { instance } = await WebAssembly.instantiate(bytes, { env: { memory } });
        mod = instance.exports;
        opsArena = mod.paint_ops_arena_ptr();
        tileArena = mod.paint_pool_tile_ptr();
        const ready = new Int32Array(d.readyFlag);
        Atomics.store(ready, 0, 1);
        Atomics.notify(ready, 0);
      } catch (err) {
        (self as unknown as Worker).postMessage({
          type: 'log',
          text: `tile pool worker ${d.workerId} failed: ${(err as Error)?.message ?? err}`,
        });
      }
      return;
    })();
    return;
  }
  if (d?.cmd !== 'job' || !mod) return;
  // d.ops: Uint8Array (op bytes), d.tile: Uint8Array (64x64 rgba16), d.tx/d.ty.
  const heap = new Uint8Array(mod.memory.buffer);
  heap.set(d.ops, opsArena);
  heap.set(d.tile, tileArena);
  mod.paint_blend_tile_ops(opsArena, d.opCount, tileArena, d.tx, d.ty);
  const blended = new Uint8Array(memory_slice(mod, tileArena, TILE_PX * 2));
  (self as unknown as Worker).postMessage({ type: 'jobDone', id: d.id, tile: blended }, [blended.buffer]);
};

function memory_slice(mod: any, ptr: number, len: number): ArrayBuffer {
  // Copy out so the buffer is transferable independent of the wasm memory.
  const out = new Uint8Array(len);
  out.set(new Uint8Array(mod.memory.buffer, ptr, len));
  return out.buffer;
}
