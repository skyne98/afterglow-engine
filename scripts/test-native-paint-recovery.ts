import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createServer } from 'node:net';

const directory = path.resolve(process.argv[2] ?? 'docs/benchmarks/native-paint-shell/recovery-check');
await mkdir(directory, { recursive: true });
// This check must never open or discard user recovery data.
const storage = await mkdtemp(path.join(tmpdir(), 'afterglow-paint-recovery-'));
const results: unknown[] = [];
async function xdo(args: string[]) {
  const child = Bun.spawn(['xdotool', ...args], { stdout: 'pipe', stderr: 'pipe' });
  const [output, error, code] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
  if (code !== 0) throw new Error(`xdotool failed: ${error}`);
  return output.trim();
}
async function run(phase: 'seed' | 'restore' | 'discard' | 'raster' | 'stress') {
  const reservation = createServer();
  await new Promise<void>((resolve, reject) => { reservation.once('error', reject); reservation.listen(0, '127.0.0.1', resolve); });
  const port = (reservation.address() as { port: number }).port;
  await new Promise<void>((resolve, reject) => reservation.close(error => error ? reject(error) : resolve()));
  const env = { ...process.env, AFTERGLOW_STORAGE_ROOT: storage, AFTERGLOW_DIAGNOSTICS_PORT: String(port),
    AFTERGLOW_STARTUP_TIMEOUT_MS: '60000' };
  delete env.AFTERGLOW_CAPTURE_PATH;
  delete env.WAYLAND_DISPLAY;
  delete env.WAYLAND_SOCKET;
  const child = Bun.spawn(['./target/release/afterglow-shell', '--document',
    `prototype/character-editor/dist/paint/paint-${phase === 'raster' || phase === 'stress' ? phase : 'recovery'}-probe.html`], { env, stdout: 'pipe', stderr: 'pipe' });
  let log = '', ready = false, clicked = false, clickStarted = false, failure = '', result: any;
  let pendingPoint: { x: number; y: number } | undefined;
  const deadline = setTimeout(() => { failure = 'Native recovery deadline'; child.kill('SIGKILL'); }, phase === 'stress' ? 900_000 : 90_000);
  const memory: unknown[] = [];
  let readingMemory = false;
  const sampler = setInterval(async () => {
    if (readingMemory) return;
    readingMemory = true;
    try {
      const status = await readFile(`/proc/${child.pid}/status`, 'utf8');
      memory.push({ time: Date.now(), values: status.split('\n').filter(line => /^Vm(HWM|RSS):/.test(line)) });
    } catch { /* The child can exit between samples. */ }
    finally { readingMemory = false; }
  }, 10_000);
  async function drain(stream: ReadableStream<Uint8Array>) {
    const decoder = new TextDecoder();
    let tail = '';
    for await (const chunk of stream) {
      const text = decoder.decode(chunk, { stream: true });
      log += text;
      if (log.length > 1024 * 1024) throw new Error('Native recovery log capacity exceeded');
      process.stdout.write(text);
      tail += text;
      let end;
      while ((end = tail.indexOf('\n')) >= 0) {
        const line = tail.slice(0, end); tail = tail.slice(end + 1);
        if (line.includes('afterglow-shell game ready after')) ready = true;
        if (line.includes('[paint-recovery-failed]')) throw new Error(line);
        const click = '[paint-recovery-click] ';
        if (line.includes(click)) {
          if (phase === 'seed' || phase === 'raster' || phase === 'stress' || pendingPoint) throw new Error('Unexpected recovery prompt');
          const point = JSON.parse(line.slice(line.indexOf(click) + click.length))[phase];
          if (!point || !Number.isSafeInteger(point.x) || !Number.isSafeInteger(point.y)) throw new Error('Invalid recovery point');
          pendingPoint = point;
        }
        // The two output streams can arrive in either order. Wait for both markers.
        if (ready && pendingPoint && !clickStarted) {
          clickStarted = true;
          const point = pendingPoint;
          const windows = (await xdo(['search', '--onlyvisible', '--pid', String(child.pid)])).split('\n');
          if (windows.length !== 1 || !/^\d+$/.test(windows[0])) throw new Error('Expected one owned recovery window');
          const id = windows[0];
          await xdo(['windowactivate', '--sync', id]);
          await xdo(['mousemove', '--sync', '--window', id, String(point.x), String(point.y)]);
          const pointer = await xdo(['getmouselocation', '--shell']);
          if (!pointer.split('\n').includes(`WINDOW=${id}`)) throw new Error(`Pointer is outside the owned recovery window ${id}: ${pointer}`);
          await xdo(['click', '1']);
          clicked = true;
        }
        const marker = '[paint-recovery-result] ';
        if (line.includes(marker)) {
          result = JSON.parse(line.slice(line.indexOf(marker) + marker.length));
          if (result.phase !== phase) throw new Error(`Incorrect recovery phase: ${result.phase}`);
          if (phase === 'restore' && JSON.stringify(result.snapshot) !== JSON.stringify((results[0] as any).snapshot))
            throw new Error('Recovered pixels or composition differ from the killed process');
          result.memorySamples = memory;
          result.processMemory = (await readFile(`/proc/${child.pid}/status`, 'utf8')).split('\n').filter(line => /^Vm(HWM|RSS|Peak):/.test(line));
          // Kill without document shutdown, then reopen the same private database.
          child.kill('SIGKILL');
        }
      }
    }
  }
  try {
    await Promise.all([drain(child.stdout), drain(child.stderr), child.exited]);
    if (failure || !result || !ready || ((phase === 'restore' || phase === 'discard') && !clicked)) throw new Error(failure || 'Native recovery check incomplete');
    results.push(result);
  } finally {
    clearTimeout(deadline);
    clearInterval(sampler);
    if (child.exitCode === null) child.kill('SIGKILL');
    await child.exited;
    await writeFile(path.join(directory, `${phase}.log`), log);
    await writeFile(path.join(directory, `${phase}-memory.json`), JSON.stringify(memory, null, 2));
  }
}
try {
  if (process.argv[3] === '--stress') await run('stress');
  else if (process.argv[3] === '--raster') await run('raster');
  else for (const phase of ['seed', 'restore', 'discard'] as const) await run(phase);
  await writeFile(path.join(directory, 'result.json'), JSON.stringify({ results }, null, 2));
  console.log('[native-paint-check] PASS');
} finally {
  await rm(storage, { recursive: true, force: true });
}
