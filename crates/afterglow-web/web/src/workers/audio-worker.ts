import { AudioPcmRingWriter, AudioRingWord } from '../engine/audio/pcm-ring.ts';
import { HEADER, U32, rdU32, wrU32, xfer } from './ring-buf.ts';

interface AudioWorkerInit {
  pcmMemory: SharedArrayBuffer;
  emscriptenModuleUrl?: string;
}

interface WorkerInitMessage {
  type: 'init';
  sab: SharedArrayBuffer;
  reqBase: number;
  respBase: number;
  bufSize: number;
  wasmUrl: string;
  workerInit: AudioWorkerInit;
}

interface AudioWorkerExports extends WebAssembly.Exports {
  memory?: WebAssembly.Memory;
  afterglow_wasm_init(): void;
  afterglow_wasm_serve_frame(
    method: number, argsPtr: number, argsLen: number,
    outputPtr: number, outputSize: number,
  ): number;
  afterglow_wasm_input_ptr(): number;
  afterglow_wasm_input_size(): number;
  afterglow_wasm_output_ptr(): number;
  afterglow_wasm_output_size(): number;
  afterglow_audio_pump(): number;
  afterglow_audio_pcm_ptr(): number;
  afterglow_audio_pcm_samples(): number;
  afterglow_audio_simulate_motion(phase: number): number;
}

let initMessage: WorkerInitMessage | null = null;
let wasm: AudioWorkerExports | null = null;
let wasmMemory: WebAssembly.Memory | null = null;
let wasmBuffer: ArrayBufferLike | null = null;
let inputPtr = 0;
let inputSize = 0;
let outputPtr = 0;
let outputSize = 0;
let pcmWriter: AudioPcmRingWriter | null = null;
let pcmTelemetry: Int32Array | null = null;
let pcmQuantum: Float32Array | null = null;
let state: 'init' | 'ready' | 'running' = 'init';

self.onmessage = (event: MessageEvent): void => {
  const message = event.data as WorkerInitMessage | { type: 'run' } | 'wake';
  if (state === 'init' && typeof message === 'object' && message.type === 'init') {
    void initialize(message);
    return;
  }
  if (state === 'ready' && typeof message === 'object' && message.type === 'run') {
    state = 'running';
    run();
  }
};

async function initialize(message: WorkerInitMessage): Promise<void> {
  try {
    if (!(message.workerInit?.pcmMemory instanceof SharedArrayBuffer))
      throw new Error('audio worker requires PCM SharedArrayBuffer');
    initMessage = message;
    if (message.workerInit.emscriptenModuleUrl !== undefined) {
      const moduleUrl = message.workerInit.emscriptenModuleUrl;
      const imported = await import(moduleUrl) as { default: (options: object) => Promise<Record<string, unknown>> };
      const emscripten = await imported.default({
        locateFile: (path: string): string => new URL(path, moduleUrl).href,
      });
      const heap = emscripten.HEAPU8 as Uint8Array;
      wasmBuffer = heap.buffer;
      wasm = {
        afterglow_wasm_init: emscripten._afterglow_wasm_init,
        afterglow_wasm_serve_frame: emscripten._afterglow_wasm_serve_frame,
        afterglow_wasm_input_ptr: emscripten._afterglow_wasm_input_ptr,
        afterglow_wasm_input_size: emscripten._afterglow_wasm_input_size,
        afterglow_wasm_output_ptr: emscripten._afterglow_wasm_output_ptr,
        afterglow_wasm_output_size: emscripten._afterglow_wasm_output_size,
        afterglow_audio_pump: emscripten._afterglow_audio_pump,
        afterglow_audio_pcm_ptr: emscripten._afterglow_audio_pcm_ptr,
        afterglow_audio_pcm_samples: emscripten._afterglow_audio_pcm_samples,
        afterglow_audio_simulate_motion: emscripten._afterglow_audio_simulate_motion,
      } as AudioWorkerExports;
    } else {
      wasmMemory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
      const imports = { env: {
        memory: wasmMemory,
        notify_worker: (): void => {},
        performance_now: (): number => performance.now(),
      } };
      const module = await WebAssembly.compile(await (await fetch(message.wasmUrl)).arrayBuffer());
      wasm = (await WebAssembly.instantiate(module, imports)).exports as AudioWorkerExports;
      wasmBuffer = wasmMemory.buffer;
    }
    wasm.afterglow_wasm_init();
    inputPtr = wasm.afterglow_wasm_input_ptr();
    inputSize = wasm.afterglow_wasm_input_size();
    outputPtr = wasm.afterglow_wasm_output_ptr();
    outputSize = wasm.afterglow_wasm_output_size();
    pcmWriter = new AudioPcmRingWriter(message.workerInit.pcmMemory);
    pcmTelemetry = pcmWriter.telemetry;
    refreshPcmView();
    state = 'ready';
    self.postMessage({ type: 'ready' });
  } catch (error) {
    self.postMessage({ type: 'error', message: String((error as Error).message ?? error) });
  }
}

function refreshPcmView(): void {
  if (wasm === null || wasmBuffer === null) return;
  const pointer = wasm.afterglow_audio_pcm_ptr();
  const samples = wasm.afterglow_audio_pcm_samples();
  if (pointer === 0 || samples !== 256) throw new Error('invalid Rust audio PCM export');
  pcmQuantum = new Float32Array(wasmBuffer, pointer, samples);
}

