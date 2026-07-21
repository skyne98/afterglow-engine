// Lossless single-channel 16-bit displacement interchange decoder.
//
// The `.r16` format is a cook-time interchange produced by
// `afterglow-pipeline height-r16` and consumed by `afterglow-pipeline
// resident-texture --format r8` (which quantizes 16->8). The runtime no longer
// loads `.r16` directly — height ships as a resident R8 `.big` asset loaded via
// `loadResidentTexture` (see resident-texture.ts). This module remains for
// source-asset validation tests of the committed `.r16` intermediates.

const HEIGHT_R16_MAGIC = new Uint8Array([0x41, 0x47, 0x52, 0x31, 0x36, 0x4c, 0x45, 0x01]);
const HEIGHT_R16_HEADER_BYTES = 16;

export type HeightR16 = Readonly<{
  width: number;
  height: number;
  pixels: Uint16Array;
}>;

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
