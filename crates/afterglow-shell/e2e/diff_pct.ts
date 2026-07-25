// Match three.js test/e2e/image.js: decode PNG/JPEG with the same libraries,
// box-downscale the native render to reference size with coverage weighting,
// then count pixels whose normalized squared RGB distance exceeds 0.1.
const { PNG } = require('pngjs');
const jpeg = require('jpeg-js');
const fs = require('fs');

const decode = (path) => {
  const bytes = fs.readFileSync(path);
  if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) {
    return PNG.sync.read(bytes);
  }
  const image = jpeg.decode(bytes, { useTArray: true });
  return { width: image.width, height: image.height, data: image.data };
};

const actual = decode(process.argv[2]);
const reference = decode(process.argv[3]);
const width = reference.width;
const height = reference.height;
const scaleX = actual.width / width;
const scaleY = actual.height / height;

if (scaleX < 1 || scaleY < 1) {
  throw new Error(`Cannot downscale ${actual.width}x${actual.height} to ${width}x${height}`);
}

const downscaled = new Uint8Array(width * height * 4);
for (let y = 0; y < height; y++) {
  for (let x = 0; x < width; x++) {
    const sourceX0 = x * scaleX;
    const sourceY0 = y * scaleY;
    const sourceX1 = (x + 1) * scaleX;
    const sourceY1 = (y + 1) * scaleY;
    const x0 = Math.floor(sourceX0);
    const y0 = Math.floor(sourceY0);
    const x1 = Math.min(Math.ceil(sourceX1), actual.width);
    const y1 = Math.min(Math.ceil(sourceY1), actual.height);
    const sums = [0, 0, 0, 0];
    let totalWeight = 0;

    for (let sy = y0; sy < y1; sy++) {
      for (let sx = x0; sx < x1; sx++) {
        const wx0 = Math.max(0, Math.min(1, sx + 1 - sourceX0));
        const wx1 = Math.max(0, Math.min(1, sourceX1 - sx));
        const wy0 = Math.max(0, Math.min(1, sy + 1 - sourceY0));
        const wy1 = Math.max(0, Math.min(1, sourceY1 - sy));
        const weight = Math.min(wx0, wx1) * Math.min(wy0, wy1);
        const source = (sy * actual.width + sx) * 4;
        for (let channel = 0; channel < 4; channel++) {
          sums[channel] += actual.data[source + channel] * weight;
        }
        totalWeight += weight;
      }
    }

    const destination = (y * width + x) * 4;
    for (let channel = 0; channel < 4; channel++) {
      downscaled[destination + channel] = Math.round(sums[channel] / totalWeight);
    }
  }
}

const maxDelta = 255 * 255 * 3;
const pixelThreshold = 0.1;
let different = 0;
for (let index = 0; index < downscaled.length; index += 4) {
  const red = downscaled[index] - reference.data[index];
  const green = downscaled[index + 1] - reference.data[index + 1];
  const blue = downscaled[index + 2] - reference.data[index + 2];
  const delta = (red * red + green * green + blue * blue) / maxDelta;
  if (delta > pixelThreshold * pixelThreshold) different++;
}

console.log((100 * different / (width * height)).toFixed(3));
