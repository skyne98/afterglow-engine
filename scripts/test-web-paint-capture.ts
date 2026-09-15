// Run after the release collector and character-editor Vite build.
// The test owns its browser session, HTTP server, collector, and output directory.
// The pixel probe checks paint output. This transport test does not need a screenshot.
import { createServer } from 'node:net';
import { mkdir } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('prototype/character-editor/dist');
const directory = resolve(process.argv[2] ?? `docs/benchmarks/native-paint-shell/profiling-web-${Date.now()}`);
await mkdir(directory, { recursive: true });
const reservation = createServer();
await new Promise<void>((resolve, reject) => { reservation.once('error', reject); reservation.listen(0, '127.0.0.1', resolve); });
const port = (reservation.address() as { port: number }).port;
await new Promise<void>((resolve, reject) => reservation.close(error => error ? reject(error) : resolve()));
const server = Bun.serve({ hostname: '127.0.0.1', port: 0, async fetch(request) {
  let path: string;
  try { path = resolve(root, '.' + decodeURIComponent(new URL(request.url).pathname)); }
  catch { return new Response('Invalid path', { status: 400 }); }
  if (!path.startsWith(root + sep)) return new Response('Not found', { status: 404 });
  const file = Bun.file(path);
  if (!await file.exists()) return new Response('Not found', { status: 404 });
  return new Response(file, { headers: { 'Cross-Origin-Opener-Policy': 'same-origin',
    'Cross-Origin-Embedder-Policy': 'require-corp', 'Cache-Control': 'no-store' } });
} });
const session = `afterglow-paint-capture-${process.pid}`;
const url = `http://127.0.0.1:${server.port}/paint/paint-capture-probe.html?target=web&profilingServer=${encodeURIComponent(`ws://127.0.0.1:${port}/`)}`;
let collector: ReturnType<typeof Bun.spawn> | null = null;
let browserCommand: ReturnType<typeof Bun.spawn> | null = null;
const deadline = setTimeout(() => { collector?.kill(); browserCommand?.kill(); }, 180_000);
async function browser(...args: string[]) {
  const child = Bun.spawn(['agent-browser', '--fresh', '--session', session, ...args], {
    stdout: 'pipe', stderr: 'pipe', env: { ...process.env, AGENT_BROWSER_DEFAULT_TIMEOUT: '60000' },
  });
  browserCommand = child;
  const [stdout, stderr, code] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
  browserCommand = null;
  if (code !== 0) throw new Error(`Browser command failed: ${stderr}\n${stdout}`);
  return stdout;
}
async function readProbe(field: string) {
  const response = JSON.parse(await browser('eval', '--json', `(async () => {
    const deadline = performance.now() + 55000;
    while (!window.${field}) {
      if (window.__paintCaptureError) throw new Error(window.__paintCaptureError);
      if (performance.now() > deadline) throw new Error('Browser capture deadline');
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    return JSON.stringify(window.${field});
  })()`));
  if (!response.success) throw new Error(JSON.stringify(response));
  return typeof response.data.result === 'string' ? JSON.parse(response.data.result) : response.data.result;
}
try {
  await browser('tab', 'new', url);
  await readProbe('__paintCaptureReady');
  collector = Bun.spawn(['target/release/afterglow-collector', 'capture', `127.0.0.1:${port}`, directory, '15', '16777216'], { stdout: 'pipe', stderr: 'pipe' });
  const output = new Response(collector.stdout).text();
  const errors = new Response(collector.stderr).text();
  const probe = await readProbe('__paintCaptureResult');
  const code = await collector.exited;
  const stdout = await output, stderr = await errors;
  await Bun.write(resolve(directory, 'collector.log'), stdout + stderr);
  if (code !== 0) throw new Error(`Collector failed: ${stderr}`);
  const capture = stdout.trim().split('\n').map(line => JSON.parse(line)).find(value => value.capture);
  if (!capture) throw new Error('Missing capture path');
  const summaryCommand = Bun.spawn(['target/release/afterglow-collector', 'summary', capture.capture], { stdout: 'pipe', stderr: 'pipe' });
  const [summaryText, summaryErrors, summaryCode] = await Promise.all([new Response(summaryCommand.stdout).text(), new Response(summaryCommand.stderr).text(), summaryCommand.exited]);
  if (summaryCode !== 0) throw new Error(`Capture validation failed: ${summaryErrors}`);
  const summary = JSON.parse(summaryText);
  const source = summary.sources[0];
  if (summary.sources.length !== 1 || source.name !== 'afterglow-engine.paint'
      || source.dropped_records !== '0' || source.overwritten_records !== '0' || source.sequence_gaps !== '0'
      || probe.target !== 'web' || probe.stats.failures !== 0) throw new Error('Invalid browser capture identity or loss counters');
  const samples = source.descriptors.find((site: { name: string }) => site.name === 'paint.stroke_sample');
  const replies = source.descriptors.find((site: { name: string }) => site.name === 'paint.worker_message');
  if (!(samples?.phase_counts[1] >= 40) || !(replies?.phase_counts[1] > 0)) throw new Error('Missing browser paint records');
  await Bun.write(resolve(directory, 'summary.json'), summaryText);
  await Bun.write(resolve(directory, 'result.json'), JSON.stringify({ ...probe, captureFile: capture, source }, null, 2) + '\n');
  console.log(JSON.stringify({ directory, records: source.records, probe }));
} finally {
  clearTimeout(deadline);
  if (collector && collector.exitCode === null) { collector.kill(); await collector.exited; }
  await browser('close').catch(error => console.error(error));
  server.stop(true);
}
