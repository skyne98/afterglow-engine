import { describe, expect, test } from 'bun:test';
import { NativeRpcTransport } from './native-transport.ts';
import type { RpcTransport } from '../../workers/codec.ts';
import {
  EngineMetric, ENGINE_METRIC_DESCRIPTORS, ENGINE_TRACE_DESCRIPTORS,
} from '../telemetry/catalog.ts';
import { EngineTelemetry } from '../telemetry/telemetry.ts';

describe('NativeRpcTransport', () => {
  test('implements the RpcTransport interface', () => {
    const t: RpcTransport = new NativeRpcTransport(0);
    expect(t).toBeDefined();
    expect(typeof t.call).toBe('function');
  });

  test('routes call() through the asynchronous native op', async () => {
    const calls: Array<[number, number, Uint8Array]> = [];
    (globalThis as unknown as { Deno: unknown }).Deno = {
      core: {
        ops: {
          op_afterglow_rpc_call_async: async (workerId: number, method: number, args: Uint8Array): Promise<Uint8Array> => {
            calls.push([workerId, method, args]);
            // Echo the args back as the "payload".
            return args;
          },
        },
      },
    };
    try {
      const telemetry = new EngineTelemetry(
        ENGINE_TRACE_DESCRIPTORS,
        ENGINE_METRIC_DESCRIPTORS,
        new ArrayBuffer(40 * 8),
        new Float64Array(256),
        () => 1,
      );
      telemetry.trace.arm(3);
      const t = new NativeRpcTransport(7, telemetry);
      const args = new Uint8Array([1, 2, 3]);
      const out = await t.call(42, args);
      expect(calls).toEqual([[7, 42, args]]);
      expect(out).toBe(args); // the op's return is passed through
      expect(telemetry.metrics.readCell(EngineMetric.RpcCalls)).toBe(1);
      telemetry.trace.stop();
      expect(telemetry.trace.snapshot()?.count).toBe(2);
    } finally {
      delete (globalThis as unknown as { Deno?: unknown }).Deno;
    }
  });
});
