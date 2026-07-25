export interface WavAnalysis {
  frames: number;
  channels: number;
  sampleRate: number;
  durationSeconds: number;
  nonzeroSamples: number;
  rmsI16: number;
  peakI16: number;
  firstNonzeroFrame: number;
  lastNonzeroFrame: number;
  longestInternalZeroFrames: number;
  longestInternalZeroMs: number;
}

function fourCc(view: DataView, offset: number): string {
  return String.fromCharCode(
    view.getUint8(offset), view.getUint8(offset + 1),
    view.getUint8(offset + 2), view.getUint8(offset + 3),
  );
}

export function analyzePcm16Wav(bytes: Uint8Array): WavAnalysis {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.byteLength < 44 || fourCc(view, 0) !== 'RIFF' || fourCc(view, 8) !== 'WAVE')
    throw new Error('not a RIFF/WAVE file');
  let channels = 0;
  let sampleRate = 0;
  let dataOffset = 0;
  let dataLength = 0;
  for (let offset = 12; offset + 8 <= bytes.byteLength;) {
    const kind = fourCc(view, offset);
    const length = view.getUint32(offset + 4, true);
    const body = offset + 8;
    if (body + length > bytes.byteLength) throw new Error('truncated WAV chunk');
    if (kind === 'fmt ') {
      if (length < 16 || view.getUint16(body, true) !== 1 || view.getUint16(body + 14, true) !== 16)
        throw new Error('expected PCM16 WAV');
      channels = view.getUint16(body + 2, true);
      sampleRate = view.getUint32(body + 4, true);
    } else if (kind === 'data') {
      dataOffset = body;
      dataLength = length;
    }
    offset = body + length + (length & 1);
  }
  if (channels === 0 || sampleRate === 0 || dataLength === 0 || dataLength % (channels * 2) !== 0)
    throw new Error('missing or malformed WAV format/data');
  const sampleCount = dataLength / 2;
  const frames = sampleCount / channels;
  let nonzeroSamples = 0;
  let sumSquares = 0;
  let peakI16 = 0;
  let firstNonzeroFrame = -1;
  let lastNonzeroFrame = -1;
  let zeroRun = 0;
  let longestInternalZeroFrames = 0;
  for (let frame = 0; frame < frames; ++frame) {
    let frameNonzero = false;
    for (let channel = 0; channel < channels; ++channel) {
      const sample = view.getInt16(dataOffset + (frame * channels + channel) * 2, true);
      const absolute = Math.abs(sample);
      if (sample !== 0) { ++nonzeroSamples; frameNonzero = true; }
      sumSquares += sample * sample;
      peakI16 = Math.max(peakI16, absolute);
    }
    if (frameNonzero) {
      if (firstNonzeroFrame < 0) firstNonzeroFrame = frame;
      lastNonzeroFrame = frame;
      if (firstNonzeroFrame >= 0) longestInternalZeroFrames = Math.max(longestInternalZeroFrames, zeroRun);
      zeroRun = 0;
    } else if (firstNonzeroFrame >= 0) {
      ++zeroRun;
    }
  }
  return {
    frames, channels, sampleRate,
    durationSeconds: frames / sampleRate,
    nonzeroSamples,
    rmsI16: Math.sqrt(sumSquares / sampleCount),
    peakI16,
    firstNonzeroFrame,
    lastNonzeroFrame,
    longestInternalZeroFrames,
    longestInternalZeroMs: longestInternalZeroFrames * 1_000 / sampleRate,
  };
}

if (import.meta.main) {
  const path = process.argv[2];
  if (path === undefined) throw new Error('usage: bun scripts/analyze-audio-wav.ts <pcm16.wav>');
  const analysis = analyzePcm16Wav(new Uint8Array(await Bun.file(path).arrayBuffer()));
  console.log(JSON.stringify({ path, ...analysis }));
}
