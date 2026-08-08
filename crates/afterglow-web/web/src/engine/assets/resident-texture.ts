// Resident (non-virtual) texture loading from a `.big` container.
//
// Resident textures are single-mip, always-resident byte streams stored as
// `AssetType::Texture` chunks (uncompressed, with an explicit `TextureFormat`).
// They are sampled directly at runtime — no page table, no mip tail, no VT
// feedback. The canonical use is the R8 height field consumed by the POM march,
// kept out of VT so the march loop pays one direct fetch per step at mip 0.

import type { BigHeader, ChunkInfo, TextureFormat } from './big-format.ts';

/** Minimal Three constructor surface used to build a DataTexture. */
export type ResidentTextureThree = {
  DataTexture: new (
    data: Uint8Array,
    width: number,
    height: number,
    format: number,
    type: number,
  ) => ResidentTexture;
  RedFormat: number;
  RGBAFormat: number;
  UnsignedByteType: number;
  RepeatWrapping: number;
  LinearFilter: number;
  NoColorSpace: string;
};

/** The texture handle returned to callers; mirrors THREE.DataTexture surface. */
export interface ResidentTexture {
  readonly image: { data: Uint8Array; width: number; height: number };
  readonly format: number;
  readonly type: number;
  wrapS: number;
  wrapT: number;
  minFilter: number;
  magFilter: number;
  generateMipmaps: boolean;
  flipY: boolean;
  colorSpace: string;
  unpackAlignment: number;
  needsUpdate: boolean;
  name: string;
  dispose(): void;
}

/** A byte-range source for a single `.big` container. */
export interface ResidentTextureSource {
  read(offset: number, length: number): Promise<Uint8Array>;
}

export interface ResidentTextureResult {
  readonly texture: ResidentTexture;
  readonly format: TextureFormat;
  readonly width: number;
  readonly height: number;
}

/** Bytes per texel for a resident texture format. */
export function residentTextureBytesPerTexel(format: TextureFormat): number {
  return format === 'R8' ? 1 : 4;
}

/** Find the single resident `Texture` chunk for `name`, validating its shape. */
export function findResidentTextureChunk(header: BigHeader, name: string): ChunkInfo {
  const asset = header.assets.find((entry) => entry.name === name);
  if (!asset) throw new Error(`resident texture not found in BIG: ${name}`);
  if (asset.assetType !== 'Texture') {
    throw new Error(`BIG asset ${name} is ${asset.assetType}, not a resident Texture`);
  }
  if (asset.chunks.length !== 1) {
    throw new Error(`resident texture ${name} must have one chunk, got ${asset.chunks.length}`);
  }
  const chunk = asset.chunks[0];
  if (!chunk) throw new Error(`resident texture ${name} has no chunk`);
  if (chunk.meta.type !== 'Texture') {
    throw new Error(`resident texture ${name} chunk is ${chunk.meta.type}, not Texture`);
  }
  if (!chunk.meta.format) {
    throw new Error(`resident texture ${name} chunk is missing its format field`);
  }
  if (chunk.compression !== 'None') {
    throw new Error(`resident texture ${name} must be uncompressed, got ${chunk.compression}`);
  }
  const expected =
    (chunk.meta.width ?? 0) * (chunk.meta.height ?? 0) * residentTextureBytesPerTexel(chunk.meta.format);
  if (Number(chunk.uncompressedSize) !== expected) {
    throw new Error(
      `resident texture ${name} byte length ${chunk.uncompressedSize} != ${expected} (w*h*bpp)`,
    );
  }
  return chunk;
}

/**
 * Load a resident (non-virtual) texture from a `.big` container by asset name.
 *
 * Builds a `DataTexture` with the correct GPU format for the stored
 * `TextureFormat`:
 * - `R8`   → `RedFormat` + `UnsignedByteType` (WebGPU `r8unorm`, filterable,
 *   samples as f32 in [0,1] — no `float32-filterable` feature required).
 * - `Rgba8`→ `RGBAFormat` + `UnsignedByteType`.
 *
 * The texture is mip-0, repeat-wrapped, linear-filtered, and `NoColorSpace`
 * (it carries data, not color). Caller owns disposal.
 */
export async function loadResidentTexture(
  three: ResidentTextureThree,
  source: ResidentTextureSource,
  header: BigHeader,
  name: string,
): Promise<ResidentTextureResult> {
  const chunk = findResidentTextureChunk(header, name);
  const format = chunk.meta.format as TextureFormat;
  const width = chunk.meta.width as number;
  const height = chunk.meta.height as number;
  const bytes = await source.read(Number(chunk.offset), Number(chunk.uncompressedSize));
  if (bytes.byteLength !== Number(chunk.uncompressedSize)) {
    throw new Error(
      `resident texture ${name} read ${bytes.byteLength} bytes, expected ${chunk.uncompressedSize}`,
    );
  }
  // Copy into a owned Uint8Array so the texture owns its backing store
  // (range reads may alias a shared transfer buffer).
  const data = new Uint8Array(bytes.byteLength);
  data.set(bytes);
  const [threeFormat, threeType] =
    format === 'R8'
      ? [three.RedFormat, three.UnsignedByteType]
      : [three.RGBAFormat, three.UnsignedByteType];
  const texture = new three.DataTexture(data, width, height, threeFormat, threeType);
  texture.name = name;
  texture.wrapS = texture.wrapT = three.RepeatWrapping;
  texture.minFilter = texture.magFilter = three.LinearFilter;
  texture.generateMipmaps = false;
  texture.flipY = false;
  texture.colorSpace = three.NoColorSpace;
  texture.unpackAlignment = 1;
  texture.needsUpdate = true;
  return { texture, format, width, height };
}
