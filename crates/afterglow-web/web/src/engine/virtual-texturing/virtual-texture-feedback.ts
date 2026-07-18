// RG32Uint feedback encoding shared by readback code and tests.
// word 0: valid[31], y[17..27], x[6..16], mip[0..5]
// word 1: full 32-bit virtual-texture ID.

export interface DecodedFeedback {
  textureId: number;
  mip: number;
  x: number;
  y: number;
}

const MAX_MIP = 0x3f;
const MAX_COORD = 0x7ff;

export function encodeFeedback(textureId: number, mip: number, x: number, y: number): [number, number] {
  if (![textureId, mip, x, y].every(Number.isSafeInteger)) throw new RangeError('feedback fields must be integers');
  if (textureId < 0 || textureId > 0xffffffff) throw new RangeError('textureId must fit u32');
  if (mip < 0 || mip > MAX_MIP) throw new RangeError('mip must fit 6 bits');
  if (x < 0 || x > MAX_COORD || y < 0 || y > MAX_COORD)
    throw new RangeError('feedback page coordinates must fit 11 bits');
  const packed = (0x80000000 | mip | (x << 6) | (y << 17)) >>> 0;
  return [packed, textureId >>> 0];
}

export function decodeFeedback(packed: number, textureId: number): DecodedFeedback | null {
  packed >>>= 0;
  if ((packed & 0x80000000) === 0) return null;
  return {
    textureId: textureId >>> 0,
    mip: packed & MAX_MIP,
    x: (packed >>> 6) & MAX_COORD,
    y: (packed >>> 17) & MAX_COORD,
  };
}
