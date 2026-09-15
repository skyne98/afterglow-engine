import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { createServer } from 'node:net';
import { decodeTelemetryBatch } from '../crates/afterglow-telemetry/web/src/batch.ts';

// Run inside shell.nix after the release binaries and Vite page build.
// The optional --control argument keeps the server absent during the same workload.
const control = process.argv[3] === '--control';
const strace = process.argv[3] === '--strace';
if (process.argv.length > 4 || (process.argv[3] !== undefined && !control && !strace)) throw new Error('Expected [directory] [--control|--strace]');
const directory = path.resolve(process.argv[2] ?? `docs/benchmarks/native-paint-shell/profiling-${Date.now()}`);
await mkdir(directory, { recursive: true });
const reservation = createServer();
await new Promise<void>((resolve, reject) => { reservation.once('error', reject); reservation.listen(0, '127.0.0.1', resolve); });
const port = (reservation.address() as { port: number }).port;
await new Promise<void>((resolve, reject) => reservation.close(error => error ? reject(error) : resolve()));
const address = `127.0.0.1:${port}`;
const page = control ? 'paint-capture-control.html' : 'paint-capture-probe.html';
// Syscall tracing changes timing. Use it only to locate waits, not for overhead acceptance.
const prefix = strace ? ['strace', '--interruptible=1', '--kill-on-exit', '--syscall-limit=250000', '--decode-fds=path,socket', '-ttt', '-T',
  '-e', 'trace=futex,ioctl,poll,ppoll,epoll_wait,epoll_pwait,clock_nanosleep', '-e', 'raw=ioctl',
  '-o', path.join(directory, 'syscalls.log'), '--'] : [];
