import { RING_BYTES, initializeRing, readFrame, writeFrame } from './shared-ring.ts';

interface SampleResult {
  sequence: number;
  status: number;
  internalMs: number;
  occlusion: number;
  transmission: readonly [number, number, number];
  roundTripMs: number;
}

interface Summary {
  mean: number;
  p50: number;
  p90: number;
  p99: number;
  max: number;
}

interface ScenarioResult {
  triangles: number;
  sources: number;
  initializationMs: number;
  batchedOccludedMeanMs: number;
  batchedVisibleMeanMs: number;
  internal: Summary;
  ringRoundTrip: Summary;
  occluded: number;
  visible: number;
  transmission: readonly [number, number, number];
}

const output = document.getElementById('output');
if (!output) throw new Error('missing prototype output');
const log = (message: string): void => {
  output.textContent += `${message}\n`;
  console.log(message);
};

const memory = new SharedArrayBuffer(RING_BYTES * 2);
initializeRing(memory, 0);
initializeRing(memory, RING_BYTES);
const worker = new Worker('./worker.js?v=4', { type: 'module' });
const response = new Uint8Array(32);
const request = new Uint8Array(16);
let sequence = 0;
let resolveWake: (() => void) | null = null;
worker.onmessage = (event: MessageEvent): void => {
  if (event.data === 'ready' || event.data === 'wake') resolveWake?.();
};
const waitWake = (): Promise<void> => new Promise(resolve => { resolveWake = resolve; });
worker.postMessage(memory);
await waitWake();
resolveWake = null;

async function call(command: number, value0: number, value1: number): Promise<SampleResult> {
  const current = ++sequence;
  const input = new DataView(request.buffer);
  input.setUint32(0, command, true);
  input.setUint32(4, current, true);
  input.setUint32(8, value0, true);
  input.setUint32(12, value1, true);
  writeFrame(memory, 0, request);
  const started = performance.now();
  const wake = waitWake();
  worker.postMessage('wake');
  await wake;
  resolveWake = null;
  const elapsed = performance.now() - started;
  const bytes = readFrame(memory, RING_BYTES, response);
  if (bytes !== 32) throw new Error(`unexpected response size ${bytes}`);
  const result = new DataView(response.buffer);
  if (result.getUint32(0, true) !== current) throw new Error('response sequence mismatch');
  return {
    sequence: current,
    status: result.getUint32(4, true),
    internalMs: result.getFloat64(8, true),
    occlusion: result.getFloat32(16, true),
    transmission: [
      result.getFloat32(20, true), result.getFloat32(24, true), result.getFloat32(28, true),
    ],
    roundTripMs: elapsed,
  };
}

function summarize(values: Float64Array): Summary {
  const sorted = Array.from(values).sort((left, right) => left - right);
  const percentile = (fraction: number): number => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))] ?? 0;
  let total = 0;
  for (const value of values) total += value;
  return {
    mean: total / values.length,
    p50: percentile(0.50),
    p90: percentile(0.90),
    p99: percentile(0.99),
    max: sorted[sorted.length - 1] ?? 0,
  };
}

async function benchmark(triangles: number, sources: number, samples = 2000): Promise<ScenarioResult> {
  const initialized = await call(1, triangles, sources);
  if (initialized.status !== 0) throw new Error(`Steam Audio initialization failed: ${initialized.status}`);
  for (let index = 0; index < 100; index++) await call(2, index & 1, 0);
  const internal = new Float64Array(samples);
  const roundTrip = new Float64Array(samples);
  let occluded = 0;
  let visible = 0;
  let transmission: readonly [number, number, number] = [0, 0, 0];
  for (let index = 0; index < samples; index++) {
    const result = await call(2, index & 1, 0);
    internal[index] = result.internalMs;
    roundTrip[index] = result.roundTripMs;
    if (index & 1) {
      occluded = result.occlusion;
      transmission = result.transmission;
    } else {
      visible = result.occlusion;
    }
  }
  const batchedOccluded = await call(3, 10_000, 1);
  const batchedVisible = await call(3, 10_000, 0);
  return {
    triangles,
    sources,
    initializationMs: initialized.internalMs,
    batchedOccludedMeanMs: batchedOccluded.internalMs,
    batchedVisibleMeanMs: batchedVisible.internalMs,
    internal: summarize(internal),
    ringRoundTrip: summarize(roundTrip),
    occluded,
    visible,
    transmission,
  };
}

const scenarios: Array<readonly [number, number]> = [
  [24, 1],
  [10_000, 1],
  [100_000, 1],
  [10_000, 32],
  [10_000, 128],
];
const results: ScenarioResult[] = [];
for (const [triangles, sources] of scenarios) {
  log(`running ${triangles} triangles × ${sources} sources…`);
  const result = await benchmark(triangles, sources);
  results.push(result);
  log(JSON.stringify(result));
}
(globalThis as typeof globalThis & { __steamAudioWasmResults?: ScenarioResult[] }).__steamAudioWasmResults = results;
log(`RESULTS ${JSON.stringify(results)}`);
log('Steam Audio WASM benchmark complete');
