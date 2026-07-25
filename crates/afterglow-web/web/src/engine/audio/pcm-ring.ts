export const AUDIO_SAMPLE_RATE = 48_000;
export const AUDIO_QUANTUM_FRAMES = 128;
export const AUDIO_CHANNELS = 2;
export const AUDIO_PCM_SAMPLES = AUDIO_QUANTUM_FRAMES * AUDIO_CHANNELS;

// Exact afterglow-rpc::RingBuffer framing:
// [capacity_bytes:u32][write_bytes:AtomicU32][read_bytes:AtomicU32]
// [payload_len:u32][sequence:u32][256 interleaved f32]...
export const AUDIO_RING_HEADER_WORDS = 3;
export const AUDIO_RING_PAYLOAD_WORDS = 1 + AUDIO_PCM_SAMPLES;
export const AUDIO_RING_FRAME_WORDS = 1 + AUDIO_RING_PAYLOAD_WORDS;
export const AUDIO_RING_FRAME_BYTES = AUDIO_RING_FRAME_WORDS * 4;
export const AUDIO_RING_TELEMETRY_WORDS = 12;

export const enum AudioRingHeaderWord {
  CapacityBytes = 0,
  WriteBytes = 1,
  ReadBytes = 2,
}

/** Offsets within the separate telemetry tail, not the RingBuffer header. */
export const enum AudioRingWord {
  ConsumeEpoch = 0,
  Underruns = 1,
  SequenceErrors = 2,
  Callbacks = 3,
  Rendered = 4,
  Fatal = 5,
  WakeHits = 6,
  WakeMisses = 7,
  PumpMicros = 8,
  PumpMaxMicros = 9,
  PumpOverBudget = 10,
  Armed = 11,
}

export function audioPcmRingBytes(slotCapacity: number): number {
  if (!Number.isInteger(slotCapacity) || slotCapacity < 2 || slotCapacity > 8)
    throw new RangeError('audio PCM ring capacity must be between 2 and 8 quanta');
  return (AUDIO_RING_HEADER_WORDS + slotCapacity * AUDIO_RING_FRAME_WORDS +
    AUDIO_RING_TELEMETRY_WORDS) * 4;
}

export function createAudioPcmRing(slotCapacity: number): SharedArrayBuffer {
  const memory = new SharedArrayBuffer(audioPcmRingBytes(slotCapacity));
  const header = new Int32Array(memory, 0, AUDIO_RING_HEADER_WORDS);
  Atomics.store(
    header, AudioRingHeaderWord.CapacityBytes,
    slotCapacity * AUDIO_RING_FRAME_BYTES,
  );
  return memory;
}

export function audioPcmRingTelemetry(memory: SharedArrayBuffer): Int32Array {
  const header = new Int32Array(memory, 0, AUDIO_RING_HEADER_WORDS);
  const capacityBytes = Atomics.load(header, AudioRingHeaderWord.CapacityBytes) >>> 0;
  return new Int32Array(
    memory,
    AUDIO_RING_HEADER_WORDS * 4 + capacityBytes,
    AUDIO_RING_TELEMETRY_WORDS,
  );
}

abstract class AudioPcmRingView {
  protected readonly header: Int32Array;
  protected readonly words: Int32Array;
  protected readonly samples: Float32Array;
  readonly telemetry: Int32Array;
  readonly slotCapacity: number;
  protected readonly capacityBytes: number;

  constructor(readonly memory: SharedArrayBuffer) {
    this.header = new Int32Array(memory, 0, AUDIO_RING_HEADER_WORDS);
    this.capacityBytes = Atomics.load(this.header, AudioRingHeaderWord.CapacityBytes) >>> 0;
    if (this.capacityBytes % AUDIO_RING_FRAME_BYTES !== 0)
      throw new RangeError('invalid audio PCM RingBuffer byte capacity');
    this.slotCapacity = this.capacityBytes / AUDIO_RING_FRAME_BYTES;
    if (memory.byteLength !== audioPcmRingBytes(this.slotCapacity))
      throw new RangeError('invalid audio PCM ring storage');
    this.words = new Int32Array(memory);
    this.samples = new Float32Array(memory);
    this.telemetry = audioPcmRingTelemetry(memory);
  }

  protected frameWord(byteOffset: number): number {
    return AUDIO_RING_HEADER_WORDS + (byteOffset % this.capacityBytes) / 4;
  }
}