const host = Bun.spawn([...prefix, './target/release/afterglow-shell', '--document', `prototype/character-editor/dist/paint/${page}`], {
  env: { ...process.env, AFTERGLOW_DIAGNOSTICS_PORT: String(port), AFTERGLOW_STARTUP_TIMEOUT_MS: '120000', AFTERGLOW_CAPTURE_PATH: undefined },
  stdout: 'pipe', stderr: 'pipe',
});
let viewer: ReturnType<typeof Bun.spawn> | undefined;
let output = '', result = '', nativeReady = false;
const deadline = setTimeout(() => { host.kill('SIGKILL'); viewer?.kill('SIGKILL'); }, 180_000);
try {
  async function collect(stream: ReadableStream<Uint8Array>) {
    let tail = '';
    for await (const chunk of stream) {
      const text = new TextDecoder().decode(chunk);
      output += text;
      process.stdout.write(text);
      tail += text;
      let end: number;
      while ((end = tail.indexOf('\n')) >= 0) {
        const line = tail.slice(0, end);
        tail = tail.slice(end + 1);
        if (line.includes('afterglow-shell game ready after')) nativeReady = true;
        if (line.includes('[paint-capture-ready]') && !nativeReady) throw new Error('Capture started before native input readiness');
        if (!control && line.includes('[paint-capture-ready]')) {
          if (viewer) throw new Error('Invalid profiling startup');
          viewer = Bun.spawn(['./target/release/afterglow-collector', 'capture', address, directory, '15', '16777216'], { stdout: 'pipe', stderr: 'pipe' });
        }
        if (line.includes('[paint-capture-failed]')) throw new Error(line);
        if (line.includes('[paint-capture-result]')) {
          result = line.slice(line.indexOf('{'));
          if (!control) {
            if (!viewer) throw new Error('Missing viewer');
            if (await viewer.exited !== 0) throw new Error(await new Response(viewer.stderr).text());
          }
          host.kill();
        }
      }
    }
  }
  await Promise.all([collect(host.stdout), collect(host.stderr)]);
  if (!result) throw new Error('Missing paint result');
  const files = (await readdir(directory)).filter(name => name.endsWith('.dgtl'));
  if (control) {
    if (files.length !== 0) throw new Error('Control run must not create a capture');
    await writeFile(path.join(directory, 'result.json'), JSON.stringify(JSON.parse(result), null, 2));
    console.log('[native-paint-control] PASS', directory);
  } else {
  if (files.length !== 1) throw new Error('Expected one completed capture');
  const bytes = await readFile(path.join(directory, files[0]!));
  let cursor = 0;
  function take(count: number) {
    if (!Number.isSafeInteger(count) || count < 0 || cursor + count > bytes.length) throw new Error('Invalid DGTL range');
    const value = bytes.subarray(cursor, cursor + count); cursor += count; return value;
  }
  const u32 = () => take(4).readUInt32LE();
  const text = () => take(u32()).toString('utf8');
  const header = take(28);
  if (header.toString('ascii', 0, 4) !== 'DGTL' || header.readUInt16LE(4) !== 1) throw new Error('Invalid DGTL header');
  const sources = u32();
  if (sources !== 2) throw new Error(`Expected shell and paint sources, got ${sources}`);
  const summary: unknown[] = [];
  const names = new Set<string>();
  for (let source = 0; source < sources; source++) {
    const sourceId = u32(); take(12);
    const name = text(); names.add(name);
    take(44);
    const descriptors: string[] = [];
    for (let left = u32(); left; left--) {
      text(); descriptors.push(text()); take(2);
      for (let argument = 0; argument < 2; argument++) { text(); take(2); }
    }
    for (let left = u32(); left; left--) { text(); text(); take(2); }
    const counts = new Map<string, number>();
    let next = 0n, dropped = 0n, overwritten = 0n, pairs = 0;
    const pending = new Set<bigint>();
    for (let left = u32(); left; left--) {
      const batch = decodeTelemetryBatch(take(u32()), 1024);
      if (batch.identity.sourceId !== sourceId || batch.firstSequence !== next) throw new Error('Capture sequence mismatch');
      next = batch.nextSequence; dropped = batch.dropped; overwritten = batch.overwritten;
      for (const record of batch.records) {
        const descriptor = descriptors[record.descriptor];
        if (!descriptor) throw new Error('Unknown capture descriptor');
        counts.set(descriptor, (counts.get(descriptor) ?? 0) + 1);
        if (record.phase === 4) pending.add(record.correlation);
        if (record.phase === 5 && pending.delete(record.correlation)) pairs++;
      }
    }
    take(u32() * 24);
    if (dropped || overwritten) throw new Error(`Capture lost records: ${name}`);
    if (name === 'afterglow-shell') {
      if (!(counts.get('host.present')! > 100) || pairs < 10) throw new Error('Missing host frames or RPC pairs');
      for (const descriptor of ['host.frame', 'host.runtime_turn', 'host.present_work', 'host.redraw_request', 'host.hud_scene', 'host.hud_composite', 'host.surface_present']) {
        if (!(counts.get(descriptor)! > 100)) throw new Error(`Missing host measurement: ${descriptor}`);
      }
      for (const descriptor of ['host.focus', 'host.occluded', 'host.suspended']) {
        if (!(counts.get(descriptor)! >= 1)) throw new Error(`Missing window state: ${descriptor}`);
      }
    }
    if (name === 'afterglow-engine.paint' && (!(counts.get('paint.stroke_sample')! >= 40) || !(counts.get('paint.publish')! > 0))) throw new Error('Missing paint work');
    summary.push({ name, records: Number(next), counts: Object.fromEntries(counts), pairs, dropped: Number(dropped), overwritten: Number(overwritten) });
  }
  if (cursor !== bytes.length || !names.has('afterglow-shell') || !names.has('afterglow-engine.paint')) throw new Error('Invalid capture contents');
  await writeFile(path.join(directory, 'result.json'), JSON.stringify({ ...JSON.parse(result), syscallTrace: strace, captureFile: files[0], sources: summary }, null, 2));
  console.log('[native-paint-capture] PASS', directory);
  }
} finally {
  clearTimeout(deadline);
  host.kill('SIGKILL'); viewer?.kill('SIGKILL');
  await host.exited;
  await writeFile(path.join(directory, 'host.log'), output);
}
