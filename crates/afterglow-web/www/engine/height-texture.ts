const HEIGHT_R16_MAGIC = new Uint8Array([0x41, 0x47, 0x52, 0x31, 0x36, 0x4c, 0x45, 0x01]);
const HEIGHT_R16_HEADER_BYTES = 16;

export type HeightR16 = Readonly<{
  width: number;
  height: number;
  pixels: Uint16Array;
}>;

type HeightTexture = {
  image: { data: Float32Array; width: number; height: number };
  format: number;
  type: number;
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
};

type HeightThree = {
  DataTexture: new (data: Float32Array, width: number, height: number, format: number, type: number) => HeightTexture;
  RedFormat: number;
  FloatType: number;
  RepeatWrapping: number;
  LinearFilter: number;
  NoColorSpace: string;
};

/** Parse the offline pipeline's exact, versioned little-endian R16 payload. */
export function parseHeightR16(buffer: ArrayBuffer): HeightR16 {
  if (buffer.byteLength < HEIGHT_R16_HEADER_BYTES) throw new Error('R16 height payload is truncated');
  const bytes = new Uint8Array(buffer);
  for (let index = 0; index < HEIGHT_R16_MAGIC.length; index++) {
    if (bytes[index] !== HEIGHT_R16_MAGIC[index]) throw new Error('R16 height magic/version mismatch');
  }
  const header = new DataView(buffer, 8, 8);
  const width = header.getUint32(0, true);
  const height = header.getUint32(4, true);
  if (width === 0 || height === 0) throw new Error('R16 height dimensions must be non-zero');
  const count = width * height;
  if (!Number.isSafeInteger(count)) throw new Error('R16 height dimensions overflow');
  const expectedBytes = HEIGHT_R16_HEADER_BYTES + count * 2;
  if (buffer.byteLength !== expectedBytes) {
    throw new Error(`R16 height byte length mismatch: expected ${expectedBytes}, got ${buffer.byteLength}`);
  }
  const endianProbe = new Uint16Array([0x0102]);
  if (new Uint8Array(endianProbe.buffer)[0] !== 0x02) throw new Error('R16 height loading requires a little-endian platform');
  return { width, height, pixels: new Uint16Array(buffer, HEIGHT_R16_HEADER_BYTES, count) };
}

/** Three's filterable precision-preserving path requires float32 filtering. */
export function assertHeightTextureSupport(device: { features?: { has(name: string): boolean } }): void {
  if (device.features?.has('float32-filterable') !== true) {
    throw new Error('16-bit displacement requires the WebGPU float32-filterable feature');
  }
}

/** Verify Three actually created the required GPU format after warm-up. */
export function assertHeightTextureGpuFormat(
  backend: { utils?: { getTextureFormatGPU(texture: HeightTexture): string | undefined } },
  texture: HeightTexture,
): void {
  const format = backend.utils?.getTextureFormatGPU(texture);
  if (format !== 'r32float') throw new Error(`displacement GPU format mismatch: expected r32float, got ${format ?? 'unavailable'}`);
}

/** Load exact R16 source samples into a filterable, single-channel float texture. */
export async function loadHeightTextureR16(
  three: HeightThree,
  device: { features?: { has(name: string): boolean } },
  url: string,
): Promise<HeightTexture> {
  assertHeightTextureSupport(device);
  const response = await fetch(url);
  if (!response.ok) throw new Error(`failed to load R16 height ${url}: HTTP ${response.status}`);
  const asset = parseHeightR16(await response.arrayBuffer());
  // Float32 has enough mantissa precision to keep every normalized u16 value
  // distinct. R16_UNORM itself is unfilterable in WebGPU and Three r185 cannot
  // generate a valid custom-function binding for that format.
  const normalized = new Float32Array(asset.pixels.length);
  for (let index = 0; index < asset.pixels.length; index++) normalized[index] = asset.pixels[index] / 65535;
  const texture = new three.DataTexture(normalized, asset.width, asset.height, three.RedFormat, three.FloatType);
  texture.name = url;
  texture.wrapS = texture.wrapT = three.RepeatWrapping;
  texture.minFilter = texture.magFilter = three.LinearFilter;
  texture.generateMipmaps = false;
  texture.flipY = false;
  texture.colorSpace = three.NoColorSpace;
  texture.unpackAlignment = 4;
  texture.needsUpdate = true;
  return texture;
}
