import type { BigHeader, TextureEncoding } from './big-format.ts';

interface RuntimeMipDirectory {
  pagesX: number;
  pagesY: number;
  offsets: Float64Array;
  sizes: Uint32Array;
}

interface RuntimeTextureDirectory {
  encoding: TextureEncoding;
  mips: (RuntimeMipDirectory | null)[];
  tailOffset: number;
  tailSize: number;
}

export interface VtPageAddress {
  path: string;
  mip: number;
  x: number;
  y: number;
  tail?: boolean;
}

export interface ResolvedVtPage {
  path: string;
  mip: number;
  x: number;
  y: number;
  tail: boolean;
  offset: number;
  length: number;
  encoding: TextureEncoding;
}

/** Immutable, direct-indexed view of every VT page range in one BIG header. */
export class VtPageDirectory {
  private readonly textures = new Map<string, RuntimeTextureDirectory>();

  constructor(header: BigHeader) {
    for (const asset of header.assets) {
      const source = asset.virtualTexture;
      if (!source) continue;
      let maxMip = 0;
      for (const mip of source.mips) maxMip = Math.max(maxMip, mip.mip);
      const mips: (RuntimeMipDirectory | null)[] = new Array(maxMip + 1).fill(null);
      for (const mip of source.mips) {
        const sizes = Uint32Array.from(mip.pageSizes);
        const offsets = new Float64Array(sizes.length);
        let offset = Number(mip.offset);
        for (let page = 0; page < sizes.length; page++) {
          offsets[page] = offset;
          offset += sizes[page] ?? 0;
        }
        mips[mip.mip] = {
          pagesX: mip.pagesX,
          pagesY: mip.pagesY,
          offsets,
          sizes,
        };
      }
      this.textures.set(asset.name, {
        encoding: source.encoding,
        mips,
        tailOffset: source.tail ? Number(source.tail.offset) : 0,
        tailSize: source.tail?.size ?? 0,
      });
    }
  }

  resolve(address: VtPageAddress): ResolvedVtPage {
    const texture = this.textures.get(address.path);
    if (!texture)
      throw new Error(`VT directory not found: ${address.path}`);
    let offset = 0;
    let length = 0;
    if (address.tail) {
      offset = texture.tailOffset;
      length = texture.tailSize;
    } else {
      const mip = texture.mips[address.mip];
      if (!mip || address.x < 0 || address.y < 0 ||
          address.x >= mip.pagesX || address.y >= mip.pagesY) {
        throw new Error(
          `VT page out of range: ${address.path} mip=${address.mip} (${address.x},${address.y})`,
        );
      }
      const page = address.y * mip.pagesX + address.x;
      offset = mip.offsets[page] ?? 0;
      length = mip.sizes[page] ?? 0;
    }
    if (length === 0) {
      throw new Error(
        `VT page not found: ${address.path} mip=${address.mip} (${address.x},${address.y})`,
      );
    }
    return {
      path: address.path,
      mip: address.mip,
      x: address.x,
      y: address.y,
      tail: address.tail === true,
      offset,
      length,
      encoding: texture.encoding,
    };
  }
}
