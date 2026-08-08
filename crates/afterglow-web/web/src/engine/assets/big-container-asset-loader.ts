import type { FetchRangeLoader } from './asset-range.ts';
import type { BigHeader, ChunkInfo } from './big-format.ts';

/**
 * Asset-loader view over raw, uncompressed payloads packed in a `.big` file.
 * The path is the container asset name, not a deployment URL. Indexing occurs
 * once at bootstrap; runtime reads are direct bounded ranges.
 */
export class BigContainerAssetLoader {
  private readonly assets = new Map<string, ChunkInfo>();

  constructor(
    private readonly source: FetchRangeLoader,
    private readonly containerPath: string,
    header: BigHeader,
  ) {
    for (const asset of header.assets) {
      if (asset.chunks.length !== 1 || asset.chunks[0].meta.type !== 'Raw') continue;
      const chunk = asset.chunks[0];
      if (chunk.compression !== 'None' || chunk.compressedSize !== chunk.uncompressedSize)
        throw new Error(`raw BIG asset must be uncompressed: ${asset.name}`);
      if (chunk.uncompressedSize > BigInt(Number.MAX_SAFE_INTEGER))
        throw new RangeError(`raw BIG asset exceeds browser safe size: ${asset.name}`);
      this.assets.set(asset.name, chunk);
    }
  }

  private chunk(path: string): ChunkInfo {
    const chunk = this.assets.get(path);
    if (!chunk) throw new Error(`raw BIG asset not found: ${path}`);
    return chunk;
  }

  load(path: string): Promise<Uint8Array> {
    const chunk = this.chunk(path);
    return this.source.read(this.containerPath, Number(chunk.offset), Number(chunk.uncompressedSize));
  }

  async size(path: string): Promise<number> { return Number(this.chunk(path).uncompressedSize); }

  read(path: string, offset: number, length: number): Promise<Uint8Array> {
    const chunk = this.chunk(path);
    const size = Number(chunk.uncompressedSize);
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(length) || length < 0 || offset + length > size)
      throw new RangeError(`raw BIG asset range exceeds ${path}: ${offset}+${length} > ${size}`);
    return this.source.read(this.containerPath, Number(chunk.offset) + offset, length);
  }

  poll(): void {}
}
