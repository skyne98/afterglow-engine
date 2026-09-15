import { afterAll, expect, mock, test } from 'bun:test';

const originalDeno = (globalThis as any).Deno;
const originalImageData = globalThis.ImageData;
const commands: any[] = [];
let color = 0;
let onCommand: ((command: any) => void) | null = null;
let commandResponse: ((command: any) => object | null) | null = null;
let readTile: ((tx: number, ty: number) => Promise<Uint8Array>) | null = null;
const state = { width: 64, height: 64, displayScale: 1, layers: [], groups: [] };
mock.module('../../../crates/afterglow-web/web/src/workers/paint.client.ts', () => ({
  PaintClient: class {
    async command(encoded: string) {
      const command = JSON.parse(encoded);
      commands.push(command);
      onCommand?.(command);
      if (command.cmd === 'strokeSample') color = command.x;
      if (command.cmd === 'strokeBatch') color = command.samples.at(-1).x;
      return JSON.stringify(commandResponse?.(command) ?? { state, dirty: [0, 0, state.width, state.height] });
    }
    async tile(_layer: number, tx: number, ty: number) {
      if (readTile) return readTile(tx, ty);
      const bytes = new Uint8Array(16384); bytes[0] = color; bytes[3] = 255; return bytes;
    }
    async writeTile(_layer: number, _tx: number, _ty: number, bytes: Uint8Array) {
      color = bytes[0];
      return JSON.stringify({ state, dirty: [0, 0, 64, 64] });
    }
  },
}));
(globalThis as any).Deno = { core: { ops: { op_afterglow_worker_ids: () => [3] } } };
(globalThis as any).ImageData = class {
  data: Uint8ClampedArray;
  constructor(public width: number, public height: number) { this.data = new Uint8ClampedArray(width * height * 4); }
};
const { NativePaint } = await import('../../../crates/afterglow-web/web/src/engine/paint/native-paint.ts');
afterAll(() => { (globalThis as any).Deno = originalDeno; globalThis.ImageData = originalImageData; mock.restore(); });

function setup() {
  let commits = 0;
  const pixels = new Uint8ClampedArray(16384);
  const placements: number[][] = [];
  const regions: number[][] = [];
  const context = {
    putImageData(image: ImageData, x: number, y: number) { pixels.set(image.data); placements.push([x, y, image.data[0]]); },
    getImageData() { return { data: pixels }; },
    commit(...region: number[]) { commits++; regions.push(region); },
  };
  const canvas = { width: 64, height: 64, getContext: () => context } as unknown as HTMLCanvasElement;
  return { paint: new NativePaint(canvas), pixels, placements, regions, commits: () => commits };
}

function probe(paint: InstanceType<typeof NativePaint>): Promise<any> {
  return new Promise((resolve, reject) => {
    paint.onmessage = event => { if (event.data.type === 'probeResult') resolve(event.data); };
    paint.onerror = event => reject(new Error(event.message));
    paint.send({ cmd: 'probe', id: 1, y: 0 });
  });
}

test('native paint captures reused input and publishes RGBA before probe', async () => {
  commands.length = 0;
  const { paint, pixels, commits } = setup();
  paint.send({ cmd: 'setView', zoom: 1 });
  paint.send({ cmd: 'init', width: 64, height: 64 });
  const sample = { cmd: 'strokeSample', x: 42 };
  paint.send(sample);
  sample.x = 99;
  const result = await probe(paint);
  expect(commands.map(command => command.cmd)).toEqual(['init', 'strokeSample']);
  expect(commands[1].x).toBe(42);
  expect(pixels[0]).toBe(42);
  expect(commits()).toBeGreaterThan(0);
  expect(result.native).toBe(true);
});

test('native tile input owns its bytes', async () => {
  const { paint, pixels } = setup();
  paint.send({ cmd: 'init', width: 64, height: 64 });
  const bytes = new Uint8Array(16384);
  bytes[0] = 27;
  paint.send({ cmd: 'writeTile', layer: 0, tx: 0, ty: 0, data: bytes.buffer });
  bytes[0] = 81;
  await probe(paint);
  expect(pixels[0]).toBe(27);
});

