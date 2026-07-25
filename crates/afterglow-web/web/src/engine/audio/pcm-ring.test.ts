import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  AUDIO_PCM_SAMPLES,
  AUDIO_QUANTUM_FRAMES,
  AUDIO_RING_FRAME_BYTES,
  AUDIO_RING_FRAME_WORDS,
  AUDIO_RING_HEADER_WORDS,
  AUDIO_RING_PAYLOAD_WORDS,
  AUDIO_RING_TELEMETRY_WORDS,
  AudioPcmRingReader,
  AudioPcmRingWriter,
  AudioRingHeaderWord,
  AudioRingWord,
  audioPcmRingTelemetry,
  createAudioPcmRing,
} from './pcm-ring.ts';

function arm(memory: SharedArrayBuffer): void {
  Atomics.store(audioPcmRingTelemetry(memory), AudioRingWord.Armed, 1);
}

function quantum(seed: number): Float32Array {
  const values = new Float32Array(AUDIO_PCM_SAMPLES);
  for (let index = 0; index < values.length; ++index) values[index] = seed + index / 1_000;
  return values;
}

describe('EngineAudio final PCM ring', () => {
  test('supports bounded two-to-eight quantum depths', () => {
    for (const capacity of [2, 3, 4, 8]) {
      const memory = createAudioPcmRing(capacity);
      expect(new Int32Array(memory)[AudioRingHeaderWord.CapacityBytes])
        .toBe(capacity * AUDIO_RING_FRAME_BYTES);
    }
    expect(() => createAudioPcmRing(1)).toThrow();
    expect(() => createAudioPcmRing(9)).toThrow();
  });

  test('preserves stereo samples and sequence across wrap without allocation growth', () => {
    const memory = createAudioPcmRing(2);
    const writer = new AudioPcmRingWriter(memory);
    const reader = new AudioPcmRingReader(memory);
    arm(memory);
    const left = new Float32Array(AUDIO_QUANTUM_FRAMES);
    const right = new Float32Array(AUDIO_QUANTUM_FRAMES);
    for (let sequence = 0; sequence < 20; ++sequence) {
      const input = quantum(sequence);
      expect(writer.tryWrite(input)).toBe(true);
      expect(reader.readStereo(left, right, 0.5)).toBe(true);
      expect(left[0]).toBeCloseTo(input[0] * 0.5);
      expect(right[127]).toBeCloseTo(input[255] * 0.5);
    }
    const telemetry = audioPcmRingTelemetry(memory);
    expect(telemetry[AudioRingWord.Rendered]).toBe(20);
    expect(telemetry[AudioRingWord.Callbacks]).toBe(20);
    expect(telemetry[AudioRingWord.Underruns]).toBe(0);
    expect(telemetry[AudioRingWord.SequenceErrors]).toBe(0);
  });

  test('reports deterministic full, underrun, and corrupt-sequence behavior', () => {
    const memory = createAudioPcmRing(2);
    const writer = new AudioPcmRingWriter(memory);
    const reader = new AudioPcmRingReader(memory);
    arm(memory);
    const left = new Float32Array(AUDIO_QUANTUM_FRAMES).fill(7);
    const right = new Float32Array(AUDIO_QUANTUM_FRAMES).fill(7);
    expect(reader.readStereo(left, right, 1)).toBe(false);
    expect(left[0]).toBe(0);
    expect(writer.tryWrite(quantum(1))).toBe(true);
    expect(writer.tryWrite(quantum(2))).toBe(true);
    expect(writer.tryWrite(quantum(3))).toBe(false);
    const words = new Int32Array(memory);
    expect(words[AUDIO_RING_HEADER_WORDS]).toBe(AUDIO_RING_PAYLOAD_WORDS * 4);
    words[AUDIO_RING_HEADER_WORDS] = 99;
    expect(reader.readStereo(left, right, 1)).toBe(false);
    const telemetry = audioPcmRingTelemetry(memory);
    expect(telemetry[AudioRingWord.SequenceErrors]).toBe(1);
    expect(telemetry[AudioRingWord.Fatal]).toBe(1);
  });

  test('rejects malformed storage and source quantum sizes', () => {
    const memory = createAudioPcmRing(2);
    const writer = new AudioPcmRingWriter(memory);
    expect(writer.tryWrite(new Float32Array(1))).toBe(false);
    expect(audioPcmRingTelemetry(memory)[AudioRingWord.Fatal]).toBe(1);
    const malformed = new SharedArrayBuffer(
      (AUDIO_RING_HEADER_WORDS + 2 * AUDIO_RING_FRAME_WORDS +
        AUDIO_RING_TELEMETRY_WORDS - 1) * 4,
    );
    new Int32Array(malformed)[AudioRingHeaderWord.CapacityBytes] =
      2 * AUDIO_RING_FRAME_BYTES;
    expect(() => new AudioPcmRingReader(malformed)).toThrow();
  });

  test('authored AudioWorklet callback contains no allocation or messaging', () => {
    const source = readFileSync(
      new URL('./audio-worklet.ts', import.meta.url), 'utf8',
    );
    const marker = '// @hot-' + 'no-alloc-';
    const hot = source.split(`${marker}begin EngineAudioSinkProcessor.process`)[1]
      ?.split(`${marker}end EngineAudioSinkProcessor.process`)[0] ?? '';
    expect(hot).not.toMatch(/\bnew\b|postMessage|\.port\.|Promise|=>|`/);
    expect(hot).toContain('readStereo');
  });
});
