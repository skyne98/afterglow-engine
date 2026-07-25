// Native RPC transport: calls a spawned native `afterglow-rpc` worker through
// the shell's async op bridge. Implements the same `RpcTransport`
// interface the generated TS clients use; this is
// the only target-specific piece — on the native target it replaces the
// SAB+`postMessage` transport with a direct op call. The worker runs on a real
// OS thread; wakeups are payload-free `unpark`s, exactly as on web.
//
// Calls return immediately and resolve when the host's bounded worker poll
// drains the response ring. Worker payloads still use only afterglow-rpc rings.

import { EngineMetric, EngineTelemetryCategory, EngineTraceDescriptor } from '../telemetry/catalog.ts';
import type { EngineTelemetry } from '../telemetry/telemetry.ts';

// `Deno.core.ops` is provided by the afterglow-shell deno_core runtime on the
// native target. Declared locally so this file type-checks under `bun` without
// the `Deno` global.
interface RpcTransport {
  call(method: number, args: Uint8Array): Promise<Uint8Array>;
}

declare const Deno: {
  core: {
    ops: {
      op_afterglow_rpc_call_async(workerId: number, method: number, args: Uint8Array): Promise<Uint8Array>;
    };
  };
};

export class NativeRpcTransport implements RpcTransport {
  constructor(
    private readonly workerId: number,
    private readonly telemetry?: EngineTelemetry,
  ) {}

  call(method: number, args: Uint8Array): Promise<Uint8Array> {
    const correlation = this.telemetry?.nextCorrelation(EngineTelemetryCategory.Rpc) ?? 0;
    const startedAt = performance.now();
    this.telemetry?.metrics.counterAdd(EngineMetric.RpcCalls, 1);
    this.telemetry?.trace.asyncBegin(EngineTraceDescriptor.RpcCall, correlation, method, args.byteLength);
    return Deno.core.ops.op_afterglow_rpc_call_async(this.workerId, method, args).then(
      result => {
        this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.RpcCall, correlation, result.byteLength, 0);
        this.telemetry?.metrics.histogramLog2(
          EngineMetric.RpcNs, Math.max(1, Math.floor((performance.now() - startedAt) * 1_000_000)),
        );
        return result;
      },
      error => {
        this.telemetry?.trace.asyncEnd(EngineTraceDescriptor.RpcCall, correlation, 0, 1);
        this.telemetry?.metrics.histogramLog2(
          EngineMetric.RpcNs, Math.max(1, Math.floor((performance.now() - startedAt) * 1_000_000)),
        );
        throw error;
      },
    );
  }
}
