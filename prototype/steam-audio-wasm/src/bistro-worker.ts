import createSteamAudio from '../dist/bistro-steam-audio.js';
import { RING_BYTES, readFrame, writeFrame } from './shared-ring.ts';

interface BistroSteamAudioModule {
  HEAPU8: Uint8Array;
  _malloc(bytes: number): number;
  _free(pointer: number): void;
  _bistro_init(pointer: number, bytes: number): number;
  _bistro_run_reflections(rays: number, phase: number): number;
  _bistro_get_vertices(): number;
  _bistro_get_triangles(): number;
  _bistro_get_tracer_nodes(): number;
  _bistro_get_tracer_build_ms(): number;
  _bistro_get_tracer_owned_bytes(): number;
  _bistro_get_reverb_low(): number;
  _bistro_get_ir_valid(): number;
  _bistro_get_simulation_threads(): number;
  _bistro_get_tracer_lanes(): number;
  _bistro_shutdown(): void;
}

const allowedScenes = new Set(['BistroExterior', 'BistroInterior', 'BistroInterior_Wine']);
const scene = new URL(self.location.href).searchParams.get('scene') ?? '';
if (!allowedScenes.has(scene)) throw new Error(`invalid Bistro scene ${scene}`);

const request = new Uint8Array(16);
const response = new Uint8Array(80);
let rings: SharedArrayBuffer | null = null;
let steam: BistroSteamAudioModule | null = null;
let initializationMs = 0;
let geometryBytes = 0;

self.onmessage = async (event: MessageEvent): Promise<void> => {
  if (event.data instanceof SharedArrayBuffer) {
    rings = event.data;
    const started = performance.now();
    let status = 0;
    try {
      steam = await createSteamAudio({ locateFile: (path: string) => `${path}?v=2` }) as BistroSteamAudioModule;
      const asset = new Uint8Array(await (await fetch(`./assets/${scene}.acoustic.bin`)).arrayBuffer());
      geometryBytes = asset.byteLength;
      const pointer = steam._malloc(asset.byteLength);
      if (pointer === 0) throw new Error(`failed to allocate ${asset.byteLength} Bistro bytes`);
      steam.HEAPU8.set(asset, pointer);
      status = steam._bistro_init(pointer, asset.byteLength);
      steam._free(pointer);
    } catch (error) {
      console.error(error);
      status = 200;
    }
    initializationMs = performance.now() - started;
    const output = new DataView(response.buffer);
    response.fill(0);
    output.setUint32(4, status, true);
    output.setFloat64(16, initializationMs, true);
    if (status === 0 && steam !== null) {
      output.setUint32(24, steam._bistro_get_vertices(), true);
      output.setUint32(28, steam._bistro_get_triangles(), true);
      output.setUint32(32, steam._bistro_get_tracer_nodes(), true);
      output.setFloat64(40, steam._bistro_get_tracer_build_ms(), true);
      output.setFloat64(48, steam._bistro_get_tracer_owned_bytes(), true);
      output.setUint32(64, steam._bistro_get_simulation_threads(), true);
      output.setUint32(68, steam._bistro_get_tracer_lanes(), true);
      output.setUint32(72, geometryBytes, true);
    }
    writeFrame(rings, RING_BYTES, response);
    self.postMessage('wake');
    return;
  }
  if (event.data !== 'wake' || rings === null || steam === null) return;
  const bytes = readFrame(rings, 0, request);
  if (bytes !== request.length) throw new Error(`unexpected Bistro request size ${bytes}`);
  const input = new DataView(request.buffer);
  const command = input.getUint32(0, true);
  const sequence = input.getUint32(4, true);
  const started = performance.now();
  let status = 0;
  if (command === 1) {
    status = steam._bistro_run_reflections(input.getUint32(8, true), input.getFloat32(12, true));
  } else {
    status = 0xffff_ffff;
  }
  const output = new DataView(response.buffer);
  output.setUint32(0, sequence, true);
  output.setUint32(4, status, true);
  output.setFloat64(8, performance.now() - started, true);
  output.setFloat64(16, initializationMs, true);
  output.setUint32(24, steam._bistro_get_vertices(), true);
  output.setUint32(28, steam._bistro_get_triangles(), true);
  output.setUint32(32, steam._bistro_get_tracer_nodes(), true);
  output.setFloat64(40, steam._bistro_get_tracer_build_ms(), true);
  output.setFloat64(48, steam._bistro_get_tracer_owned_bytes(), true);
  output.setFloat32(56, steam._bistro_get_reverb_low(), true);
  output.setUint32(60, steam._bistro_get_ir_valid(), true);
  output.setUint32(64, steam._bistro_get_simulation_threads(), true);
  output.setUint32(68, steam._bistro_get_tracer_lanes(), true);
  output.setUint32(72, geometryBytes, true);
  writeFrame(rings, RING_BYTES, response);
  self.postMessage('wake');
};
