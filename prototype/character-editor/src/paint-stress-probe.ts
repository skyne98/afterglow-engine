import { PaintSession } from './paint-session.ts';

const canvas = document.querySelector<HTMLCanvasElement>('canvas')!;
const session = new PaintSession(canvas);
const host = globalThis as typeof globalThis & { __documentLoaded?: boolean; paintStressResult?: unknown };
let failure = '', ready = false, nextId = 0, state: any;
let pending: { id: number; resolve: (data: any) => void } | null = null;
session.onerror = event => { failure = event.message; };
session.onmessage = ({ data }) => {
  if (data.type === 'ready') ready = true;
  if (data.type === 'state') state = data.state;
  if (data.type === 'status' && /failed|rollback|rolled back|exceeded|capacity/i.test(data.text)) failure = data.text;
  if (data.type === 'probeResult' && pending !== null && pending.id === data.id) { const call = pending; pending = null; call.resolve(data); }
};
const wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
function send(message: any) { if (failure) throw new Error(failure); session.send(message); }
async function row(y: number, pixels = false): Promise<any> {
  const id = ++nextId;
  let timer: ReturnType<typeof setTimeout>;
  try {
    return await Promise.race([
      new Promise(resolve => { pending = { id, resolve }; send({ cmd: 'probe', id, y, pixels }); }),
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(failure || 'Paint probe deadline')), 120_000); }),
    ]);
  } finally { clearTimeout(timer!); pending = null; }
}
async function stroke(index: number) {
  const x = 512 + (index % 16) * 960, y = 512 + (Math.floor(index / 16) % 16) * 960;
  const common = { xtilt: 0, ytilt: 0, zoom: 1, rotation: 0, barrel: 0 };
  send({ cmd: 'beginStroke', ...common, x, y });
  for (let sample = 0; sample < 17; sample++) send({ cmd: 'strokeSample', ...common,
    x: x + sample * 32, y, pressure: 1, time: sample * 16 });
  send({ cmd: 'commit' });
  await row(y / 16384);
}
async function run() {
  const startup = performance.now();
  send({ cmd: 'init', recovery: 'discard', width: 16384, height: 16384,
    documentId: `paint-stress-${Date.now()}`, hardwareConcurrency: navigator.hardwareConcurrency });
  while (!ready) { if (failure || performance.now() - startup > 120_000) throw new Error(failure || 'Paint startup deadline'); await wait(20); }
  send({ cmd: 'setBackground', r: 0.5, g: 0.5, b: 0.5 });
  send({ cmd: 'config', settings: [['radius_logarithmic', Math.log(60)], ['hardness', 0.8],
    ['opaque', 1], ['opaque_multiply', 1], ['color_h', 0], ['color_s', 1], ['color_v', 0.8],
    ['radius_by_random', 0], ['offset_by_random', 0], ['tracking_noise', 0]] });
  const timings: number[] = [], checkpoints: unknown[] = [];
  const start = performance.now();
  let frameCount = 0, frameTotal = 0, frameMax = 0, below55 = 0, previousFrame = 0, captureFrames = true;
  function frame(time: number) {
    if (!captureFrames) return;
    if (previousFrame) { const delta = time - previousFrame; frameCount++; frameTotal += delta; frameMax = Math.max(frameMax, delta); if (delta > 1000 / 55) below55++; }
    previousFrame = time;
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
  for (let index = 0; index < 256; index++) {
    const before = performance.now(); await stroke(index); timings.push(performance.now() - before);
  }
  if (state.residentTiles <= 4096) throw new Error('16K traversal did not cross the initial tile block');
  const comparison: number[][] = [];
  for (let index = 0; index < 8; index++) comparison.push((await row((512 + index * 960) / 16384, true)).rgba);
  if (comparison.some(bytes => !bytes || bytes.length !== 4096 * 4)) throw new Error('Missing 16K comparison pixels');
  if (!comparison.some(bytes => bytes.some((red, offset) => offset % 4 === 0 && red > bytes[offset + 1] + 20)))
    throw new Error('16K traversal has no red paint');
  const lastRow = (512 + 15 * 960) / 16384;
  const painted = JSON.stringify((await row(lastRow, true)).rgba);
  send({ cmd: 'undo' });
  if (JSON.stringify((await row(lastRow, true)).rgba) === painted) throw new Error('16K undo did not remove paint');
  send({ cmd: 'redo' });
  if (JSON.stringify((await row(lastRow, true)).rgba) !== painted) throw new Error('16K redo pixel mismatch');
  let strokes = 256;
  // Ten minutes includes the initial traversal. One acknowledged stroke bounds input work.
  while (performance.now() - start < 600_000) {
    const before = performance.now(); await stroke(strokes % 256); strokes++;
    if (strokes % 64 === 0) {
      checkpoints.push({ elapsedMs: performance.now() - start, strokes, state });
      console.log('[paint-stress-progress]', JSON.stringify({ strokes, elapsedMs: performance.now() - start }));
    }
    await wait(Math.max(0, 100 - (performance.now() - before)));
  }
  await session.flush();
  captureFrames = false;
  timings.sort((a, b) => a - b);
  const result = { phase: 'stress', width: 16384, height: 16384, radius: 60, displayWidth: canvas.width,
    elapsedMs: performance.now() - start, strokes, comparison, checkpoints,
    frames: { count: frameCount, meanMs: frameTotal / frameCount, maxMs: frameMax, below55 },
    initialStrokeMs: { mean: timings.reduce((a, b) => a + b, 0) / timings.length, p99: timings[Math.floor(timings.length * 0.99)], max: timings.at(-1) }, state };
  host.paintStressResult = result;
  console.log('[paint-recovery-result]', JSON.stringify(result));
}
// Keep native module startup open only for module evaluation, not the ten-minute check.
setTimeout(() => { void run().catch(error => {
  host.paintStressResult = { error: String(error) };
  console.error('[paint-recovery-failed]', error instanceof Error ? error.stack : String(error));
}); }, 0);
