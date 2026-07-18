// crates/afterglow-web/web/src/engine/virtual-texturing/procedural-vt.ts
var VT_PAGE_SIZE = 128;
var VT_PAGE_BORDER = 4;
var VT_SLOT_SIZE = VT_PAGE_SIZE + VT_PAGE_BORDER * 2;
function noiseHash(x, y, seed = 0) {
  let h = Math.imul(x | 0, 521288629) ^ Math.imul(y | 0, 1597334677) ^ Math.imul(seed | 0, 1831565813);
  h = Math.imul(h ^ h >>> 15, h | 1);
  h ^= h + Math.imul(h ^ h >>> 7, h | 61);
  return ((h ^ h >>> 14) >>> 0) / 4294967295;
}
function smoothNoise(x, y, scale, seed = 0) {
  const gx = x / scale, gy = y / scale, ix = Math.floor(gx), iy = Math.floor(gy), fx = gx - ix, fy = gy - iy;
  const sx = fx * fx * (3 - 2 * fx), sy = fy * fy * (3 - 2 * fy);
  const a = noiseHash(ix, iy, seed), b = noiseHash(ix + 1, iy, seed);
  const c = noiseHash(ix, iy + 1, seed), d = noiseHash(ix + 1, iy + 1, seed);
  return a + (b - a) * sx + (c + (d - c) * sx - (a + (b - a) * sx)) * sy;
}
function pagePixels(mip, pageX, pageY, virtualSize, pixel) {
  const out = new Uint8Array(VT_SLOT_SIZE * VT_SLOT_SIZE * 4), scale = 2 ** mip;
  for (let sy = 0;sy < VT_SLOT_SIZE; sy++)
    for (let sx = 0;sx < VT_SLOT_SIZE; sx++) {
      const mx = pageX * VT_PAGE_SIZE + sx - VT_PAGE_BORDER, my = pageY * VT_PAGE_SIZE + sy - VT_PAGE_BORDER;
      const x = Math.max(0, Math.min(virtualSize - 1, mx * scale));
      const y = Math.max(0, Math.min(virtualSize - 1, my * scale));
      const [r, g, b] = pixel(x, y, scale, mip);
      const i = (sy * VT_SLOT_SIZE + sx) * 4;
      out[i] = r;
      out[i + 1] = g;
      out[i + 2] = b;
      out[i + 3] = 255;
    }
  return out;
}
function generateTerrainPage(mip, pageX, pageY, virtualSize) {
  return pagePixels(mip, pageX, pageY, virtualSize, (x, y, mipScale) => {
    let value = 0, amplitude = 0.55, total = 0;
    for (const scale of [16384, 4096, 1024, 256, 64]) {
      if (scale >= mipScale * 2) {
        value += smoothNoise(x, y, scale) * amplitude;
        total += amplitude;
      }
      amplitude *= 0.5;
    }
    if (!total) {
      value = smoothNoise(x, y, mipScale * 2);
      total = 1;
    }
    value /= total;
    const ridge = 1 - Math.abs(value * 2 - 1), elevation = Math.max(0, Math.min(1, value * 0.72 + ridge * 0.28));
    let r, g, b;
    if (elevation < 0.38) {
      const t = elevation / 0.38;
      r = 8 + 18 * t;
      g = 24 + 55 * t;
      b = 55 + 95 * t;
    } else if (elevation < 0.58) {
      const t = (elevation - 0.38) / 0.2;
      r = 24 + 42 * t;
      g = 72 + 55 * t;
      b = 45 + 22 * t;
    } else if (elevation < 0.78) {
      const t = (elevation - 0.58) / 0.2;
      r = 66 + 78 * t;
      g = 127 + 48 * t;
      b = 67 + 54 * t;
    } else {
      const t = (elevation - 0.78) / 0.22;
      r = 144 + 105 * t;
      g = 175 + 74 * t;
      b = 121 + 128 * t;
    }
    const edge = Math.min(x, y, virtualSize - 1 - x, virtualSize - 1 - y);
    if (edge < 1024) {
      const checker = (Math.floor(x / 256) + Math.floor(y / 256) & 1) === 0;
      r = 255;
      g = checker ? 245 : 92;
      b = checker ? 235 : 18;
    }
    return [r, g, b];
  });
}
function sampleStoneSurface(seed, x, y) {
  const blockX = 16384, blockY = 8192, row = Math.floor(y / blockY), stagger = (row & 1) * blockX * 0.5;
  const bx = Math.floor((x + stagger) / blockX), by = row, base = 72 + noiseHash(bx, by, seed) * 58;
  const grain = smoothNoise(x, y, 1024, seed) * 22 + smoothNoise(x, y, 256, seed + 17) * 10 - 16;
  const crack = smoothNoise(x + seed * 31, y - seed * 19, 2048, seed + 91) > 0.865 && smoothNoise(x, y, 256, seed) < 0.42;
  const tint = seed * 13 % 19, v = Math.max(22, Math.min(190, base + grain - (crack ? 38 : 0)));
  return [v + tint * 0.35, v * 0.96, v * 0.88];
}
function sampleStoneBase(seed, x, y) {
  const blockX = 16384, blockY = 8192, row = Math.floor(y / blockY), stagger = (row & 1) * blockX * 0.5;
  const lx = ((x + stagger) % blockX + blockX) % blockX, ly = (y % blockY + blockY) % blockY;
  return Math.min(lx, blockX - lx, ly, blockY - ly) < 96 ? [38, 35, 31] : sampleStoneSurface(seed, x, y);
}
function periodicLineCoverage(start, size, period, halfWidth = 96) {
  const end = start + size;
  let covered = 0;
  for (let line = Math.floor((start - halfWidth) / period);line <= Math.ceil((end + halfWidth) / period); line++) {
    const center = line * period;
    covered += Math.max(0, Math.min(end, center + halfWidth) - Math.max(start, center - halfWidth));
  }
  return Math.min(1, covered / size);
}
function sampleStoneResized(seed, x, y, mipScale, virtualSize) {
  if (mipScale === 1)
    return sampleStoneBase(seed, x, y);
  const sum = [0, 0, 0], samples = 4, step = mipScale / samples;
  for (let sy = 0;sy < samples; sy++)
    for (let sx = 0;sx < samples; sx++) {
      const sample = sampleStoneSurface(seed, Math.min(virtualSize - 1, x + (sx + 0.5) * step), Math.min(virtualSize - 1, y + (sy + 0.5) * step));
      sum[0] += sample[0];
      sum[1] += sample[1];
      sum[2] += sample[2];
    }
  const surface = sum.map((channel) => channel / (samples * samples));
  const blockX = 16384, blockY = 8192, row = Math.floor((y + mipScale * 0.5) / blockY), stagger = (row & 1) * blockX * 0.5;
  const horizontal = periodicLineCoverage(y, mipScale, blockY), vertical = periodicLineCoverage(x + stagger, mipScale, blockX);
  const coverage = horizontal + vertical - horizontal * vertical, mortar = [38, 35, 31];
  return surface.map((channel, index) => channel + (mortar[index] - channel) * coverage);
}
function generateStonePage(seed, mip, pageX, pageY, virtualSize = 131072) {
  return pagePixels(mip, pageX, pageY, virtualSize, (x, y, mipScale) => sampleStoneResized(seed, x, y, mipScale, virtualSize));
}
function createStoneMipChain(seed, virtualSize = 131072, sourceMip = 7) {
  const levels = new Map, size = virtualSize >> sourceMip, data = new Uint8Array(size * size * 4), scale = 2 ** sourceMip;
  for (let y = 0;y < size; y++)
    for (let x = 0;x < size; x++) {
      const color = sampleStoneResized(seed, x * scale, y * scale, scale, virtualSize), i = (y * size + x) * 4;
      data[i] = color[0];
      data[i + 1] = color[1];
      data[i + 2] = color[2];
      data[i + 3] = 255;
    }
  levels.set(sourceMip, { size, data });
  for (let mip = sourceMip + 1, previous = { size, data };previous.size > 1; mip++) {
    const nextSize = previous.size >> 1, next = new Uint8Array(nextSize * nextSize * 4);
    for (let y = 0;y < nextSize; y++)
      for (let x = 0;x < nextSize; x++)
        for (let c = 0;c < 4; c++) {
          let sum = 0;
          for (let oy = 0;oy < 2; oy++)
            for (let ox = 0;ox < 2; ox++)
              sum += previous.data[((y * 2 + oy) * previous.size + x * 2 + ox) * 4 + c];
          next[(y * nextSize + x) * 4 + c] = Math.round(sum / 4);
        }
    previous = { size: nextSize, data: next };
    levels.set(mip, previous);
  }
  return { seed, virtualSize, sourceMip, levels };
}
function pageFromStoneMipChain(chain, mip, pageX, pageY) {
  if (mip < chain.sourceMip)
    return generateStonePage(chain.seed, mip, pageX, pageY, chain.virtualSize);
  const level = chain.levels.get(mip);
  if (!level)
    throw new RangeError(`stone mip ${mip} is unavailable`);
  const out = new Uint8Array(VT_SLOT_SIZE * VT_SLOT_SIZE * 4);
  for (let sy = 0;sy < VT_SLOT_SIZE; sy++)
    for (let sx = 0;sx < VT_SLOT_SIZE; sx++) {
      const x = Math.max(0, Math.min(level.size - 1, pageX * VT_PAGE_SIZE + sx - VT_PAGE_BORDER));
      const y = Math.max(0, Math.min(level.size - 1, pageY * VT_PAGE_SIZE + sy - VT_PAGE_BORDER));
      const src = (y * level.size + x) * 4, dst = (sy * VT_SLOT_SIZE + sx) * 4;
      out.set(level.data.subarray(src, src + 4), dst);
    }
  return out;
}
export {
  smoothNoise,
  sampleStoneBase,
  pageFromStoneMipChain,
  noiseHash,
  generateTerrainPage,
  generateStonePage,
  createStoneMipChain,
  VT_SLOT_SIZE,
  VT_PAGE_SIZE,
  VT_PAGE_BORDER
};
