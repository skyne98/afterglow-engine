/* maipointo wasm loader — hand-written, no Emscripten runtime.
 * Instantiates the pure-Rust module and exposes the same interface the
 * paint worker expects (exports + HEAPU8/HEAP32 + _malloc/_free). */

export interface MaipoModule {
  memory: WebAssembly.Memory;
  HEAPU8: Uint8Array;
  HEAP32: Int32Array;
  [name: string]: unknown;
}

let loadPromise: Promise<MaipoModule> | null = null;

/** Load the maipointo wasm module (plain C-ABI, no runtime glue). */
export function loadBrushModule(): Promise<MaipoModule> {
  if (loadPromise) return loadPromise;
  loadPromise = (async () => {
    const dynamicImport = new Function('url', 'return import(url)') as (url: string) => Promise<any>;
    // The build script drops an ESM default export next to the wasm.
    const js = await dynamicImport('/wasm/brushlib.js');
    const wasmUrl: string = js.brushlibWasm ?? '/wasm/brushlib.wasm';
    const fetchSource = js.brushlibFetch ?? ((u: string) => fetch(u));
    const response = await fetch(wasmUrl);
    const bytes = await response.arrayBuffer();
    // The module imports its memory (--import-memory --shared-memory); the
    // host owns it so HEAP views stay valid across growth.
    const memory = new WebAssembly.Memory({ initial: 256, maximum: 1024, shared: true });
    const { instance } = await WebAssembly.instantiate(bytes, { env: { memory } });
    const exports = instance.exports as unknown as Record<string, CallableFunction>;
    const mod: MaipoModule = {
      memory,
      get HEAPU8() { return new Uint8Array(memory.buffer); },
      get HEAP32() { return new Int32Array(memory.buffer); },
      _malloc: exports._malloc as (n: number) => number,
      _free: exports._free as (p: number, n: number) => void,
    };
    // Expose every paint export under its emscripten-era `_name`.
    for (const name of Object.keys(exports)) {
      if (name === 'memory' || name === '_malloc' || name === '_free') continue;
      const key = `_${name}`;
      if (!(key in mod)) {
        mod[key] = exports[name];
      }
    }
    return mod;
  })();
  return loadPromise;
}
