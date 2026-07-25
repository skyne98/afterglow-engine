export { FrameBench, BenchStartStatus, benchFromUrl, formatBenchResults } from './diagnostics/bench.ts';
export {
  DiagnosticCode, DiagnosticSource, DiagnosticStatus, EngineDiagnostics,
  type DiagnosticRecord,
} from './core/diagnostics.ts';
export { RenderAdapter, RenderAttachStatus } from './renderer/render-adapter.ts';
export {
  GpuProfiler,
  type GpuFrameScope,
  type GpuProfilerOptions,
  type GpuScopeTiming,
  type GpuZone,
} from './renderer/gpu-profiler.ts';
export {
  RendererHost,
  type GpuAdapterIdentity,
  type RendererHostOptions,
  type RendererViewport,
} from './renderer/renderer-host.ts';
export {
  EngineRuntime, RegistrationStatus, RuntimeState,
  type AnimationScheduler, type EngineFrameClient, type EngineRenderPass,
  type EngineRuntimeOptions, type SceneEngineRuntimeOptions,
} from './core/runtime.ts';
export { RenderTier, RenderDirty } from './core/types.ts';
export type { EntityId, RenderDescriptorId, RenderFrame } from './core/types.ts';
export {
  EngineMetric,
  EngineTelemetry,
  EngineTelemetryCategory,
  EngineTraceDescriptor,
  ENGINE_METRIC_DESCRIPTORS,
  ENGINE_TRACE_DESCRIPTORS,
  FRAME_BUDGET_TRACE_DESCRIPTORS,
  TelemetryCaptureState,
  TelemetryDescriptorKind,
  TelemetryMetricBank,
  TelemetryMetricKind,
  TelemetryMetricStatus,
  TelemetryPhase,
  TelemetryRecorder,
  TelemetryRecordStatus,
  TelemetryRes,
  TELEMETRY_BATCH_HEADER_BYTES,
  TELEMETRY_BATCH_VERSION,
  TELEMETRY_HISTOGRAM_BUCKETS,
  TELEMETRY_RECORD_BYTES,
  type TelemetryClock,
  type TelemetryDescriptor,
  type TelemetryMetricDescriptor,
  type TelemetrySnapshot,
} from './telemetry/index.ts';
export {
  Profiling,
  ProfilingRes,
  type ProfilingFrame,
  type ProfilingHost,
  type ProfilingOptions,
} from './profiling/index.ts';
