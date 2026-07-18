import { RING_BYTES, initializeRing, readFrame, writeFrame } from './shared-ring.ts';

interface Summary { mean: number; p50: number; p90: number; p99: number; max: number }
interface CallResult {
  status: number;
  internalMs: number;
  roundTripMs: number;
  initializationMs: number;
  vertices: number;
  triangles: number;
  tracerNodes: number;
  tracerBuildMs: number;
  tracerOwnedBytes: number;
  reverbLow: number;
  irValid: boolean;
  simulationThreads: number;
  tracerLanes: number;
  geometryBytes: number;
}
interface ScenarioResult {
  rays: number;
  bounces: 2;
  samples: number;
  reflectionSimulation: Summary;
  simulationRoundTrip: Summary;
  reverbLowRange: readonly [number, number];
  irValid: boolean;
}
interface BistroResult {
  scene: string;
  initializationMs: number;
  geometryBytes: number;
  vertices: number;
  triangles: number;
  tracerNodes: number;
  tracerBuildMs: number;
  tracerOwnedBytes: number;
  simulationThreads: number;
  tracerLanes: number;
  scenarios: ScenarioResult[];
}

const allowedScenes = new Set(['BistroExterior', 'BistroInterior', 'BistroInterior_Wine']);
const scene = new URLSearchParams(location.search).get('scene') ?? 'BistroInterior';
if (!allowedScenes.has(scene)) throw new Error(`invalid Bistro scene ${scene}`);
const target = document.getElementById('output');
if (!target) throw new Error('missing Bistro output');
const log = (message: string): void => { target.textContent += `${message}\n`; console.log(message); };
globalThis.addEventListener('unhandledrejection', event => {
  log(`FAILED: ${event.reason instanceof Error ? event.reason.message : String(event.reason)}`);
});

const rings = new SharedArrayBuffer(RING_BYTES * 2);
initializeRing(rings, 0);
initializeRing(rings, RING_BYTES);
const worker = new Worker(`./bistro-worker.js?scene=${scene}&v=2`, { type: 'module' });
const request = new Uint8Array(16);
const response = new Uint8Array(80);
let sequence = 0;
let resolveWake: (() => void) | null = null;
let rejectWake: ((reason: Error) => void) | null = null;
worker.onmessage = (event: MessageEvent): void => {
  if (event.data === 'ready' || event.data === 'wake') resolveWake?.();
};
worker.onerror = (event: ErrorEvent): void => rejectWake?.(new Error(event.message));
const waitWake = (): Promise<void> => new Promise((resolve, reject) => {
  resolveWake = resolve;
  rejectWake = reject;
});
worker.postMessage(rings);
log(`loading full ${scene} geometry…`);
await waitWake();
resolveWake = null;
rejectWake = null;

function readResult(expectedSequence: number, roundTripMs: number): CallResult {
  const bytes = readFrame(rings, RING_BYTES, response);
  if (bytes !== response.length) throw new Error(`unexpected Bistro response size ${bytes}`);
  const output = new DataView(response.buffer);
  const actualSequence = output.getUint32(0, true);
  if (actualSequence !== expectedSequence)
    throw new Error(`Bistro response sequence mismatch: expected ${expectedSequence}, received ${actualSequence}`);
  return {
    status: output.getUint32(4, true),
    internalMs: output.getFloat64(8, true),
    initializationMs: output.getFloat64(16, true),
    vertices: output.getUint32(24, true),
    triangles: output.getUint32(28, true),
    tracerNodes: output.getUint32(32, true),
    tracerBuildMs: output.getFloat64(40, true),
    tracerOwnedBytes: output.getFloat64(48, true),
    reverbLow: output.getFloat32(56, true),
    irValid: output.getUint32(60, true) !== 0,
    simulationThreads: output.getUint32(64, true),
    tracerLanes: output.getUint32(68, true),
    geometryBytes: output.getUint32(72, true),
    roundTripMs,
  };
}
const initialized = readResult(0, 0);
if (initialized.status !== 0)
  throw new Error(`${scene} initialization failed ${initialized.status} after ${initialized.initializationMs.toFixed(1)} ms`);

async function run(rays: number, phase: number): Promise<CallResult> {
  const current = ++sequence;
  request.fill(0);
  const input = new DataView(request.buffer);
  input.setUint32(0, 1, true);
  input.setUint32(4, current, true);
  input.setUint32(8, rays, true);
  input.setFloat32(12, phase, true);
  writeFrame(rings, 0, request);
  const started = performance.now();
  const wake = waitWake();
  worker.postMessage('wake');
  await wake;
  resolveWake = null;
  rejectWake = null;
  return readResult(current, performance.now() - started);
}

function summarize(values: Float64Array): Summary {
  const sorted = Array.from(values).sort((left, right) => left - right);
  const at = (fraction: number): number => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))] ?? 0;
  let total = 0;
  for (const value of values) total += value;
  return { mean: total / values.length, p50: at(0.5), p90: at(0.9), p99: at(0.99), max: sorted.at(-1) ?? 0 };
}

async function benchmark(rays: number): Promise<{ scenario: ScenarioResult; last: CallResult }> {
  for (let index = 0; index < 3; ++index) {
    const warmup = await run(rays, index * 0.31);
    if (warmup.status !== 0) throw new Error(`Bistro warm-up failed ${warmup.status}`);
  }
  const samples = 30;
  const internal = new Float64Array(samples);
  const roundTrips = new Float64Array(samples);
  let reverbLowMin = Number.POSITIVE_INFINITY;
  let reverbLowMax = Number.NEGATIVE_INFINITY;
  let allIrValid = true;
  let last = await run(rays, 0);
  for (let index = 0; index < samples; ++index) {
    last = await run(rays, index * 0.173);
    if (last.status !== 0) throw new Error(`Bistro simulation failed ${last.status}`);
    internal[index] = last.internalMs;
    roundTrips[index] = last.roundTripMs;
    reverbLowMin = Math.min(reverbLowMin, last.reverbLow);
    reverbLowMax = Math.max(reverbLowMax, last.reverbLow);
    allIrValid = allIrValid && last.irValid;
  }
  return {
    scenario: {
      rays,
      bounces: 2,
      samples,
      reflectionSimulation: summarize(internal),
      simulationRoundTrip: summarize(roundTrips),
      reverbLowRange: [reverbLowMin, reverbLowMax],
      irValid: allIrValid,
    },
    last,
  };
}

const scenarios: ScenarioResult[] = [];
let telemetry: CallResult | null = null;
for (const rays of [512, 1024]) {
  log(`running 64-source ${rays}×2…`);
  const measured = await benchmark(rays);
  scenarios.push(measured.scenario);
  telemetry = measured.last;
}
if (telemetry === null) throw new Error('missing Bistro telemetry');
const result: BistroResult = {
  scene,
  initializationMs: telemetry.initializationMs,
  geometryBytes: telemetry.geometryBytes,
  vertices: telemetry.vertices,
  triangles: telemetry.triangles,
  tracerNodes: telemetry.tracerNodes,
  tracerBuildMs: telemetry.tracerBuildMs,
  tracerOwnedBytes: telemetry.tracerOwnedBytes,
  simulationThreads: telemetry.simulationThreads,
  tracerLanes: telemetry.tracerLanes,
  scenarios,
};
(globalThis as typeof globalThis & { __steamAudioBistroResults?: BistroResult }).__steamAudioBistroResults = result;
log(`BISTRO_WASM_RESULTS ${JSON.stringify(result)}`);
log('Full Bistro WASM benchmark complete');
