import createWorkletGate from '../dist/engine-audio-worklet-gate.js';

interface WorkletGateModule {
  _afterglow_steam_audio_init(triangles: number, voices: number, reflectionVoices: number,
    rays: number, bounces: number, durationMs: number, order: number): number;
  _afterglow_steam_audio_set_active_reflection_voices(voices: number): number;
  _afterglow_steam_audio_run_simulation(): number;
  _afterglow_steam_audio_shutdown(): void;
  _afterglow_worklet_gate_create(): number;
  _afterglow_worklet_gate_start_simulation(): number;
  _afterglow_worklet_gate_stop_simulation(): number;
  _afterglow_worklet_gate_resume(): number;
  _afterglow_worklet_gate_status(): number;
  _afterglow_worklet_gate_callbacks(): number;
  _afterglow_worklet_gate_errors(): number;
  _afterglow_worklet_gate_max_micros(): number;
  _afterglow_worklet_gate_over_budget(): number;
  _afterglow_worklet_gate_max_gap_micros(): number;
  _afterglow_worklet_gate_simulation_updates(): number;
  _afterglow_worklet_gate_reflection_updates(): number;
  _afterglow_worklet_gate_simulation_errors(): number;
  _afterglow_worklet_gate_simulation_max_micros(): number;
  _afterglow_worklet_gate_simulation_running(): number;
  _afterglow_worklet_gate_energy(): number;
  _afterglow_worklet_gate_peak(): number;
}

const output = document.getElementById('output');
const start = document.getElementById('start');
if (!(output instanceof HTMLElement) || !(start instanceof HTMLButtonElement))
  throw new Error('missing worklet gate controls');

let gate: WorkletGateModule | null = null;
const parameters = new URLSearchParams(location.search);
const wetVoices = Math.min(64, Math.max(0,
  Number(parameters.get('wet') ?? 16) | 0,
));
const concurrentSimulation = parameters.get('simulation') !== 'off';
let gpuFrames = 0;
let rafLast = 0;
let rafMax = 0;
let rafTotal = 0;
let rafSamples = 0;

function renderFrame(now: number): void {
  if (rafLast !== 0) {
    const elapsed = now - rafLast;
    rafTotal += elapsed;
    rafMax = Math.max(rafMax, elapsed);
    ++rafSamples;
  }
  rafLast = now;
  ++gpuFrames;
  requestAnimationFrame(renderFrame);
}
requestAnimationFrame(renderFrame);

async function startGpuLoad(): Promise<void> {
  const adapter = await navigator.gpu?.requestAdapter();
  if (adapter === null || adapter === undefined) throw new Error('hardware WebGPU unavailable');
  const device = await adapter.requestDevice();
  const canvas = document.createElement('canvas');
  canvas.width = 1200;
  canvas.height = 800;
  document.body.append(canvas);
  const context = canvas.getContext('webgpu');
  if (context === null) throw new Error('WebGPU canvas context unavailable');
  context.configure({ device, format: navigator.gpu.getPreferredCanvasFormat() });
  const submit = (): void => {
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginRenderPass({ colorAttachments: [{
      view: context.getCurrentTexture().createView(),
      clearValue: { r: 0.018, g: 0.026, b: 0.038, a: 1 },
      loadOp: 'clear', storeOp: 'store',
    }] });
    pass.end();
    device.queue.submit([encoder.finish()]);
    requestAnimationFrame(submit);
  };
  requestAnimationFrame(submit);
}

async function bootstrap(): Promise<void> {
  const module = await createWorkletGate({
    locateFile: (path: string): string => path,
  }) as WorkletGateModule;
  const initialized = module._afterglow_steam_audio_init(10_000, 128, 64, 512, 2, 500, 0);
  if (initialized !== 0) throw new Error(`Steam Audio initialization failed: ${initialized}`);
  const admitted = module._afterglow_steam_audio_set_active_reflection_voices(wetVoices);
  if (admitted !== 0) throw new Error(`Steam Audio wet admission failed: ${admitted}`);
  const simulated = module._afterglow_steam_audio_run_simulation();
  if (simulated !== 0) throw new Error(`Steam Audio simulation failed: ${simulated}`);
  const created = module._afterglow_worklet_gate_create();
  if (created !== 0) throw new Error(`Wasm AudioWorklet creation failed: ${created}`);
  const deadline = performance.now() + 10_000;
  while (module._afterglow_worklet_gate_status() !== 3) {
    const status = module._afterglow_worklet_gate_status() >>> 0;
    if ((status & 0x80000000) !== 0) throw new Error(`Wasm AudioWorklet setup failed: ${status}`);
    if (performance.now() >= deadline) throw new Error('Wasm AudioWorklet setup timed out');
    await new Promise<void>(resolve => setTimeout(resolve, 10));
  }
  if (concurrentSimulation) {
    const simulationStarted = module._afterglow_worklet_gate_start_simulation();
    if (simulationStarted !== 0)
      throw new Error(`simulation worker startup failed: ${simulationStarted}`);
  }
  gate = module;
  start.disabled = false;
  output.textContent = 'ready — press Start';
}

start.addEventListener('click', (): void => {
  if (gate === null) return;
  const status = gate._afterglow_worklet_gate_resume();
  if (status !== 0) {
    output.textContent = `resume failed: ${status}`;
    return;
  }
  start.disabled = true;
});

setInterval((): void => {
  if (gate === null) return;
  output.textContent = JSON.stringify({
    backend: 'Steam Audio Emscripten Wasm AudioWorklet',
    status: gate._afterglow_worklet_gate_status(),
    callbacks: gate._afterglow_worklet_gate_callbacks(),
    callbackErrors: gate._afterglow_worklet_gate_errors(),
    callbackMaxMs: gate._afterglow_worklet_gate_max_micros() / 1_000,
    callbackOverBudget: gate._afterglow_worklet_gate_over_budget(),
    callbackMaxGapMs: gate._afterglow_worklet_gate_max_gap_micros() / 1_000,
    concurrentSimulation,
    simulationRunning: gate._afterglow_worklet_gate_simulation_running() !== 0,
    simulationUpdates: gate._afterglow_worklet_gate_simulation_updates(),
    reflectionUpdates: gate._afterglow_worklet_gate_reflection_updates(),
    simulationErrors: gate._afterglow_worklet_gate_simulation_errors(),
    simulationMaxMs: gate._afterglow_worklet_gate_simulation_max_micros() / 1_000,
    outputEnergy: gate._afterglow_worklet_gate_energy(),
    outputPeak: gate._afterglow_worklet_gate_peak(),
    gpuFrames,
    rafMeanMs: rafSamples === 0 ? 0 : rafTotal / rafSamples,
    rafMaxMs: rafMax,
    activeReflectionVoices: wetVoices,
  }, null, 2);
}, 250);

void Promise.all([bootstrap(), startGpuLoad()]).catch((error: unknown): void => {
  output.textContent = `FATAL: ${String((error as Error).message ?? error)}`;
});