/** Worker-side producer. Construct once; `tryWrite` is allocation-free. */
export class AudioPcmRingWriter extends AudioPcmRingView {
  // @hot-no-alloc-begin AudioPcmRingWriter.tryWrite
  tryWrite(interleaved: Float32Array): boolean {
    if (interleaved.length !== AUDIO_PCM_SAMPLES) {
      Atomics.store(this.telemetry, AudioRingWord.Fatal, 1);
      return false;
    }
    const write = Atomics.load(this.header, AudioRingHeaderWord.WriteBytes) >>> 0;
    const read = Atomics.load(this.header, AudioRingHeaderWord.ReadBytes) >>> 0;
    if (((write - read) >>> 0) > this.capacityBytes - AUDIO_RING_FRAME_BYTES) return false;
    const frame = this.frameWord(write);
    const sequence = Math.floor(write / AUDIO_RING_FRAME_BYTES) >>> 0;
    this.words[frame] = AUDIO_RING_PAYLOAD_WORDS * 4;
    this.words[frame + 1] = sequence | 0;
    const sampleBase = frame + 2;
    for (let index = 0; index < AUDIO_PCM_SAMPLES; ++index)
      this.samples[sampleBase + index] = interleaved[index];
    Atomics.store(
      this.header, AudioRingHeaderWord.WriteBytes,
      (write + AUDIO_RING_FRAME_BYTES) | 0,
    );
    Atomics.add(this.telemetry, AudioRingWord.Rendered, 1);
    return true;
  }
  // @hot-no-alloc-end AudioPcmRingWriter.tryWrite

  get depth(): number {
    const write = Atomics.load(this.header, AudioRingHeaderWord.WriteBytes) >>> 0;
    const read = Atomics.load(this.header, AudioRingHeaderWord.ReadBytes) >>> 0;
    return Math.floor(((write - read) >>> 0) / AUDIO_RING_FRAME_BYTES);
  }
}

/** AudioWorklet-side consumer. Construct once; `readStereo` is allocation-free. */
export class AudioPcmRingReader extends AudioPcmRingView {
  // @hot-no-alloc-begin AudioPcmRingReader.readStereo
  readStereo(left: Float32Array, right: Float32Array, gain: number): boolean {
    if (Atomics.load(this.telemetry, AudioRingWord.Armed) === 0) {
      this.silence(left, right);
      return false;
    }
    Atomics.add(this.telemetry, AudioRingWord.Callbacks, 1);
    if (Atomics.load(this.telemetry, AudioRingWord.Fatal) !== 0) {
      this.silence(left, right);
      return false;
    }
    if (left.length !== AUDIO_QUANTUM_FRAMES || right.length !== AUDIO_QUANTUM_FRAMES) {
      Atomics.store(this.telemetry, AudioRingWord.Fatal, 1);
      this.silence(left, right);
      return false;
    }
    const read = Atomics.load(this.header, AudioRingHeaderWord.ReadBytes) >>> 0;
    const write = Atomics.load(this.header, AudioRingHeaderWord.WriteBytes) >>> 0;
    if (read === write) {
      Atomics.add(this.telemetry, AudioRingWord.Underruns, 1);
      this.silence(left, right);
      return false;
    }
    const frame = this.frameWord(read);
    const sequence = Math.floor(read / AUDIO_RING_FRAME_BYTES) >>> 0;
    if ((this.words[frame] >>> 0) !== AUDIO_RING_PAYLOAD_WORDS * 4 ||
        (this.words[frame + 1] >>> 0) !== sequence) {
      Atomics.add(this.telemetry, AudioRingWord.SequenceErrors, 1);
      Atomics.store(this.telemetry, AudioRingWord.Fatal, 1);
      this.silence(left, right);
      return false;
    }
    const sampleBase = frame + 2;
    for (let sample = 0; sample < AUDIO_QUANTUM_FRAMES; ++sample) {
      left[sample] = this.samples[sampleBase + sample * 2] * gain;
      right[sample] = this.samples[sampleBase + sample * 2 + 1] * gain;
    }
    Atomics.store(
      this.header, AudioRingHeaderWord.ReadBytes,
      (read + AUDIO_RING_FRAME_BYTES) | 0,
    );
    Atomics.add(this.telemetry, AudioRingWord.ConsumeEpoch, 1);
    const woken = Atomics.notify(this.telemetry, AudioRingWord.ConsumeEpoch, 1);
    Atomics.add(
      this.telemetry,
      woken === 0 ? AudioRingWord.WakeMisses : AudioRingWord.WakeHits,
      1,
    );
    return true;
  }
  // @hot-no-alloc-end AudioPcmRingReader.readStereo

  private silence(left: Float32Array, right: Float32Array): void {
    for (let sample = 0; sample < left.length; ++sample) left[sample] = 0;
    for (let sample = 0; sample < right.length; ++sample) right[sample] = 0;
  }
}
