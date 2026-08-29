// crates/afterglow-web/web/src/engine/audio/pcm-ring.ts
var AUDIO_QUANTUM_FRAMES = 128;
var AUDIO_CHANNELS = 2;
var AUDIO_PCM_SAMPLES = AUDIO_QUANTUM_FRAMES * AUDIO_CHANNELS;
var AUDIO_RING_HEADER_WORDS = 3;
var AUDIO_RING_PAYLOAD_WORDS = 1 + AUDIO_PCM_SAMPLES;
var AUDIO_RING_FRAME_WORDS = 1 + AUDIO_RING_PAYLOAD_WORDS;
var AUDIO_RING_FRAME_BYTES = AUDIO_RING_FRAME_WORDS * 4;
var AUDIO_RING_TELEMETRY_WORDS = 12;
function audioPcmRingBytes(slotCapacity) {
  if (!Number.isInteger(slotCapacity) || slotCapacity < 2 || slotCapacity > 8)
    throw new RangeError("audio PCM ring capacity must be between 2 and 8 quanta");
  return (AUDIO_RING_HEADER_WORDS + slotCapacity * AUDIO_RING_FRAME_WORDS + AUDIO_RING_TELEMETRY_WORDS) * 4;
}
function audioPcmRingTelemetry(memory) {
  const header = new Int32Array(memory, 0, AUDIO_RING_HEADER_WORDS);
  const capacityBytes = Atomics.load(header, 0 /* CapacityBytes */) >>> 0;
  return new Int32Array(memory, AUDIO_RING_HEADER_WORDS * 4 + capacityBytes, AUDIO_RING_TELEMETRY_WORDS);
}

class AudioPcmRingView {
  memory;
  header;
  words;
  samples;
  telemetry;
  slotCapacity;
  capacityBytes;
  constructor(memory) {
    this.memory = memory;
    this.header = new Int32Array(memory, 0, AUDIO_RING_HEADER_WORDS);
    this.capacityBytes = Atomics.load(this.header, 0 /* CapacityBytes */) >>> 0;
    if (this.capacityBytes % AUDIO_RING_FRAME_BYTES !== 0)
      throw new RangeError("invalid audio PCM RingBuffer byte capacity");
    this.slotCapacity = this.capacityBytes / AUDIO_RING_FRAME_BYTES;
    if (memory.byteLength !== audioPcmRingBytes(this.slotCapacity))
      throw new RangeError("invalid audio PCM ring storage");
    this.words = new Int32Array(memory);
    this.samples = new Float32Array(memory);
    this.telemetry = audioPcmRingTelemetry(memory);
  }
  frameWord(byteOffset) {
    return AUDIO_RING_HEADER_WORDS + byteOffset % this.capacityBytes / 4;
  }
}
class AudioPcmRingReader extends AudioPcmRingView {
  readStereo(left, right, gain) {
    if (Atomics.load(this.telemetry, 11 /* Armed */) === 0) {
      this.silence(left, right);
      return false;
    }
    Atomics.add(this.telemetry, 3 /* Callbacks */, 1);
    if (Atomics.load(this.telemetry, 5 /* Fatal */) !== 0) {
      this.silence(left, right);
      return false;
    }
    if (left.length !== AUDIO_QUANTUM_FRAMES || right.length !== AUDIO_QUANTUM_FRAMES) {
      Atomics.store(this.telemetry, 5 /* Fatal */, 1);
      this.silence(left, right);
      return false;
    }
    const read = Atomics.load(this.header, 2 /* ReadBytes */) >>> 0;
    const write = Atomics.load(this.header, 1 /* WriteBytes */) >>> 0;
    if (read === write) {
      Atomics.add(this.telemetry, 1 /* Underruns */, 1);
      this.silence(left, right);
      return false;
    }
    const frame = this.frameWord(read);
    const sequence = Math.floor(read / AUDIO_RING_FRAME_BYTES) >>> 0;
    if (this.words[frame] >>> 0 !== AUDIO_RING_PAYLOAD_WORDS * 4 || this.words[frame + 1] >>> 0 !== sequence) {
      Atomics.add(this.telemetry, 2 /* SequenceErrors */, 1);
      Atomics.store(this.telemetry, 5 /* Fatal */, 1);
      this.silence(left, right);
      return false;
    }
    const sampleBase = frame + 2;
    for (let sample = 0;sample < AUDIO_QUANTUM_FRAMES; ++sample) {
      left[sample] = this.samples[sampleBase + sample * 2] * gain;
      right[sample] = this.samples[sampleBase + sample * 2 + 1] * gain;
    }
    Atomics.store(this.header, 2 /* ReadBytes */, read + AUDIO_RING_FRAME_BYTES | 0);
    Atomics.add(this.telemetry, 0 /* ConsumeEpoch */, 1);
    const woken = Atomics.notify(this.telemetry, 0 /* ConsumeEpoch */, 1);
    Atomics.add(this.telemetry, woken === 0 ? 7 /* WakeMisses */ : 6 /* WakeHits */, 1);
    return true;
  }
  silence(left, right) {
    for (let sample = 0;sample < left.length; ++sample)
      left[sample] = 0;
    for (let sample = 0;sample < right.length; ++sample)
      right[sample] = 0;
  }
}

// crates/afterglow-web/web/src/engine/audio/audio-worklet.ts
class EngineAudioSinkProcessor extends AudioWorkletProcessor {
  reader;
  gain;
  constructor(options) {
    super(options);
    const processor = options.processorOptions;
    this.reader = new AudioPcmRingReader(processor.memory);
    this.gain = Number.isFinite(processor.masterGain) ? processor.masterGain : 1;
  }
  process(_inputs, outputs) {
    const output = outputs[0];
    if (output === undefined || output[0] === undefined || output[1] === undefined)
      return true;
    this.reader.readStereo(output[0], output[1], this.gain);
    return true;
  }
}
registerProcessor("afterglow-engine-audio-sink", EngineAudioSinkProcessor);
