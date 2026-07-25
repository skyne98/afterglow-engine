import {
  AUDIO_RING_FRAME_BYTES, AudioRingHeaderWord, AudioRingWord,
  audioPcmRingTelemetry, createAudioPcmRing,
} from '../../engine/audio/pcm-ring.ts';
import { EngineAudioServiceClient } from '../../workers/engineaudioservice.client.ts';
import { Rpc } from '../../workers/rpc.ts';

const UPLOAD_CHUNK_BYTES = 512 * 1024;
const REAL_SOUND_NAMES = ['abcd.wav', 'counting.wav', 'impulse.wav', 'ozymandias.wav', 'pinknoise.wav'] as const;

const output = document.getElementById('output');
const startButton = document.getElementById('start');
const stopButton = document.getElementById('stop');
if (!(output instanceof HTMLElement) || !(startButton instanceof HTMLButtonElement) ||
    !(stopButton instanceof HTMLButtonElement)) throw new Error('missing audio diagnostic controls');

let context: AudioContext | null = null;
let sink: AudioWorkletNode | null = null;
let client: EngineAudioServiceClient | null = null;
let timer = 0;

function depthFromUrl(): number {
  const value = Number(new URLSearchParams(location.search).get('quanta') ?? 8);
  return Number.isInteger(value) && value >= 2 && value <= 8 ? value : 8;
}

function status(code: number, operation: string): void {
  if (code !== 0) throw new Error(`${operation} failed with status ${code}`);
}

