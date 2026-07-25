import { expect, test } from 'bun:test';
import { analyzePcm16Wav } from './analyze-audio-wav.ts';

test('analyzes PCM16 stereo waveform and internal silence', () => {
  const samples = new Int16Array([100, -100, 0, 0, 200, -200]);
  const bytes = new Uint8Array(44 + samples.byteLength);
  const view = new DataView(bytes.buffer);
  const text = (offset: number, value: string): void => {
    for (let index = 0; index < value.length; ++index)
      view.setUint8(offset + index, value.charCodeAt(index));
  };
  text(0, 'RIFF'); view.setUint32(4, bytes.length - 8, true); text(8, 'WAVE');
  text(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true);
  view.setUint16(22, 2, true); view.setUint32(24, 48_000, true);
  view.setUint32(28, 192_000, true); view.setUint16(32, 4, true); view.setUint16(34, 16, true);
  text(36, 'data'); view.setUint32(40, samples.byteLength, true);
  for (let index = 0; index < samples.length; ++index)
    view.setInt16(44 + index * 2, samples[index]!, true);

  const result = analyzePcm16Wav(bytes);
  expect(result.frames).toBe(3);
  expect(result.channels).toBe(2);
  expect(result.nonzeroSamples).toBe(4);
  expect(result.peakI16).toBe(200);
  expect(result.firstNonzeroFrame).toBe(0);
  expect(result.lastNonzeroFrame).toBe(2);
  expect(result.longestInternalZeroFrames).toBe(1);
});
