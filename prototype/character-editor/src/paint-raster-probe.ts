const host = globalThis as typeof globalThis & { __documentLoaded?: boolean };
const canvas = document.getElementById('raster') as HTMLCanvasElement;
const context = canvas.getContext('2d') as CanvasRenderingContext2D & { commit(x?: number, y?: number, width?: number, height?: number): void };
if (typeof context.commit !== 'function') throw new Error('Raster check needs the native Canvas2D interface');
const nextFrame = () => new Promise<number>(resolve => requestAnimationFrame(resolve));
function summary(values: Float64Array) {
  values.sort();
  let sum = 0;
  for (const value of values) sum += value;
  return { meanMs: sum / values.length, p99Ms: values[Math.ceil(values.length * 0.99) - 1], maxMs: values[values.length - 1] };
}
async function run() {
  const deadline = performance.now() + 30_000;
  while (!host.__documentLoaded) {
    if (performance.now() >= deadline) throw new Error('Raster startup deadline');
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  const results = [];
  const patch = new ImageData(64, 64);
  for (let i = 0; i < patch.data.length; i += 4) { patch.data[i] = 255; patch.data[i + 3] = 255; }
  for (const dimension of [512, 2048, 4096]) {
    canvas.width = canvas.height = dimension;
    context.fillStyle = '#20242b';
    context.fillRect(0, 0, dimension, dimension);
    context.commit();
    const copy = new Float64Array(120), commit = new Float64Array(120), frames = new Float64Array(120);
    let previous = await nextFrame();
    for (let index = -20; index < 120; index++) {
      const start = performance.now();
      context.putImageData(patch, 64, 64);
      const copied = performance.now();
      context.commit(64, 64, 64, 64);
      const published = performance.now();
      const now = await nextFrame();
      if (index >= 0) { copy[index] = copied - start; commit[index] = published - copied; frames[index] = now - previous; }
      previous = now;
    }
    const untouched = context.getImageData(0, 0, 1, 1).data;
    const changed = context.getImageData(64, 64, 1, 1).data;
    if (untouched.join(',') !== '32,36,43,255' || changed.join(',') !== '255,0,0,255')
      throw new Error('Raster benchmark changed pixels outside its patch');
    results.push({ dimension, changedBytes: patch.data.byteLength, requestedBytes: patch.data.byteLength,
      copy: summary(copy), commit: summary(commit), raf: summary(frames) });
  }
  console.log('[paint-recovery-result]', JSON.stringify({ phase: 'raster', results,
    limits: 'Commit is synchronous CPU time. rAF is frame production, not GPU time or input-to-present latency.' }));
}
// Keep native startup separate from frame measurements.
void run().catch(error => console.error('[paint-recovery-failed]', error instanceof Error ? error.stack : String(error)));
