import { decodeBytes, decodeU32, decodeU8, encodeString, encodeU8 } from './codec.ts';
import { installRingService } from './ring-service.ts';
import { WebSocketCaptureClient } from '../../../../afterglow-telemetry/web/src/websocket.ts';

installRingService(configuration => {
  if (typeof configuration !== 'object' || configuration === null
    || typeof (configuration as { url?: unknown }).url !== 'string') throw new Error('Missing profiling server URL');
  const client = new WebSocketCaptureClient((configuration as { url: string }).url);
  return async (method, args) => {
    if (method === 0 && args.byteLength === 0) return encodeString(await client.start());
    if (method === 2 && args.byteLength === 0) return encodeString(await client.finish());
    if (method === 1) {
      const [epoch, a] = decodeU32(args, 0);
      const [opcode, b] = decodeU8(args, a);
      const [payload, end] = decodeBytes(args, b);
      if (end !== args.byteLength) throw new Error('Trailing profiling RPC data');
      return encodeU8(await client.ingest(epoch, opcode, payload));
    }
    throw new Error('Invalid profiling RPC method');
  };
});
