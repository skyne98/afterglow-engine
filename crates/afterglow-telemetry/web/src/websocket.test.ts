import { expect, test } from 'bun:test';
import { WebSocketCaptureClient } from './websocket.ts';

class FakeSocket {
  readyState = 1;
  bufferedAmount = 0;
  binaryType = '';
  onopen: ((event: Event) => void) | null = null;
  onclose: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: Event) => void) | null = null;
  messages: Uint8Array[] = [];
  closed = false;
  failSend = false;
  send(bytes: Uint8Array) { if (this.failSend) throw new Error('Closed'); this.messages.push(bytes.slice()); }
  close() { this.closed = true; }
  opened() { this.onopen?.({ target: this } as unknown as Event); }
}
function setup() {
  const socket = new FakeSocket();
  let calls = 0;
  const client = new WebSocketCaptureClient('ws://127.0.0.1:8086/', () => { calls++; return socket as unknown as WebSocket; });
  return { client, socket, calls: () => calls };
}
test('browser application connects outward and sends the same protocol header', async () => {
  const { client, socket, calls } = setup();
  expect(JSON.parse(await client.start()).connected).toBe(false);
  await client.start(); expect(calls()).toBe(1);
  socket.opened();
  const status = JSON.parse(await client.start());
  const hello = JSON.parse(new TextDecoder().decode(socket.messages[0]!.subarray(1)));
  expect(socket.messages[0]![0]).toBe(0);
  expect(hello).toEqual({ protocol: 3, session: status.session, epoch: 1, max_frame_bytes: 65_536 });
  expect(status.connected).toBe(true);
  expect(await client.ingest(2,2,new Uint8Array([1]))).toBe(2);
  expect(await client.ingest(1,2,new Uint8Array([1,2]))).toBe(0);
  expect(Array.from(socket.messages[1]!)).toEqual([2,1,2]);
  await client.finish();
  expect(Array.from(socket.messages[2]!)).toEqual([4]);
  expect(socket.closed).toBe(true);
  expect(JSON.parse(await client.start()).connected).toBe(false);
  expect(calls()).toBe(1);
});
test('socket backpressure and failures stop only capture', async () => {
  for (const failure of ['capacity','send','incoming']) {
    const { client, socket } = setup();
    await client.start(); socket.opened();
    if (failure === 'capacity') socket.bufferedAmount = 65_536;
    if (failure === 'send') socket.failSend = true;
    if (failure === 'incoming') socket.onmessage?.({ target: socket } as unknown as Event);
    expect(await client.ingest(1,2,new Uint8Array([1]))).toBe(2);
    expect(socket.closed).toBe(true);
    expect(JSON.parse(await client.start()).connected).toBe(false);
    await client.finish();
  }
});
test('invalid URLs and payload sizes are rejected', async () => {
  for (const url of ['ws://example.org/', 'http://localhost/', 'ws://localhost/private', 'ws://name:password@localhost/']) {
    expect(() => new WebSocketCaptureClient(url)).toThrow();
  }
  const { client } = setup();
  await expect(client.ingest(1,2,new Uint8Array(65_536))).rejects.toThrow();
  await expect(client.ingest(1,55,new Uint8Array())).rejects.toThrow();
  await client.finish();
});
