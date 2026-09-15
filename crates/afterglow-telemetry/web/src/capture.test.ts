import { describe, expect, test } from 'bun:test';
import { CapturePump, RecorderSource, type CaptureClient } from './capture.ts';
import { TelemetryRecorder, TelemetryDescriptorKind, TelemetryRecordStatus } from './telemetry.ts';

function fixture() {
  let epoch = 1, connected = true, tick = 10;
  const sent: { epoch: number; opcode: number; bytes: Uint8Array }[] = [];
  let inFlight = 0, highWater = 0;
  const client: CaptureClient = {
    async start() { inFlight++; highWater = Math.max(highWater, inFlight); await Promise.resolve(); inFlight--;
      return JSON.stringify({ connected, epoch, session: [1, 2, 3, 4], nativeTick: '20', maxFrameBytes: 65536 }); },
    async ingest(epoch, opcode, bytes) { inFlight++; highWater = Math.max(highWater, inFlight); await Promise.resolve(); inFlight--;
      sent.push({ epoch, opcode, bytes: bytes.slice() }); return 0; },
    async finish() { return ''; },
  };
  const recorder = new TelemetryRecorder([{ category: 0, categoryName: 'test', name: 'record', kind: TelemetryDescriptorKind.Instant }], new ArrayBuffer(80), () => ++tick);
  const pump = new CapturePump(client, () => tick);
  pump.add(new RecorderSource(1, 'not-an-engine', recorder));
  return { recorder, pump, client, sent, highWater: () => highWater, reconnect: () => epoch++, disconnect: () => { connected = false; } };
}

describe('bounded capture pump', () => {
  test('registration precedes batches, continuation retains sequence and losses', async () => {
    const f = fixture();
    expect(f.recorder.instant(0, 0)).toBe(TelemetryRecordStatus.Disabled);
    await f.pump.step();
    expect(f.sent[0]!.opcode).toBe(1);
    expect(JSON.parse(new TextDecoder().decode(f.sent[0]!.bytes)).name).toBe('not-an-engine');
    for (let i = 0; i < 3; i++) f.recorder.instant(0, 0);
    await f.pump.step();
    const first = new DataView(f.sent[1]!.bytes.buffer);
    expect(first.getUint32(24, true)).toBe(2);
    expect(first.getBigUint64(80, true)).toBe(1n);
    f.recorder.instant(0, 0);
    await f.pump.step();
    expect(new DataView(f.sent[2]!.bytes.buffer).getBigUint64(64, true)).toBe(2n);
    f.disconnect(); await f.pump.step();
    expect(f.recorder.instant(0, 0)).toBe(TelemetryRecordStatus.Disabled);
  });
  test('reconnection starts a new epoch and repeats registration', async () => {
    const f = fixture(); await f.pump.step(); f.recorder.instant(0, 0);
    f.reconnect(); await f.pump.step(); f.recorder.instant(0, 0); await f.pump.step();
    expect(f.sent.map(v => v.opcode)).toEqual([1, 1, 2]);
    expect(new DataView(f.sent[2]!.bytes.buffer).getUint32(12, true)).toBe(2);
    expect(new DataView(f.sent[2]!.bytes.buffer).getBigUint64(64, true)).toBe(0n);
  });
  test('idle sources do not send duplicate sequence batches', async () => {
    const f = fixture();
    await f.pump.step();
    await f.pump.step();
    await f.pump.step();
    expect(f.sent.map(v => v.opcode)).toEqual([1]);
    expect(f.pump.batches).toBe(0);
    f.recorder.instant(0, 0);
    await f.pump.step();
    expect(f.sent.map(v => v.opcode)).toEqual([1, 2]);
    expect(f.pump.batches).toBe(1);
  });
  test('an empty source cannot stop another source', async () => {
    const f = fixture();
    const recorder = new TelemetryRecorder(f.recorder.descriptors, new ArrayBuffer(80));
    f.pump.add(new RecorderSource(2, 'second', recorder));
    await f.pump.step(); await f.pump.step();
    recorder.instant(0, 0);
    await f.pump.step(); await f.pump.step();
    expect(f.sent.map(v => v.opcode)).toEqual([1, 1, 2]);
    expect(new DataView(f.sent[2]!.bytes.buffer).getUint32(8, true)).toBe(2);
  });
  test('overlapping steps do not create overlapping requests', async () => {
    const f = fixture(); await Promise.all([f.pump.step(), f.pump.step(), f.pump.step()]);
    expect(f.sent.length).toBe(1); expect(f.highWater()).toBe(1);
  });
  test('faults stop capture without escaping into the application', async () => {
    const f = fixture(); await f.pump.step();
    f.client.ingest = async () => { throw new Error('slow viewer'); };
    f.recorder.instant(0, 0);
    await f.pump.step();
    expect(f.pump.failures).toBe(1);
    expect(f.recorder.instant(0, 0)).toBe(TelemetryRecordStatus.Disabled);
    f.client.start = async () => '{'; await f.pump.step();
    expect(f.pump.failures).toBe(2);
  });
  test('source cleanup failure cannot stop the application', async () => {
    const f = fixture();
    f.pump.add({ register() { throw new Error('unavailable'); }, drain() { return new Uint8Array(); }, stop() { throw new Error('cleanup'); } });
    await f.pump.step();
    await f.pump.step();
    expect(f.pump.failures).toBeGreaterThan(0);
    expect(f.pump.connected).toBe(false);
    expect(f.recorder.instant(0, 0)).toBe(TelemetryRecordStatus.Disabled);
  });
  test('source and record capacities are explicit', () => {
    const f = fixture();
    for (let i = 0; i < 7; i++) expect(f.pump.add(new RecorderSource(i + 2, 'bounded', f.recorder))).toBe(true);
    expect(f.pump.add(new RecorderSource(9, 'overflow', f.recorder))).toBe(false);
    expect(() => new RecorderSource(1, 'large', new TelemetryRecorder([], new ArrayBuffer(1025 * 40)))).toThrow();
  });
});
