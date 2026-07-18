import { RING_BYTES, initializeRing, readFrame, writeFrame } from './shared-ring.ts';

interface Summary { mean: number; p50: number; p90: number; p99: number; max: number }
interface CallResult {
  status: number;
  internalMs: number;
  roundTripMs: number;
  reverb: readonly [number, number, number];
  irValid: boolean;
  outputEnergy: number;
  tracerNodes: number;
  tracerBuildMs: number;
  tracerOwnedBytes: number;
  simulationThreads: number;
  tracerLanes: number;
}
interface Scenario {
  name: string;
  triangles: number;
  sources: number;
  rays: number;
  bounces: number;
  durationMs: number;
  order: number;
  reflectionType: number;
  samples: number;
}
interface ScenarioResult extends Scenario {
  initializationMs: number;
  sceneUpdate: Summary;
  reflectionSimulation: Summary;
  simulationRoundTrip: Summary;
  reflectionQuantumMeanMs: number;
  binauralQuantumMeanMs: number;
  combinedAudioQuantumMeanMs: number;
  reverb: readonly [number, number, number];
  reverbLowRange: readonly [number, number];
  irValid: boolean;
  outputEnergy: number;
  tracerNodes: number;
  tracerBuildMs: number;
  tracerOwnedBytes: number;
  simulationThreads: number;
  tracerLanes: number;
}

const target = document.getElementById('output');
if (!target) throw new Error('missing dynamic prototype output');
const log = (message: string): void => { target.textContent += `${message}\n`; console.log(message); };
const memory = new SharedArrayBuffer(RING_BYTES * 2);
initializeRing(memory, 0);
initializeRing(memory, RING_BYTES);
const worker = new Worker('./dynamic-worker.js?v=15', { type: 'module' });
const request = new Uint8Array(40);
const response = new Uint8Array(72);
let sequence = 0;
let resolveWake: (() => void) | null = null;
worker.onmessage = (event: MessageEvent): void => {
  if (event.data === 'ready' || event.data === 'wake') resolveWake?.();
};
const waitWake = (): Promise<void> => new Promise(resolve => { resolveWake = resolve; });
worker.postMessage(memory);
await waitWake();
resolveWake = null;

async function call(command: number, write: (view: DataView) => void): Promise<CallResult> {
  const current = ++sequence;
  const input = new DataView(request.buffer);
  request.fill(0);
  input.setUint32(0, command, true);
  input.setUint32(4, current, true);
  write(input);
  writeFrame(memory, 0, request);
  const started = performance.now();
  const wake = waitWake();
  worker.postMessage('wake');
  await wake;
  resolveWake = null;
  const roundTripMs = performance.now() - started;
  const bytes = readFrame(memory, RING_BYTES, response);
  if (bytes !== response.length) throw new Error(`unexpected dynamic response size ${bytes}`);
  const output = new DataView(response.buffer);
  if (output.getUint32(0, true) !== current) throw new Error('dynamic response sequence mismatch');
  return {
    status: output.getUint32(4, true),
    internalMs: output.getFloat64(8, true),
    reverb: [output.getFloat32(16, true), output.getFloat32(20, true), output.getFloat32(24, true)],
    irValid: output.getUint32(28, true) !== 0,
    outputEnergy: output.getFloat32(32, true),
    tracerNodes: output.getUint32(36, true),
    tracerBuildMs: output.getFloat64(40, true),
    tracerOwnedBytes: output.getFloat64(48, true),
    simulationThreads: output.getUint32(60, true),
    tracerLanes: output.getUint32(64, true),
    roundTripMs,
  };
}

function summarize(values: Float64Array): Summary {
  const sorted = Array.from(values).sort((left, right) => left - right);
  const at = (fraction: number): number => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * fraction))] ?? 0;
  let total = 0;
  for (const value of values) total += value;
  return { mean: total / values.length, p50: at(0.5), p90: at(0.9), p99: at(0.99), max: sorted.at(-1) ?? 0 };
}

