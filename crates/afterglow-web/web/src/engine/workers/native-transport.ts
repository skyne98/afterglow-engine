// Native RPC transport: calls a spawned native `afterglow-rpc` worker through
// the shell's async op bridge. Implements the same `RpcTransport`
// interface the generated TS clients use (imported from `./codec.ts`); this is
// the only target-specific piece — on the native target it replaces the
// SAB+`postMessage` transport with a direct op call. The worker runs on a real
// OS thread; wakeups are payload-free `unpark`s, exactly as on web.
//
// Calls return immediately and resolve when the host's bounded worker poll
// drains the response ring. Worker payloads still use only afterglow-rpc rings.

import type { RpcTransport } from './codec.ts';

// `Deno.core.ops` is provided by the afterglow-shell deno_core runtime on the
// native target. Declared locally so this file type-checks under `bun` without
// the `Deno` global.
declare const Deno: {
  core: {
    ops: {
      op_afterglow_rpc_call_async(workerId: number, method: number, args: Uint8Array): Promise<Uint8Array>;
    };
  };
};

export class NativeRpcTransport implements RpcTransport {
  constructor(private readonly workerId: number) {}

  call(method: number, args: Uint8Array): Promise<Uint8Array> {
    return Deno.core.ops.op_afterglow_rpc_call_async(this.workerId, method, args);
  }
}
