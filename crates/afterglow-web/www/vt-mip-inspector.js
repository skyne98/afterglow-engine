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

// crates/afterglow-web/web/src/demos/vt-mip-inspector/main.ts
var seed = 40503;
var virtualCrop = 32768;
var grid = document.querySelector("#grid");
function direct(mip) {
  const size = virtualCrop >> mip, pages = Math.ceil(size / VT_PAGE_SIZE), data = new Uint8ClampedArray(size * size * 4);
  for (let py = 0;py < pages; py++)
    for (let px = 0;px < pages; px++) {
      const p = generateStonePage(seed, mip, px, py, 131072);
      for (let y = 0;y < Math.min(VT_PAGE_SIZE, size - py * VT_PAGE_SIZE); y++) {
        const src = ((y + VT_PAGE_BORDER) * VT_SLOT_SIZE + VT_PAGE_BORDER) * 4, dst = ((py * VT_PAGE_SIZE + y) * size + px * VT_PAGE_SIZE) * 4;
        data.set(p.subarray(src, src + Math.min(VT_PAGE_SIZE, size - px * VT_PAGE_SIZE) * 4), dst);
      }
    }
  return { size, data };
}
function down(src) {
  const size = src.size >> 1, out = new Uint8ClampedArray(size * size * 4);
  for (let y = 0;y < size; y++)
    for (let x = 0;x < size; x++)
      for (let c = 0;c < 4; c++) {
        let n = 0;
        for (let oy = 0;oy < 2; oy++)
          for (let ox = 0;ox < 2; ox++)
            n += src.data[((y * 2 + oy) * src.size + x * 2 + ox) * 4 + c];
        out[(y * size + x) * 4 + c] = Math.round(n / 4);
      }
  return { size, data: out };
}
function bmp(img) {
  const row = img.size * 4, bytes = new Uint8Array(54 + row * img.size), v = new DataView(bytes.buffer);
  bytes.set([66, 77]);
  v.setUint32(2, bytes.length, true);
  v.setUint32(10, 54, true);
  v.setUint32(14, 40, true);
  v.setInt32(18, img.size, true);
  v.setInt32(22, img.size, true);
  v.setUint16(26, 1, true);
  v.setUint16(28, 32, true);
  v.setUint32(34, row * img.size, true);
  for (let y = 0;y < img.size; y++)
    for (let x = 0;x < img.size; x++) {
      const s = (y * img.size + x) * 4, d = 54 + ((img.size - 1 - y) * img.size + x) * 4;
      bytes[d] = img.data[s + 2];
      bytes[d + 1] = img.data[s + 1];
      bytes[d + 2] = img.data[s];
      bytes[d + 3] = 255;
    }
  return URL.createObjectURL(new Blob([bytes], { type: "image/bmp" }));
}
function panel(title, img) {
  const p = document.createElement("div");
  p.className = "panel";
  p.innerHTML = `<b>${title}</b><br>${img.size}×${img.size}`;
  const view = document.createElement("img");
  view.className = "view";
  view.src = bmp(img);
  p.append(view);
  grid.append(p);
}
var addLabel = (text) => {
  const e = document.createElement("div");
  e.className = "label";
  e.textContent = text;
  grid.append(e);
};
var levels = [];
levels[7] = direct(7);
for (let m = 8;m <= 10; m++)
  levels[m] = down(levels[m - 1]);
grid.append(document.createElement("div"));
for (let m = 8;m <= 10; m++) {
  const e = document.createElement("div");
  e.innerHTML = `<b>Mip ${m}</b>`;
  grid.append(e);
}
addLabel("independent");
for (let m = 8;m <= 10; m++)
  panel("runtime", direct(m));
addLabel("box chain");
for (let m = 8;m <= 10; m++)
  panel("filtered", levels[m]);
