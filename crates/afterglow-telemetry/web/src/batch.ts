import { TELEMETRY_BATCH_HEADER_BYTES, TELEMETRY_BATCH_VERSION, TELEMETRY_RECORD_BYTES, type TelemetryProducerIdentity } from './telemetry.ts';

export interface DecodedTelemetryRecord {
  timestamp: bigint;
  correlation: bigint;
  argument0: bigint;
  argument1: bigint;
  descriptor: number;
  phase: number;
}

export interface DecodedTelemetryBatch {
  identity: TelemetryProducerIdentity;
  epoch: number;
  rolling: boolean;
  firstSequence: bigint;
  nextSequence: bigint;
  dropped: bigint;
  overwritten: bigint;
  ticksPerSecond: bigint;
  records: DecodedTelemetryRecord[];
}

/** Cold decoder. The caller supplies a finite record limit before allocation. */
export function decodeTelemetryBatch(input: Uint8Array, maxRecords: number): DecodedTelemetryBatch {
  if (!Number.isSafeInteger(maxRecords) || maxRecords < 0) throw new RangeError('Invalid record limit');
  if (input.length < TELEMETRY_BATCH_HEADER_BYTES) throw new Error('Short DGTB header');
  if (input[0] !== 0x44 || input[1] !== 0x47 || input[2] !== 0x54 || input[3] !== 0x42) throw new Error('DGTB magic mismatch');
  const view = new DataView(input.buffer, input.byteOffset, input.byteLength);
  if (view.getUint16(4, true) !== TELEMETRY_BATCH_VERSION) throw new Error('Unsupported DGTB version');
  if (view.getUint16(6, true) !== TELEMETRY_BATCH_HEADER_BYTES) throw new Error('Invalid DGTB header length');
  const flags = view.getUint32(20, true);
  const count = view.getUint32(24, true);
  if ((flags & ~1) !== 0 || view.getUint32(28, true) !== 0) throw new Error('Invalid DGTB flags');
  if (count > maxRecords) throw new RangeError('DGTB record capacity exceeded');
  if (input.length !== TELEMETRY_BATCH_HEADER_BYTES + count * TELEMETRY_RECORD_BYTES) throw new Error('DGTB length mismatch');
  const session: [number, number, number, number] = [view.getUint32(40, true), view.getUint32(44, true), view.getUint32(48, true), view.getUint32(52, true)];
  const generation = view.getUint32(56, true);
  const clockGeneration = view.getUint32(60, true);
  if ((session[0] | session[1] | session[2] | session[3]) === 0 || generation === 0 || clockGeneration === 0) throw new Error('Invalid DGTB identity');
  const firstSequence = view.getBigUint64(64, true);
  const nextSequence = view.getBigUint64(72, true);
  const dropped = view.getBigUint64(80, true);
  const overwritten = view.getBigUint64(88, true);
  const ticksPerSecond = view.getBigUint64(32, true);
  if (ticksPerSecond === 0n) throw new Error('Invalid DGTB clock rate');
  if (nextSequence - firstSequence !== BigInt(count) || overwritten > firstSequence || (flags === 0 && overwritten !== 0n)) throw new Error('Invalid DGTB sequence');
  let previous = 0n;
  for (let index = 0; index < count; index++) {
    const offset = TELEMETRY_BATCH_HEADER_BYTES + index * TELEMETRY_RECORD_BYTES;
    const timestamp = view.getBigUint64(offset, true);
    const phase = view.getUint32(offset + 36, true);
    if (phase < 1 || phase > 8 || timestamp < previous) throw new Error('Invalid DGTB record');
    previous = timestamp;
  }
  const records: DecodedTelemetryRecord[] = [];
  for (let index = 0; index < count; index++) {
    const offset = TELEMETRY_BATCH_HEADER_BYTES + index * TELEMETRY_RECORD_BYTES;
    records.push({ timestamp: view.getBigUint64(offset, true), correlation: view.getBigUint64(offset + 8, true),
      argument0: view.getBigUint64(offset + 16, true), argument1: view.getBigUint64(offset + 24, true),
      descriptor: view.getUint32(offset + 32, true), phase: view.getUint8(offset + 36) });
  }
  return { identity: { session, sourceId: view.getUint32(8, true), generation, clockDomain: view.getUint32(16, true), clockGeneration },
    epoch: view.getUint32(12, true), rolling: flags === 1, firstSequence, nextSequence, dropped, overwritten, ticksPerSecond, records };
}