async function benchmark(scenario: Scenario): Promise<ScenarioResult> {
  const initialized = await call(1, view => {
    view.setUint32(8, scenario.triangles, true);
    view.setUint32(12, scenario.sources, true);
    view.setUint32(16, scenario.rays, true);
    view.setUint32(20, scenario.bounces, true);
    view.setUint32(24, scenario.reflectionType, true);
    view.setUint32(28, scenario.durationMs, true);
    view.setUint32(32, scenario.order, true);
  });
  if (initialized.status !== 0) throw new Error(`${scenario.name}: initialization failed ${initialized.status}`);
  for (let index = 0; index < 3; ++index) {
    const update = await call(2, view => view.setFloat32(8, index * 0.31, true));
    if (update.status !== 0) throw new Error(`${scenario.name}: warm-up update failed ${update.status}`);
    const simulation = await call(3, view => {
      view.setUint32(8, scenario.rays, true); view.setUint32(12, scenario.bounces, true);
      view.setUint32(16, scenario.durationMs, true); view.setUint32(20, scenario.order, true);
    });
    if (simulation.status !== 0) throw new Error(`${scenario.name}: warm-up simulation failed ${simulation.status}`);
  }
  const updates = new Float64Array(scenario.samples);
  const simulations = new Float64Array(scenario.samples);
  const roundTrips = new Float64Array(scenario.samples);
  let last: CallResult = initialized;
  let reverbLowMin = Number.POSITIVE_INFINITY;
  let reverbLowMax = Number.NEGATIVE_INFINITY;
  for (let index = 0; index < scenario.samples; ++index) {
    const phase = index * 0.173;
    const update = await call(2, view => view.setFloat32(8, phase, true));
    last = await call(3, view => {
      view.setUint32(8, scenario.rays, true); view.setUint32(12, scenario.bounces, true);
      view.setUint32(16, scenario.durationMs, true); view.setUint32(20, scenario.order, true);
    });
    if (update.status !== 0 || last.status !== 0)
      throw new Error(`${scenario.name}: dynamic step failed ${update.status}/${last.status}`);
    updates[index] = update.internalMs;
    simulations[index] = last.internalMs;
    roundTrips[index] = last.roundTripMs;
    reverbLowMin = Math.min(reverbLowMin, last.reverb[0]);
    reverbLowMax = Math.max(reverbLowMax, last.reverb[0]);
  }
  const audioIterations = scenario.reflectionType === 0 ? 100 : 1000;
  const audio = await call(4, view => view.setUint32(8, audioIterations, true));
  if (audio.status !== 0) throw new Error(`${scenario.name}: reflection audio failed ${audio.status}`);
  const binauralIterations = 100;
  const binaural = await call(5, view => view.setUint32(8, binauralIterations, true));
  if (binaural.status !== 0) throw new Error(`${scenario.name}: binaural audio failed ${binaural.status}`);
  const reflectionQuantumMeanMs = audio.internalMs / audioIterations;
  const binauralQuantumMeanMs = binaural.internalMs / binauralIterations;
  return {
    ...scenario,
    initializationMs: initialized.internalMs,
    sceneUpdate: summarize(updates),
    reflectionSimulation: summarize(simulations),
    simulationRoundTrip: summarize(roundTrips),
    reflectionQuantumMeanMs,
    binauralQuantumMeanMs,
    combinedAudioQuantumMeanMs: reflectionQuantumMeanMs + binauralQuantumMeanMs,
    reverb: last.reverb,
    reverbLowRange: [reverbLowMin, reverbLowMax],
    irValid: last.irValid,
    outputEnergy: audio.outputEnergy,
    tracerNodes: initialized.tracerNodes,
    tracerBuildMs: initialized.tracerBuildMs,
    tracerOwnedBytes: initialized.tracerOwnedBytes,
    simulationThreads: initialized.simulationThreads,
    tracerLanes: initialized.tracerLanes,
  };
}