test('native raster reads are bounded and retain tile order after reverse completion', async () => {
  state.width = 640;
  state.height = 128;
  let active = 0, maximum = 0;
  const pending: (() => void)[] = [];
  readTile = (tx, ty) => new Promise(resolve => {
    maximum = Math.max(maximum, ++active);
    pending.push(() => {
      active--;
      const bytes = new Uint8Array(16384);
      bytes[0] = ty * 10 + tx;
      resolve(bytes);
    });
  });
  try {
    const { paint, placements, commits } = setup();
    paint.send({ cmd: 'init' });
    const completed = paint.flush();
    for (const batchSize of [8, 8, 4]) {
      await new Promise(resolve => setTimeout(resolve, 0));
      expect(pending.length).toBe(batchSize);
      expect(commits()).toBe(0);
      for (const resolve of pending.splice(0).reverse()) resolve();
    }
    await completed;
    expect(maximum).toBe(8);
    expect(active).toBe(0);
    expect(commits()).toBe(1);
    expect(placements).toEqual(Array.from({ length: 20 }, (_, i) => [i % 10 * 64, Math.floor(i / 10) * 64, i]));
  } finally {
    readTile = null;
    state.width = state.height = 64;
  }
});

test('native publication clips changed display tiles at document edges', async () => {
  state.width = 260; state.height = 258; state.displayScale = 2;
  try {
    const { paint, regions } = setup();
    paint.send({ cmd: 'init' });
    await paint.flush();
    expect(regions).toEqual([[0, 0, 130, 129]]);
    commandResponse = () => ({ state, dirty: [250, 250, 10, 8] });
    paint.send({ cmd: 'requestState' });
    await paint.flush();
    expect(regions.at(-1)).toEqual([64, 64, 66, 65]);
    commandResponse = () => ({ state, dirty: [140, 140, 1, 1] });
    paint.send({ cmd: 'requestState' });
    await paint.flush();
    expect(regions.at(-1)).toEqual([64, 64, 64, 64]);
  } finally {
    commandResponse = null;
    state.width = state.height = 64; state.displayScale = 1;
  }
});

test('invalid raster bytes prevent publication and retain the failure', async () => {
  state.width = 512;
  readTile = async tx => new Uint8Array(tx === 3 ? 1 : 16384);
  try {
    const { paint, commits } = setup();
    paint.send({ cmd: 'init' });
    await expect(paint.flush()).rejects.toThrow('Invalid native paint tile');
    expect(commits()).toBe(0);
    expect(() => paint.send({ cmd: 'clear' })).toThrow('Invalid native paint tile');
  } finally {
    readTile = null;
    state.width = 64;
  }
});

test('native stroke batches keep sample order and command barriers', async () => {
  commands.length = 0;
  const { paint, pixels } = setup();
  paint.send({ cmd: 'init' });
  const sample = { cmd: 'strokeSample', x: 0 };
  for (let i = 0; i < 70; i++) { sample.x = i; paint.send(sample); }
  paint.send({ cmd: 'commit' });
  await paint.flush();
  expect(commands.map(command => command.cmd)).toEqual(['init', 'strokeBatch', 'strokeBatch', 'strokeBatch', 'commit']);
  const batches = commands.filter(command => command.cmd === 'strokeBatch');
  expect(batches.every(batch => batch.samples.length <= 32)).toBe(true);
  expect(batches.flatMap(batch => batch.samples.map((sample: any) => sample.x))).toEqual(Array.from({ length: 70 }, (_, i) => i));
  expect(pixels[0]).toBe(69);
});

test('continuous pointer input publishes each ready sample group', async () => {
  const { paint, placements } = setup();
  paint.send({ cmd: 'init' });
  await paint.flush();
  placements.length = 0;
  onCommand = command => {
    if (command.cmd === 'strokeSample' && command.x < 3) paint.send({ cmd: 'strokeSample', x: command.x + 1 });
  };
  try {
    paint.send({ cmd: 'strokeSample', x: 1 });
    await paint.flush();
    expect(placements.map(value => value[2])).toEqual([1, 2, 3]);
  } finally { onCommand = null; }
});

