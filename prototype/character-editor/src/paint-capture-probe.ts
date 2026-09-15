import './paint-main.ts';

declare global {
  var afterglowProfiling: { stats(): { connected: boolean; failures: number; batches: number; bytes: number } };
}
const page = window as typeof window & {
  probe(y: number): number;
  __probeResult?: { id: number; native?: boolean; colored?: number; painted: number };
  __paintCaptureReady?: unknown;
  __paintCaptureResult?: unknown;
  __paintCaptureError?: string;
  __documentLoaded?: boolean;
};
const wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
async function pixels() {
  const id = page.probe(0.5), deadline = performance.now() + 30_000;
  while (page.__probeResult?.id !== id) {
    if (performance.now() > deadline) throw new Error('Paint capture pixel deadline');
    await wait(10);
  }
  const result = page.__probeResult;
  return { native: result.native === true, colored: result.colored ?? result.painted };
}
function press(id: string) { (document.getElementById(id) as HTMLButtonElement).click(); }
async function workload() {
  const startMs = performance.now();
  const times = new Float64Array(240);
  let index = -1, previous = 0;
  await new Promise<void>(resolve => {
    function frame(now: number) {
      if (index >= 0) times[index] = now - previous;
      previous = now;
      if (++index === times.length) { resolve(); return; }
      if (index % 60 === 0) press('strokeBtn');
      requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);
  });
  const output = await pixels();
  times.sort();
  let sum = 0;
  for (const time of times) sum += time;
  return { startMs, endMs: performance.now(), mean: sum / times.length, p99: times[237], max: times[239], colored: output.colored };
}
const runtimeTicks = new Float64Array(4096);
let runtimeTickCount = 0, runtimeTickOverflow = 0;
let runtimeTimer: ReturnType<typeof setTimeout> | undefined;
let runtimeDue = 0, runtimeMaxLateMs = 0;
function runtimeTick() {
  const now = performance.now();
  runtimeMaxLateMs = Math.max(runtimeMaxLateMs, now - runtimeDue);
  if (runtimeTickCount < runtimeTicks.length) runtimeTicks[runtimeTickCount++] = now;
  else runtimeTickOverflow++;
  runtimeDue = now + 25;
  runtimeTimer = setTimeout(runtimeTick, 25);
}
async function runCapture() {
try {
  const documentDeadline = performance.now() + 30_000;
  while (page.__documentLoaded === false) {
    if (performance.now() > documentDeadline) throw new Error('Native document startup deadline');
    await wait(25);
  }
  const initial = await pixels();
  const native = new URL(location.href).searchParams.get('target') !== 'web';
  if (initial.native !== native) throw new Error('Incorrect paint worker target');
  const startupDeadline = performance.now() + 30_000;
  while (!globalThis.afterglowProfiling) {
    if (performance.now() > startupDeadline) throw new Error('Profiling startup deadline');
    await wait(25);
  }
  press('strokeBtn');
  await pixels();
  press('undoBtn');
  await pixels();
  const baseline = await workload();
  const recording = document.querySelector('meta[name="paint-capture-mode"]')?.getAttribute('content') !== 'off';
  const readyMs = performance.now();
  runtimeDue = readyMs + 25;
  runtimeTimer = setTimeout(runtimeTick, 25);
  page.__paintCaptureReady = baseline;
  console.log('[paint-capture-ready]', JSON.stringify(baseline));
  const deadline = performance.now() + 30_000;
  while (recording && !afterglowProfiling.stats().connected) {
    if (performance.now() > deadline) throw new Error('Viewer connection deadline');
    await wait(25);
  }
  // Use the same phase times and paint history in fresh capture and control processes.
  await wait(Math.max(250, readyMs + 2000 - performance.now()));
  const capture = await workload();
  await wait(500);
  const stats = afterglowProfiling.stats();
  if (capture.colored <= initial.colored || stats.connected !== recording || stats.failures
      || (recording ? stats.batches < 2 : stats.batches !== 0))
    throw new Error(`Invalid paint capture: ${JSON.stringify({ initial, baseline, capture, stats })}`);
  const disconnectDeadline = performance.now() + 30_000;
  while (afterglowProfiling.stats().connected) {
    if (performance.now() > disconnectDeadline) throw new Error('Viewer disconnect deadline');
    await wait(25);
  }
  await wait(Math.max(0, readyMs + 20_000 - performance.now()));
  const baselineAfter = await workload();
  const runtimeProgress = { intervalMs: 25, startMs: readyMs, maxLateMs: runtimeMaxLateMs,
    overflow: runtimeTickOverflow, ticks: Array.from(runtimeTicks.subarray(0, runtimeTickCount)) };
  page.__paintCaptureResult = { baseline, capture, baselineAfter, stats, recording, runtimeProgress, target: native ? 'native' : 'web' };
  console.log('[paint-capture-result]', JSON.stringify(page.__paintCaptureResult));
} catch (error) {
  page.__paintCaptureError = error instanceof Error ? error.stack : String(error);
  console.error('[paint-capture-failed]', page.__paintCaptureError);
} finally {
  clearTimeout(runtimeTimer);
}
}
// Complete module evaluation before the capture so native input can start.
void runCapture();
