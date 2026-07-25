import { AudioPcmRingReader } from './pcm-ring.ts';

declare abstract class AudioWorkletProcessor {
  constructor(options?: AudioWorkletNodeOptions);
}
declare function registerProcessor(
  name: string,
  processorCtor: new (options: AudioWorkletNodeOptions) => AudioWorkletProcessor,
): void;

interface EngineAudioProcessorOptions {
  memory: SharedArrayBuffer;
  masterGain: number;
}

class EngineAudioSinkProcessor extends AudioWorkletProcessor {
  private readonly reader: AudioPcmRingReader;
  private gain: number;

  constructor(options: AudioWorkletNodeOptions) {
    super(options);
    const processor = options.processorOptions as EngineAudioProcessorOptions;
    this.reader = new AudioPcmRingReader(processor.memory);
    this.gain = Number.isFinite(processor.masterGain) ? processor.masterGain : 1;
  }

  // @hot-no-alloc-begin EngineAudioSinkProcessor.process
  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const output = outputs[0];
    if (output === undefined || output[0] === undefined || output[1] === undefined)
      return true;
    this.reader.readStereo(output[0], output[1], this.gain);
    return true;
  }
  // @hot-no-alloc-end EngineAudioSinkProcessor.process
}

registerProcessor('afterglow-engine-audio-sink', EngineAudioSinkProcessor);
