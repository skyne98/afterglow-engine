import createSteamAudio from '../dist/steam-audio.js';
import { RING_BYTES, readFrame, writeFrame } from './shared-ring.ts';

interface SteamAudioModule {
  _sa_init(triangleCount: number, sourceCount: number): number;
  _sa_set_occluded(occluded: number): void;
  _sa_run_direct(): void;
  _sa_run_direct_batch(iterations: number): void;
  _sa_get_occlusion(): number;
  _sa_get_transmission_low(): number;
  _sa_get_transmission_mid(): number;
  _sa_get_transmission_high(): number;
  _sa_shutdown(): void;
}

const request = new Uint8Array(32);
const response = new Uint8Array(32);
let memory: SharedArrayBuffer | null = null;
let steam: SteamAudioModule | null = null;

self.onmessage = async (event: MessageEvent): Promise<void> => {
  if (event.data instanceof SharedArrayBuffer) {
    memory = event.data;
    steam = await createSteamAudio() as SteamAudioModule;
    self.postMessage('ready');
    return;
  }
  if (event.data !== 'wake' || memory === null || steam === null) return;

  const requestBytes = readFrame(memory, 0, request);
  if (requestBytes !== 16) throw new Error(`unexpected request size ${requestBytes}`);
  const input = new DataView(request.buffer, request.byteOffset, requestBytes);
  const command = input.getUint32(0, true);
  const sequence = input.getUint32(4, true);
  let status = 0;
  let elapsedMs = 0;

  if (command === 1) {
    const started = performance.now();
    status = steam._sa_init(input.getUint32(8, true), input.getUint32(12, true));
    elapsedMs = performance.now() - started;
  } else if (command === 2) {
    steam._sa_set_occluded(input.getUint32(8, true));
    const started = performance.now();
    steam._sa_run_direct();
    elapsedMs = performance.now() - started;
  } else if (command === 3) {
    const iterations = input.getUint32(8, true);
    steam._sa_set_occluded(input.getUint32(12, true));
    const started = performance.now();
    steam._sa_run_direct_batch(iterations);
    elapsedMs = (performance.now() - started) / iterations;
  } else {
    status = 0xffff_ffff;
  }

  const output = new DataView(response.buffer);
  output.setUint32(0, sequence, true);
  output.setUint32(4, status, true);
  output.setFloat64(8, elapsedMs, true);
  output.setFloat32(16, steam._sa_get_occlusion(), true);
  output.setFloat32(20, steam._sa_get_transmission_low(), true);
  output.setFloat32(24, steam._sa_get_transmission_mid(), true);
  output.setFloat32(28, steam._sa_get_transmission_high(), true);
  writeFrame(memory, RING_BYTES, response);
  self.postMessage('wake');
};