async function fetchBytes(url: string): Promise<Uint8Array> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`fetch ${url} failed with ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function uploadWav(client: EngineAudioServiceClient, bytes: Uint8Array): Promise<number> {
  status(await client.beginWavUpload(bytes.byteLength, true), 'begin WAV upload');
  for (let offset = 0; offset < bytes.byteLength; offset += UPLOAD_CHUNK_BYTES)
    status(await client.appendWavUpload(bytes.subarray(offset, offset + UPLOAD_CHUNK_BYTES)), 'append WAV upload');
  const handle = await client.finishWavUpload();
  if (handle === 0) throw new Error('finish WAV upload rejected the sound');
  return handle;
}

async function uploadAcousticScene(client: EngineAudioServiceClient, bytes: Uint8Array): Promise<void> {
  status(await client.beginAcousticSceneUpload(bytes.byteLength), 'begin acoustic scene upload');
  for (let offset = 0; offset < bytes.byteLength; offset += UPLOAD_CHUNK_BYTES)
    status(await client.appendAcousticSceneUpload(bytes.subarray(offset, offset + UPLOAD_CHUNK_BYTES)), 'append acoustic scene upload');
  status(await client.finishAcousticSceneUpload(), 'finish acoustic scene upload');
}

function diagnosticStereoWav(): Uint8Array {
  const frames = 4_800;
  const channels = 2;
  const dataBytes = frames * channels * 2;
  const bytes = new Uint8Array(44 + dataBytes);
  const view = new DataView(bytes.buffer);
  const text = (offset: number, value: string): void => {
    for (let index = 0; index < value.length; ++index) bytes[offset + index] = value.charCodeAt(index);
  };
  text(0, 'RIFF'); view.setUint32(4, 36 + dataBytes, true); text(8, 'WAVE');
  text(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true);
  view.setUint16(22, channels, true); view.setUint32(24, 48_000, true);
  view.setUint32(28, 48_000 * channels * 2, true); view.setUint16(32, channels * 2, true);
  view.setUint16(34, 16, true); text(36, 'data'); view.setUint32(40, dataBytes, true);
  for (let frame = 0; frame < frames; ++frame) {
    view.setInt16(44 + frame * 4, Math.sin(frame * Math.PI * 2 * 220 / 48_000) * 8_192, true);
    view.setInt16(46 + frame * 4, Math.sin(frame * Math.PI * 2 * 330 / 48_000) * 8_192, true);
  }
  return bytes;
}

startButton.addEventListener('click', async (): Promise<void> => {
  if (context !== null) return;
  try {
    const depth = depthFromUrl();
    const search = new URLSearchParams(location.search);
    const useSteam = search.has('steam');
    const useRealSounds = search.has('realSounds');
    const acousticScene = search.get('acousticScene');
    const pcmMemory = createAudioPcmRing(depth);
    const ringHeader = new Int32Array(pcmMemory, 0, 3);
    const telemetry = audioPcmRingTelemetry(pcmMemory);
    const rpc = await Rpc.create({
      mainWasmUrl: 'afterglow_rpc.wasm',
      workerJsUrl: 'audio-worker.js',
      workerWasmUrl: 'engineaudioservice.wasm',
      timeoutMs: 10_000,
      workerInit: {
        pcmMemory,
        emscriptenModuleUrl: useSteam
          ? new URL('engine-audio-rpc.js', location.href).href
          : undefined,
      },
    });
    client = new EngineAudioServiceClient(rpc);
    status(await client.configure(depth, 10_000, 512, 2, 500, 0), 'configure');
    let sourceX = -1.5;
    let sourceY = 0;
    let sourceZ = -1;
    if (acousticScene !== null) {
      const sceneBytes = await fetchBytes(`real-audio/${acousticScene}.acoustic.bin`);
      if (sceneBytes.byteLength < 72) throw new Error('truncated acoustic scene header');
      const header = new DataView(sceneBytes.buffer, sceneBytes.byteOffset, 72);
      sourceX = header.getFloat32(60, true);
      sourceY = header.getFloat32(64, true);
      sourceZ = header.getFloat32(68, true);
      await uploadAcousticScene(client, sceneBytes);
    }
    const sounds: number[] = [];
    if (useRealSounds) {
      for (const name of REAL_SOUND_NAMES)
        sounds.push(await uploadWav(client, await fetchBytes(`real-audio/${name}`)));
    } else {
      const residentSound = await client.loadWav(diagnosticStereoWav(), true);
      if (residentSound === 0) throw new Error('resident WAV was not admitted');
      for (let index = 0; index < REAL_SOUND_NAMES.length; ++index) sounds.push(residentSound);
    }
    status(await client.runSimulation(), 'simulation');
    status(await client.start(), 'start');
    // Exercise the real fixed scheduler rather than leaving the backend's
    // legacy all-slots benchmark mix active.
    for (let index = 0; index < 4; ++index) {
      const voice = await client.spawnAt(sounds[index], sourceX + index * 0.12, sourceY, sourceZ);
      if (voice === 0) throw new Error(`physical voice ${index} was not admitted`);
    }
    const firstDry = await client.spawn2d(sounds[4]);
    if (firstDry === 0) throw new Error('resident dry voice was not admitted');
    const secondDry = await client.crossfadeTo(firstDry, sounds[1], 0.25);
    if (secondDry === 0) throw new Error('diagnostic crossfade was not admitted');
    status(await client.setVoiceVolume(secondDry, 0.65, 0.25), 'volume ramp');
    await waitForPcmDepth(ringHeader, depth, 5_000);

    context = new AudioContext({ sampleRate: 48_000, latencyHint: 'interactive' });
    await context.audioWorklet.addModule('engine/audio-worklet.js');
    sink = new AudioWorkletNode(context, 'afterglow-engine-audio-sink', {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
      processorOptions: { memory: pcmMemory, masterGain: 0.35 },
    });
    sink.connect(context.destination);
    await context.resume();
    Atomics.store(telemetry, AudioRingWord.Armed, 1);
    timer = window.setInterval(async (): Promise<void> => {
      if (client === null) return;
      const worker = await client.stats();
      const write = Atomics.load(ringHeader, AudioRingHeaderWord.WriteBytes) >>> 0;
      const read = Atomics.load(ringHeader, AudioRingHeaderWord.ReadBytes) >>> 0;
      const rendered = Atomics.load(telemetry, AudioRingWord.Rendered);
      output.textContent = JSON.stringify({
        backend: useSteam ? 'unified-rust-rpc-steam-audio-hybrid' : 'unified-rust-rpc-synthetic-gate',
        soundSet: useRealSounds ? 'Steam Audio SDK real speech/noise/impulse set' : 'generated diagnostic WAV',
        acousticScene: acousticScene ?? 'generated room',
        targetQuanta: depth,
        ringDepth: Math.floor(((write - read) >>> 0) / AUDIO_RING_FRAME_BYTES),
        rendered,
        callbacks: Atomics.load(telemetry, AudioRingWord.Callbacks),
        underruns: Atomics.load(telemetry, AudioRingWord.Underruns),
        sequenceErrors: Atomics.load(telemetry, AudioRingWord.SequenceErrors),
        wakeHits: Atomics.load(telemetry, AudioRingWord.WakeHits),
        wakeMisses: Atomics.load(telemetry, AudioRingWord.WakeMisses),
        pumpMeanMs: rendered === 0 ? 0 : Atomics.load(telemetry, AudioRingWord.PumpMicros) / rendered / 1_000,
        pumpMaxMs: Atomics.load(telemetry, AudioRingWord.PumpMaxMicros) / 1_000,
        pumpOverBudget: Atomics.load(telemetry, AudioRingWord.PumpOverBudget),
        fatal: Atomics.load(telemetry, AudioRingWord.Fatal),
        sampleClock: worker[0],
        workerRendered: worker[1],
        simulationUpdates: worker[2],
        outputEnergy: worker[3],
        outputPeak: worker[4],
        lastImpulseSample: worker[5],
        voices: worker[7],
        reflectionVoices: worker[8],
        activeSpatialVoices: worker[15],
        activeReflectionVoices: worker[16],
        activeVoices: worker[17],
        activeWorldPhysicalVoices: worker[18],
        rejectedVoiceCapacity: worker[19],
        rejectedPhysicalCapacity: worker[20],
        staleVoiceHandles: worker[21],
        completedVoiceFades: worker[22],
        loadedResidentSounds: worker[23],
        residentSoundBytes: worker[24],
        acousticVertices: worker[25],
        acousticTriangles: worker[26],
        acousticSceneBytes: worker[27],
      }, null, 2);
    }, 250);
    startButton.disabled = true;
    stopButton.disabled = false;
  } catch (error) {
    output.textContent = `FATAL AUDIO: ${String((error as Error).message ?? error)}`;
    await dispose();
  }
});

stopButton.addEventListener('click', (): void => { void dispose(); });
window.addEventListener('pagehide', (): void => { void dispose(); }, { once: true });

async function waitForPcmDepth(header: Int32Array, target: number, timeoutMs: number): Promise<void> {
  const deadline = performance.now() + timeoutMs;
  while (performance.now() < deadline) {
    const write = Atomics.load(header, AudioRingHeaderWord.WriteBytes) >>> 0;
    const read = Atomics.load(header, AudioRingHeaderWord.ReadBytes) >>> 0;
    if (Math.floor(((write - read) >>> 0) / AUDIO_RING_FRAME_BYTES) >= target) return;
    await new Promise<void>(resolve => setTimeout(resolve, 1));
  }
  throw new Error('EngineAudio worker did not prefill the final PCM ring');
}

async function dispose(): Promise<void> {
  if (timer !== 0) { clearInterval(timer); timer = 0; }
  sink?.disconnect();
  sink = null;
  if (client !== null) {
    try { await client.stop(); await client.shutdown(); } catch {}
    client.close();
    client = null;
  }
  if (context !== null) { await context.close(); context = null; }
  startButton.disabled = false;
  stopButton.disabled = true;
}
