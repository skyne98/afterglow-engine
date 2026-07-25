import {
  TelemetryDescriptorKind,
  TelemetryMetricKind,
  type TelemetryDescriptor,
  type TelemetryMetricDescriptor,
} from './telemetry.ts';

export const enum EngineTelemetryCategory {
  Runtime = 0,
  Frame = 1,
  Worker = 2,
  VirtualTexture = 3,
  Asset = 4,
  Texture = 5,
  Gpu = 6,
  Audio = 7,
  Host = 8,
  Rpc = 9,
}

export const enum EngineTraceDescriptor {
  Frame = 0,
  WorkerPoll = 1,
  VirtualTexture = 2,
  StructuralCommands = 3,
  PoseBatches = 4,
  RenderPrepare = 5,
  GameUpdate = 6,
  RenderPasses = 7,
}

export const ENGINE_TRACE_DESCRIPTORS: readonly TelemetryDescriptor[] = [
  { category: EngineTelemetryCategory.Frame, categoryName: 'frame', name: 'frame', kind: TelemetryDescriptorKind.Span, argument0: 'frame_id', argument1: 'delta_ns' },
  { category: EngineTelemetryCategory.Worker, categoryName: 'worker', name: 'worker.poll', kind: TelemetryDescriptorKind.Span, argument0: 'stage', argument1: 'elapsed_us' },
  { category: EngineTelemetryCategory.VirtualTexture, categoryName: 'vt', name: 'vt.update', kind: TelemetryDescriptorKind.Span, argument0: 'stage', argument1: 'elapsed_us' },
  { category: EngineTelemetryCategory.Runtime, categoryName: 'runtime', name: 'structural.commands', kind: TelemetryDescriptorKind.Span, argument0: 'stage', argument1: 'elapsed_us' },
  { category: EngineTelemetryCategory.Runtime, categoryName: 'runtime', name: 'pose.batches', kind: TelemetryDescriptorKind.Span, argument0: 'stage', argument1: 'elapsed_us' },
  { category: EngineTelemetryCategory.Frame, categoryName: 'frame', name: 'render.prepare', kind: TelemetryDescriptorKind.Span, argument0: 'stage', argument1: 'elapsed_us' },
  { category: EngineTelemetryCategory.Runtime, categoryName: 'runtime', name: 'game.update', kind: TelemetryDescriptorKind.Span, argument0: 'frame_id' },
  { category: EngineTelemetryCategory.Frame, categoryName: 'frame', name: 'render.passes', kind: TelemetryDescriptorKind.Span, argument0: 'frame_id' },
];

export const FRAME_BUDGET_TRACE_DESCRIPTORS: readonly number[] = [
  EngineTraceDescriptor.WorkerPoll,
  EngineTraceDescriptor.VirtualTexture,
  EngineTraceDescriptor.StructuralCommands,
  EngineTraceDescriptor.PoseBatches,
  EngineTraceDescriptor.RenderPrepare,
];

export const enum EngineMetric {
  Frames = 0,
  FrameDeltaNs = 1,
  FrameMaxNs = 2,
}

export const ENGINE_METRIC_DESCRIPTORS: readonly TelemetryMetricDescriptor[] = [
  { category: EngineTelemetryCategory.Frame, categoryName: 'frame', name: 'frames', kind: TelemetryMetricKind.Counter, unit: 'count' },
  { category: EngineTelemetryCategory.Frame, categoryName: 'frame', name: 'frame_delta_ns', kind: TelemetryMetricKind.HistogramLog2, unit: 'nanoseconds' },
  { category: EngineTelemetryCategory.Frame, categoryName: 'frame', name: 'frame_max_ns', kind: TelemetryMetricKind.Maximum, unit: 'nanoseconds' },
];