test('a confirmed stroke rollback publishes restored pixels and keeps commands available', async () => {
  commands.length = 0;
  color = 19;
  const { paint, pixels, commits } = setup();
  const messages: any[] = [];
  let failures = 0;
  paint.onmessage = event => messages.push(event.data);
  paint.onerror = () => { failures++; };
  paint.send({ cmd: 'init' });
  await paint.flush();
  const before = commits();
  commandResponse = command => {
    if (command.cmd !== 'strokeSample') return null;
    color = 19;
    return { state, dirty: [0, 0, 64, 64], error: 'Stroke rolled back.', documentRolledBack: true };
  };
  try {
    paint.send({ cmd: 'strokeSample', x: 80 });
    paint.send({ cmd: 'commit' });
    await paint.flush();
    expect(pixels[0]).toBe(19);
    expect(commits()).toBeGreaterThan(before);
    expect(failures).toBe(0);
    expect(messages.some(message => message.type === 'status' && message.text === 'Stroke rolled back.')).toBe(true);
    expect(messages.some(message => message.type === 'log' && message.text === 'Stroke rolled back.')).toBe(true);
    paint.send({ cmd: 'undo' });
    await paint.flush();
    expect(commands.at(-1).cmd).toBe('undo');
    commandResponse = null;
    paint.send({ cmd: 'beginStroke' });
    paint.send({ cmd: 'strokeSample', x: 42 });
    paint.send({ cmd: 'commit' });
    await paint.flush();
    expect(pixels[0]).toBe(42);
  } finally { commandResponse = null; }
});

test('an unconfirmed rollback remains a fatal paint error', async () => {
  for (const flag of [undefined, false, 'true']) {
    const { paint } = setup();
    commandResponse = () => ({ state, error: 'Unrestored stroke.', documentRolledBack: flag });
    try {
      paint.send({ cmd: 'init' });
      await expect(paint.flush()).rejects.toThrow('Unrestored stroke');
      expect(() => paint.send({ cmd: 'undo' })).toThrow('Unrestored stroke');
    } finally { commandResponse = null; }
  }
});

test('recovery does not publish pixels or ready before an explicit decision', async () => {
  for (const recovery of ['restore', 'discard']) {
    commands.length = 0;
    const { paint, commits } = setup();
    const messages: any[] = [];
    paint.onmessage = event => messages.push(event.data);
    commandResponse = command => command.cmd === 'init' && !command.recovery
      ? { recoveryRequired: true, state: null, dirty: null } : null;
    try {
      paint.send({ cmd: 'init' });
      paint.send({ cmd: 'loadBrush', json: '{}' });
      paint.send({ cmd: 'config', settings: [] });
      await paint.flush();
      expect(messages.map(message => message.type)).toEqual(['recoveryRequired']);
      expect(commits()).toBe(0);
      expect(commands.map(command => command.cmd)).toEqual(['init']);
      paint.send({ cmd: 'init', recovery });
      await paint.flush();
      expect(commands.at(-1).recovery).toBe(recovery);
      expect(messages.filter(message => message.type === 'ready')).toHaveLength(1);
      expect(commits()).toBe(1);
    } finally { commandResponse = null; }
  }
});

test('recovery blocks document writes before a decision', async () => {
  const { paint, commits } = setup();
  commandResponse = () => ({ recoveryRequired: true });
  try {
    paint.send({ cmd: 'init' });
    await paint.flush();
    paint.send({ cmd: 'writeTile', layer: 0, tx: 0, ty: 0, data: new Uint8Array(16384).buffer });
    await expect(paint.flush()).rejects.toThrow('Select Restore or Discard');
    expect(commits()).toBe(0);
  } finally { commandResponse = null; }
});

test('native paint rejects excess command bytes and disables input', () => {
  const { paint } = setup();
  let error = '';
  paint.onerror = event => { error = event.message; };
  paint.send({ cmd: 'loadBrush', json: 'x'.repeat(65536) });
  expect(error).toContain('byte limit');
  expect(() => paint.send({ cmd: 'clear' })).toThrow(error);
});
