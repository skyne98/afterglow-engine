// AssetHandle — a versioned handle to a progressively-loaded asset.
//
// `assetStore.load(path, parser, fallback)` returns a handle immediately.
// The handle starts with the fallback. Internally, the store loads the asset
// (chunked, async, via the poll model). When the asset is ready, the store
// swaps `handle.asset` and increments `handle.generation`.
//
// Consumer code checks the generation number each frame:
//
// ```ts
// const handle = assetStore.load('sky.png', parseTexture, fallbackTexture);
// let lastGen = -1;
//
// // Each frame:
// assetStore.poll();
// if (handle.generation !== lastGen) {
//   material.map = handle.asset;   // swap in the new texture
//   lastGen = handle.generation;
// }
// // else: no-op — asset unchanged this frame
// ```
//
// No callbacks. No effects. No closures. One integer comparison per frame.
// Multiple consumers can read the same handle — they all see the same
// generation. The store is the only writer.

/** Loading state of a handle. */
export type AssetState = 'loading' | 'ready' | 'error';

/**
 * A versioned handle to a progressively-loaded asset.
 *
 * - `asset`: the current value (fallback initially, real asset after load).
 * - `generation`: incremented every time `asset` is swapped. Starts at 0.
 * - `state`: loading state.
 * - `path`: the original request path.
 *
 * The store is the only writer. Consumers only read.
 */
export class AssetHandle<T> {
  /** The current asset. Starts as the fallback (or undefined). */
  asset: T | undefined;

  /** Incremented every time `.asset` is swapped. Consumers compare this to
   * detect changes: `if (handle.generation !== lastGen) { ... }`. */
  generation = 0;

  /** Current loading state. */
  state: AssetState = 'loading';

  /** The original request path. */
  readonly path: string;

  /** @internal */ constructor(path: string, fallback?: T) {
    this.path = path;
    this.asset = fallback;
  }

  /** True if the real asset has been loaded (not the fallback). */
  get isReady(): boolean {
    return this.state === 'ready';
  }

  /** True if loading failed. */
  get isError(): boolean {
    return this.state === 'error';
  }

  /** The current LOD level (-1 = not started, 0 = lowest, 1+ = higher). */
  lod = -1;
}