function run(): void {
  if (initMessage === null || wasm === null || wasmBuffer === null ||
      pcmWriter === null || pcmTelemetry === null || pcmQuantum === null) return;
  const message = initMessage;
  const reqCap = Atomics.load(new Uint32Array(message.sab, message.reqBase, 1), 0) >>> 0;
  const respCap = Atomics.load(new Uint32Array(message.sab, message.respBase, 1), 0) >>> 0;
  if (reqCap !== message.bufSize - HEADER || respCap !== message.bufSize - HEADER) {
    fail('bad audio RPC ring capacity');
    return;
  }
  const reqWrite = new Int32Array(message.sab, message.reqBase + U32, 1);
  const reqRead = new Int32Array(message.sab, message.reqBase + 2 * U32, 1);
  const reqData = new Uint8Array(message.sab, message.reqBase + HEADER, reqCap);
  const respWrite = new Int32Array(message.sab, message.respBase + U32, 1);
  const respRead = new Int32Array(message.sab, message.respBase + 2 * U32, 1);
  const respData = new Uint8Array(message.sab, message.respBase + HEADER, respCap);
  const input = new Uint8Array(wasmBuffer, inputPtr, inputSize);
  const output = new Uint8Array(wasmBuffer, outputPtr, outputSize);
  let nextSimulationMs = performance.now() + 1_000;
  let simulationPhase = 0;

  for (;;) {
    let serviced = false;
    let attemptedPump = false;
    let pumpResult = 0;
    const requestWrite = Atomics.load(reqWrite, 0) >>> 0;
    const requestRead = Atomics.load(reqRead, 0) >>> 0;
    const used = (requestWrite - requestRead) >>> 0;
    if (used !== 0) {
      if (used > reqCap || used < U32) { fail('corrupt audio RPC request ring'); return; }
      const offset = requestRead % reqCap;
      const payloadLength = rdU32(reqData, offset, reqCap);
      const frameLength = U32 + payloadLength;
      if (payloadLength < U32 || frameLength > used || frameLength > reqCap) {
        fail('corrupt audio RPC request frame'); return;
      }
      const method = rdU32(reqData, offset + U32, reqCap) >>> 0;
      const argsLength = payloadLength - U32;
      if (argsLength > inputSize) { fail('audio RPC request exceeds scratch'); return; }
      xfer(reqData, offset + 2 * U32, reqCap, input, argsLength, 'rd');
      Atomics.store(reqRead, 0, (requestRead + frameLength) | 0);
      const responseLength = wasm.afterglow_wasm_serve_frame(
        method, inputPtr, argsLength, outputPtr, Math.min(outputSize, respCap - U32),
      );
      if (responseLength < 0 || responseLength > outputSize) {
        fail('audio RPC service failed'); return;
      }
      const responseWrite = Atomics.load(respWrite, 0) >>> 0;
      const responseRead = Atomics.load(respRead, 0) >>> 0;
      const responseUsed = (responseWrite - responseRead) >>> 0;
      const responseFrameLength = U32 + responseLength;
      if (responseFrameLength > respCap - responseUsed) {
        fail('audio RPC response ring full'); return;
      }
      const responseOffset = responseWrite % respCap;
      wrU32(respData, responseOffset, respCap, responseLength);
      xfer(respData, responseOffset + U32, respCap, output, responseLength, 'wr');
      Atomics.store(respWrite, 0, (responseWrite + responseFrameLength) | 0);
      if (method === 0) refreshPcmView();
      self.postMessage('wake');
      serviced = true;
    }

    if (pcmWriter.depth < pcmWriter.slotCapacity) {
      attemptedPump = true;
      const pumpStarted = performance.now();
      pumpResult = wasm.afterglow_audio_pump();
      const pumpMicros = Math.max(0, Math.round((performance.now() - pumpStarted) * 1_000));
      Atomics.add(pcmTelemetry, AudioRingWord.PumpMicros, pumpMicros);
      if (pumpMicros > 2_667) Atomics.add(pcmTelemetry, AudioRingWord.PumpOverBudget, 1);
      let priorMax = Atomics.load(pcmTelemetry, AudioRingWord.PumpMaxMicros);
      while (pumpMicros > priorMax) {
        const observed = Atomics.compareExchange(
          pcmTelemetry, AudioRingWord.PumpMaxMicros, priorMax, pumpMicros,
        );
        if (observed === priorMax) break;
        priorMax = observed;
      }
      if (pumpResult < 0) { fail(`Rust audio pump failed: ${pumpResult}`); return; }
      if (pumpResult > 0) {
        if (!pcmWriter.tryWrite(pcmQuantum)) { fail('audio PCM publication failed'); return; }
        continue;
      }
    }
    if (pcmWriter.depth >= pcmWriter.slotCapacity) {
      const now = performance.now();
      if (now >= nextSimulationMs) {
        simulationPhase += 0.025;
        const simulationStatus = wasm.afterglow_audio_simulate_motion(simulationPhase);
        if (simulationStatus !== 0) {
          fail(`Rust audio simulation failed: ${simulationStatus}`);
          return;
        }
        nextSimulationMs = now + 1_000;
        continue;
      }
    }

    if (!serviced) {
      // Load the epoch before re-checking depth. If the worklet consumes
      // between these operations, Atomics.wait observes a changed value and
      // returns immediately instead of losing the wake for up to one timeout.
      const epoch = Atomics.load(pcmTelemetry, AudioRingWord.ConsumeEpoch);
      if (pcmWriter.depth >= pcmWriter.slotCapacity)
        Atomics.wait(pcmTelemetry, AudioRingWord.ConsumeEpoch, epoch, 1);
      else if (attemptedPump && pumpResult === 0)
        Atomics.wait(pcmTelemetry, AudioRingWord.ConsumeEpoch, epoch, 1);
    }
  }
}

function fail(message: string): void {
  if (pcmTelemetry !== null) Atomics.store(pcmTelemetry, AudioRingWord.Fatal, 1);
  self.postMessage({ type: 'error', message });
}