const standardScenarios: Scenario[] = [
  { name: 'parametric-low', triangles: 10_000, sources: 1, rays: 1024, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 100 },
  { name: 'parametric-medium', triangles: 10_000, sources: 1, rays: 4096, bounces: 4, durationMs: 1000, order: 1, reflectionType: 1, samples: 100 },
  { name: 'parametric-high', triangles: 10_000, sources: 1, rays: 16_384, bounces: 8, durationMs: 1500, order: 1, reflectionType: 1, samples: 50 },
  { name: 'parametric-8-sources', triangles: 10_000, sources: 8, rays: 4096, bounces: 4, durationMs: 1000, order: 1, reflectionType: 1, samples: 50 },
  { name: 'parametric-32-sources', triangles: 10_000, sources: 32, rays: 4096, bounces: 4, durationMs: 1000, order: 1, reflectionType: 1, samples: 30 },
  { name: 'convolution-low', triangles: 10_000, sources: 1, rays: 1024, bounces: 2, durationMs: 500, order: 0, reflectionType: 0, samples: 50 },
  { name: 'convolution-medium', triangles: 10_000, sources: 1, rays: 4096, bounces: 4, durationMs: 1000, order: 1, reflectionType: 0, samples: 30 },
  { name: 'convolution-8-sources', triangles: 10_000, sources: 8, rays: 4096, bounces: 4, durationMs: 1000, order: 1, reflectionType: 0, samples: 20 },
];
const manySourceScenarios: Scenario[] = [
  { name: 'p16-512x1', triangles: 10_000, sources: 16, rays: 512, bounces: 1, durationMs: 250, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p16-1024x2', triangles: 10_000, sources: 16, rays: 1024, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p16-2048x2', triangles: 10_000, sources: 16, rays: 2048, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p32-256x1', triangles: 10_000, sources: 32, rays: 256, bounces: 1, durationMs: 250, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p32-512x1', triangles: 10_000, sources: 32, rays: 512, bounces: 1, durationMs: 250, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p32-512x2', triangles: 10_000, sources: 32, rays: 512, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p32-1024x1', triangles: 10_000, sources: 32, rays: 1024, bounces: 1, durationMs: 250, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p32-1024x2', triangles: 10_000, sources: 32, rays: 1024, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p32-2048x2', triangles: 10_000, sources: 32, rays: 2048, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p32-2048x4', triangles: 10_000, sources: 32, rays: 2048, bounces: 4, durationMs: 1000, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p64-256x1', triangles: 10_000, sources: 64, rays: 256, bounces: 1, durationMs: 250, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p64-512x1', triangles: 10_000, sources: 64, rays: 512, bounces: 1, durationMs: 250, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p64-256x2', triangles: 10_000, sources: 64, rays: 256, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p64-384x2', triangles: 10_000, sources: 64, rays: 384, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p64-512x2', triangles: 10_000, sources: 64, rays: 512, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p64-1024x1', triangles: 10_000, sources: 64, rays: 1024, bounces: 1, durationMs: 250, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p64-1024x2', triangles: 10_000, sources: 64, rays: 1024, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p64-2048x2', triangles: 10_000, sources: 64, rays: 2048, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p96-256x2', triangles: 10_000, sources: 96, rays: 256, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p96-512x2', triangles: 10_000, sources: 96, rays: 512, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p128-128x2', triangles: 10_000, sources: 128, rays: 128, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p128-256x2', triangles: 10_000, sources: 128, rays: 256, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'p128-512x2', triangles: 10_000, sources: 128, rays: 512, bounces: 2, durationMs: 500, order: 0, reflectionType: 1, samples: 30 },
  { name: 'c32-256x1-o0', triangles: 10_000, sources: 32, rays: 256, bounces: 1, durationMs: 250, order: 0, reflectionType: 0, samples: 20 },
  { name: 'c32-512x2-o0', triangles: 10_000, sources: 32, rays: 512, bounces: 2, durationMs: 500, order: 0, reflectionType: 0, samples: 20 },
  { name: 'c32-512x2-o1', triangles: 10_000, sources: 32, rays: 512, bounces: 2, durationMs: 500, order: 1, reflectionType: 0, samples: 20 },
  { name: 'c64-256x1-o0', triangles: 10_000, sources: 64, rays: 256, bounces: 1, durationMs: 250, order: 0, reflectionType: 0, samples: 20 },
  { name: 'c64-512x2-o0', triangles: 10_000, sources: 64, rays: 512, bounces: 2, durationMs: 500, order: 0, reflectionType: 0, samples: 20 },
];
const search = new URLSearchParams(location.search);
const selectedScenarios = search.has('many') ? manySourceScenarios : standardScenarios;
const only = search.get('only');
const scenarios = only === null
  ? selectedScenarios
  : selectedScenarios.filter(scenario => scenario.name === only);
if (scenarios.length === 0) throw new Error(`unknown Steam Audio scenario ${only}`);
const results: ScenarioResult[] = [];
for (const scenario of scenarios) {
  log(`running ${scenario.name}…`);
  const result = await benchmark(scenario);
  results.push(result);
  log(JSON.stringify(result));
}
(globalThis as typeof globalThis & { __steamAudioDynamicResults?: ScenarioResult[] }).__steamAudioDynamicResults = results;
log(`DYNAMIC_RESULTS ${JSON.stringify(results)}`);
log('Steam Audio fully dynamic WASM benchmark complete');
