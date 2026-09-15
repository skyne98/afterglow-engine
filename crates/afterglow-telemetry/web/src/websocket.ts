import type { CaptureClient } from './capture.ts';

const MAX_MESSAGE_BYTES = 65_536;
const EMPTY_STATUS = '{"connected":false}';
const EMPTY_BYTES = new Uint8Array(0);

/** Cold browser I/O adapter. Applications select its Worker and own the recorder. */
export class WebSocketCaptureClient implements CaptureClient {
  private socket: WebSocket | null = null;
  private readonly session = new Uint32Array(4);
  private readonly encoder = new TextEncoder();
  private epoch = 0;
  private ready = false;
  private retryAt = 0;
  private timeout: ReturnType<typeof setTimeout> | undefined;

  constructor(private readonly url = 'ws://127.0.0.1:8086/',
    private readonly openSocket: (url: string) => WebSocket = url => new WebSocket(url)) {
    const endpoint = new URL(url);
    if (endpoint.protocol !== 'ws:' || !['127.0.0.1', '[::1]', 'localhost'].includes(endpoint.hostname)
      || endpoint.pathname !== '/' || endpoint.username || endpoint.password || endpoint.search || endpoint.hash) {
      throw new Error('Profiling needs a local WebSocket URL');
    }
    crypto.getRandomValues(this.session);
    if (this.session.every(value => value === 0)) throw new Error('Invalid profiling session');
  }

  async start(): Promise<string> {
    if (!this.socket && performance.now() >= this.retryAt && this.epoch < 0xffffffff) {
      this.retryAt = performance.now() + 1000;
      try {
        const socket = this.openSocket(this.url);
        this.socket = socket;
        socket.binaryType = 'arraybuffer';
        socket.onopen = this.onOpen;
        socket.onclose = this.onClose;
        socket.onerror = this.onClose;
        socket.onmessage = this.onClose;
        this.timeout = setTimeout(this.expire, 5000);
      } catch { this.disconnect(); }
    }
    if (!this.ready) return EMPTY_STATUS;
    return JSON.stringify({ connected: true, session: Array.from(this.session), epoch: this.epoch,
      nativeTick: String(Math.floor(performance.now() * 1e6)), maxFrameBytes: MAX_MESSAGE_BYTES, maxBatchRecords: 1024 });
  }

  async ingest(epoch: number, opcode: number, payload: Uint8Array): Promise<number> {
    if (!Number.isInteger(epoch) || ![1, 2, 3].includes(opcode) || payload.byteLength >= MAX_MESSAGE_BYTES) {
      throw new Error('Invalid profiling message');
    }
    if (!this.ready || epoch !== this.epoch) return 2;
    return this.send(opcode, payload) ? 0 : 2;
  }

  async finish(): Promise<string> {
    if (this.ready) this.send(4, EMPTY_BYTES);
    this.disconnect();
    return EMPTY_STATUS;
  }

  private readonly onOpen = (event: Event): void => {
    if (event.target !== this.socket) return;
    clearTimeout(this.timeout);
    this.timeout = undefined;
    this.epoch++;
    const hello = this.encoder.encode(JSON.stringify({ protocol: 3, session: Array.from(this.session),
      epoch: this.epoch, max_frame_bytes: MAX_MESSAGE_BYTES }));
    this.ready = this.send(0, hello);
  };
  private readonly onClose = (event: Event): void => {
    if (event.target === this.socket) this.disconnect();
  };
  private readonly expire = (): void => { this.disconnect(); };

  private send(opcode: number, payload: Uint8Array): boolean {
    const socket = this.socket;
    if (!socket || socket.readyState !== 1 || socket.bufferedAmount + payload.byteLength + 1 > MAX_MESSAGE_BYTES) {
      this.disconnect(); return false;
    }
    const bytes = new Uint8Array(1 + payload.byteLength);
    bytes[0] = opcode;
    bytes.set(payload, 1);
    try { socket.send(bytes); return true; }
    catch { this.disconnect(); return false; }
  }

  private disconnect(): void {
    this.ready = false;
    clearTimeout(this.timeout);
    this.timeout = undefined;
    const socket = this.socket;
    this.socket = null;
    this.retryAt = performance.now() + 1000;
    if (socket) {
      socket.onopen = socket.onclose = socket.onerror = socket.onmessage = null;
      try { socket.close(); } catch { /* Capture failure does not stop the application. */ }
    }
  }
}
