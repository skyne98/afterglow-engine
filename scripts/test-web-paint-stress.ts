import { mkdtemp, readFile, rm, mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

const output = path.resolve(process.argv[2] ?? 'docs/benchmarks/native-paint-shell/stress-web');
const profile = await mkdtemp(path.join(tmpdir(), 'afterglow-paint-web-'));
const root = path.resolve('prototype/character-editor/dist');
const server = Bun.serve({ hostname: '127.0.0.1', port: 0, async fetch(request) {
  const file = path.resolve(root, '.' + decodeURIComponent(new URL(request.url).pathname));
  if (!file.startsWith(root + path.sep)) return new Response('Not found', { status: 404 });
  const data = Bun.file(file);
  return new Response(await data.exists() ? data : 'Not found', { status: await data.exists() ? 200 : 404,
    headers: { 'Cross-Origin-Opener-Policy': 'same-origin', 'Cross-Origin-Embedder-Policy': 'require-corp' } });
} });
const browser = Bun.spawn(['chromium', `--user-data-dir=${profile}`, '--remote-debugging-port=0',
  '--headless=new', '--no-first-run', '--no-default-browser-check', 'about:blank'], { stdout: 'ignore', stderr: 'pipe' });
let log = '', socket: WebSocket | undefined;
const drain = (async () => { for await (const bytes of browser.stderr) { if (log.length < 1024 * 1024) log += new TextDecoder().decode(bytes); } })();
const wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
const memory: unknown[] = [];
async function processMemory(pid: number): Promise<number> {
  try {
    const pss = Number((await readFile(`/proc/${pid}/smaps_rollup`, 'utf8')).match(/^Pss:\s+(\d+)/m)?.[1] ?? 0);
    const children = (await readFile(`/proc/${pid}/task/${pid}/children`, 'utf8')).trim().split(/\s+/).filter(Boolean).map(Number);
    return pss + (await Promise.all(children.map(processMemory))).reduce((a, b) => a + b, 0);
  } catch { return 0; }
}
try {
  await mkdir(output, { recursive: true });
  let port = '';
  for (let tries = 0; tries < 150 && !port; tries++) {
    if (browser.exitCode !== null) throw new Error('Private Chromium exited before CDP');
    try { port = (await readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; } catch { await wait(100); }
  }
  if (!port) throw new Error('Private Chromium startup deadline');
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json() as any[];
  socket = new WebSocket(targets.find(target => target.type === 'page').webSocketDebuggerUrl);
  await new Promise<void>((resolve, reject) => { socket!.onopen = () => resolve(); socket!.onerror = () => reject(new Error('Private CDP connection failed')); });
  let id = 0;
  const pending = new Map<number, (message: any) => void>();
  socket.onmessage = event => { const message = JSON.parse(String(event.data)); pending.get(message.id)?.(message); };
  const call = (method: string, params: object = {}) => new Promise<any>((resolve, reject) => {
    const request = ++id;
    const timer = setTimeout(() => { pending.delete(request); reject(new Error(`CDP deadline: ${method}`)); }, 15_000);
    pending.set(request, message => { clearTimeout(timer); pending.delete(request); message.error ? reject(new Error(JSON.stringify(message.error))) : resolve(message.result); });
    socket!.send(JSON.stringify({ id: request, method, params }));
  });
  await call('Page.enable');
  await call('Runtime.enable');
  await call('Page.navigate', { url: `http://127.0.0.1:${server.port}/paint/paint-stress-probe.html` });
  let result: any;
  const start = Date.now();
  while (Date.now() - start < 900_000) {
    if (browser.exitCode !== null) throw new Error('Private Chromium exited during paint');
    const response = await call('Runtime.evaluate', { expression: 'globalThis.paintStressResult', returnByValue: true });
    result = response.result?.value;
    if (result) break;
    if (memory.length === 0 || Date.now() - (memory.at(-1) as any).time >= 10_000)
      memory.push({ time: Date.now(), processTreePssKiB: await processMemory(browser.pid) });
    await wait(1000);
  }
  if (!result || result.error) throw new Error(result?.error ?? 'Web paint stress deadline');
  result.memorySamples = memory;
  await writeFile(path.join(output, 'result.json'), JSON.stringify({ results: [result] }, null, 2));
  console.log('[web-paint-stress] PASS', JSON.stringify({ strokes: result.strokes, elapsedMs: result.elapsedMs, initialStrokeMs: result.initialStrokeMs }));
} finally {
  socket?.close(); server.stop(true);
  if (browser.exitCode === null) browser.kill('SIGTERM');
  await browser.exited; await drain;
  await mkdir(output, { recursive: true });
  await writeFile(path.join(output, 'browser.log'), log);
  await writeFile(path.join(output, 'memory.json'), JSON.stringify(memory, null, 2));
  await rm(profile, { recursive: true, force: true });
}
