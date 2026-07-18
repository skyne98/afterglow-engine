import createSteamAudio from '../dist/dynamic-steam-audio.js';
import { RING_BYTES, readFrame, writeFrame } from './shared-ring.ts';

interface DynamicSteamAudioModule {
  _dyn_init(triangles: number, sources: number, maxRays: number, maxBounces: number,
    reflectionType: number, maxDurationMs: number, maxOrder: number): number;
  _dyn_update(phase: number): number;
  _dyn_run_reflections(rays: number, bounces: number, durationMs: number, order: number): number;
  _dyn_run_audio(iterations: number): number;
  _dyn_run_binaural(iterations: number): number;
  _dyn_get_reverb_low(): number;
  _dyn_get_reverb_mid(): number;
  _dyn_get_reverb_high(): number;
  _dyn_get_ir_valid(): number;
  _dyn_get_output_energy(): number;
}

const request = new Uint8Array(40);
const response = new Uint8Array(40);
let memory: SharedArrayBuffer | null = null;
let steam: DynamicSteamAudioModule | null = null;

self.onmessage = async (event: MessageEvent): Promise<void> => {
  if (event.data instanceof SharedArrayBuffer) {
    memory = event.data;
    steam = await createSteamAudio({ locateFile: (path: string) => `${path}?v=13` }) as DynamicSteamAudioModule;
    self.postMessage('ready');
    return;
  }
  if (event.data !== 'wake' || memory === null || steam === null) return;
  const bytes = readFrame(memory, 0, request);
  if (bytes !== request.length) throw new Error(`unexpected dynamic request size ${bytes}`);
  const input = new DataView(request.buffer);
  const command = input.getUint32(0, true);
  const sequence = input.getUint32(4, true);
  let status = 0;
  const started = performance.now();
  if (command === 1) {
    status = steam._dyn_init(
      input.getUint32(8, true), input.getUint32(12, true),
      input.getUint32(16, true), input.getUint32(20, true),
      input.getUint32(24, true), input.getUint32(28, true),
      input.getUint32(32, true),
    );
  } else if (command === 2) {
    status = steam._dyn_update(input.getFloat32(8, true));
  } else if (command === 3) {
    status = steam._dyn_run_reflections(
      input.getUint32(8, true), input.getUint32(12, true),
      input.getUint32(16, true), input.getUint32(20, true),
    );
  } else if (command === 4) {
    status = steam._dyn_run_audio(input.getUint32(8, true));
  } else if (command === 5) {
    status = steam._dyn_run_binaural(input.getUint32(8, true));
  } else {
    status = 0xffff_ffff;
  }
  const elapsedMs = performance.now() - started;
  const output = new DataView(response.buffer);
  output.setUint32(0, sequence, true);
  output.setUint32(4, status, true);
  output.setFloat64(8, elapsedMs, true);
  output.setFloat32(16, steam._dyn_get_reverb_low(), true);
  output.setFloat32(20, steam._dyn_get_reverb_mid(), true);
  output.setFloat32(24, steam._dyn_get_reverb_high(), true);
  output.setUint32(28, steam._dyn_get_ir_valid(), true);
  output.setFloat32(32, steam._dyn_get_output_energy(), true);
  output.setUint32(36, command, true);
  writeFrame(memory, RING_BYTES, response);
  self.postMessage('wake');
};
