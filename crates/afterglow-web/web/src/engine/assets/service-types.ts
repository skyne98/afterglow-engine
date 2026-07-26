import type { MeshOptimizer } from './asset-store.ts';

/** Pure byte-to-byte texture transform used by the public-web pipeline. */
export interface TextureTranscoder {
  /** True only when every response has independent immutable backing. */
  readonly responseIsOwned?: true;
  transcode(data: Uint8Array, targetFormat: number): Promise<Uint8Array>;
}

/** Optional native capability: read and transcode a retained source range
 * without exposing its encoded bytes to JavaScript. */
export interface SourceTextureTranscoder extends TextureTranscoder {
  transcodeSourceRange(
    offset: number,
    length: number,
    targetFormat: number,
  ): Promise<Uint8Array>;
}

export interface OwnedTextureTranscoder extends TextureTranscoder {
  transcodeSourceRange?(
    offset: number,
    length: number,
    targetFormat: number,
  ): Promise<Uint8Array>;
  close(): void | Promise<void>;
}

export interface OwnedMeshOptimizer extends MeshOptimizer {
  close(): void | Promise<void>;
}

export function hasSourceTextureTranscoder(
  worker: TextureTranscoder,
): worker is SourceTextureTranscoder {
  return typeof (worker as Partial<SourceTextureTranscoder>).transcodeSourceRange === 'function';
}
