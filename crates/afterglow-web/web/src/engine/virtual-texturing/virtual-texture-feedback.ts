// RG32Uint feedback encoding shared by readback code and tests.
// word 0: valid[31], camera closeness[28..30], y[17..27], x[6..16], mip[0..5]
// word 1: full 32-bit virtual-texture ID.

export interface DecodedFeedback {
  textureId: number;
  mip: number;
  x: number;
  y: number;
  cameraCloseness: number;
}

const MAX_MIP = 0x3f;
const MAX_COORD = 0x7ff;
const MAX_CLOSENESS = 0x7;

export function encodeFeedback(
  textureId: number,
  mip: number,
  x: number,
  y: number,
  cameraCloseness = 0,
): [number, number] {
  if (![textureId, mip, x, y, cameraCloseness].every(Number.isSafeInteger))
    throw new RangeError('feedback fields must be integers');
  if (textureId < 0 || textureId > 0xffffffff) throw new RangeError('textureId must fit u32');
  if (mip < 0 || mip > MAX_MIP) throw new RangeError('mip must fit 6 bits');
  if (x < 0 || x > MAX_COORD || y < 0 || y > MAX_COORD)
    throw new RangeError('feedback page coordinates must fit 11 bits');
  if (cameraCloseness < 0 || cameraCloseness > MAX_CLOSENESS)
    throw new RangeError('feedback camera closeness must fit 3 bits');
  const packed = (
    0x80000000 | (cameraCloseness << 28) | mip | (x << 6) | (y << 17)
  ) >>> 0;
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
    cameraCloseness: (packed >>> 28) & MAX_CLOSENESS,
  };
}
