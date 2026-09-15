import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { createServer } from 'node:net';

const directory = path.resolve(process.argv[2] ?? 'docs/benchmarks/native-paint-shell/menu-idle-check');
await mkdir(directory, { recursive: true });
const reservation = createServer();
await new Promise<void>((resolve, reject) => { reservation.once('error', reject); reservation.listen(0, '127.0.0.1', resolve); });
const port = (reservation.address() as { port: number }).port;
await new Promise<void>((resolve, reject) => reservation.close(error => error ? reject(error) : resolve()));
const env: Record<string, string | undefined> = { ...process.env, AFTERGLOW_HOST_TRACE: '1', AFTERGLOW_DIAGNOSTICS_PORT: String(port), AFTERGLOW_STARTUP_TIMEOUT_MS: '60000' };
delete env.AFTERGLOW_CAPTURE_PATH;
// The mouse check uses the child window on XWayland.
delete env.WAYLAND_DISPLAY;
delete env.WAYLAND_SOCKET;
const child = Bun.spawn(['./target/release/afterglow-shell', '--document',
  'prototype/character-editor/dist/paint/paint-menu-probe.html'], { env, stdout: 'pipe', stderr: 'pipe' });
let log = '', passed = false, ready = false, captureStarted = false, failure = '', windowId = '';
async function xdo(args: string[]) {
  const command = Bun.spawn(['xdotool', ...args], { stdout: 'pipe', stderr: 'pipe' });
  const [output, error, code] = await Promise.all([new Response(command.stdout).text(), new Response(command.stderr).text(), command.exited]);
  if (code !== 0) throw new Error(`xdotool failed: ${error}`);
  return output.trim();
}
async function ownedWindow() {
  if (!windowId) {
    const windows = (await xdo(['search', '--onlyvisible', '--pid', String(child.pid)])).split('\n');
    if (windows.length !== 1 || !/^\d+$/.test(windows[0])) throw new Error('Expected one owned native window');
    windowId = windows[0];
  }
  return windowId;
}
const deadline = setTimeout(() => { failure = 'Native menu test timeout'; child.kill(); }, 90_000);
async function drain(stream: ReadableStream<Uint8Array>) {
  const decoder = new TextDecoder();
  let tail = '';
  for await (const chunk of stream) {
    const text = decoder.decode(chunk, { stream: true });
    log += text;
    if (log.length > 1024 * 1024) { failure = 'Native menu log capacity exceeded'; child.kill(); return; }
    process.stdout.write(text);
    tail += text;
    let end;
    while ((end = tail.indexOf('\n')) >= 0) {
      const line = tail.slice(0, end); tail = tail.slice(end + 1);
      if (line.includes('afterglow-shell game ready after')) ready = true;
      if (line.includes('[paint-capture-ready]')) {
        if (!ready) throw new Error('Capture blocked native input startup');
        captureStarted = true;
      }
      if (line.includes('[paint-capture-failed]')) throw new Error(line);
      const marker = '[menu-idle-click] ';
      if (line.includes(marker)) {
        const point = JSON.parse(line.slice(line.indexOf(marker) + marker.length));
        const id = await ownedWindow();
        await xdo(['mousemove', '--sync', '--window', id, String(point.x), String(point.y)]);
        const pointer = await xdo(['getmouselocation', '--shell']);
        if (!pointer.split('\n').includes(`WINDOW=${id}`)) throw new Error('The pointer is not over the owned window');
        await xdo(['click', '1']);
      }
      if (line.includes('[menu-idle-escape]')) {
        await xdo(['windowfocus', '--sync', await ownedWindow()]);
        await xdo(['key', 'Escape']);
      }
      if (line.includes('[menu-idle] FAIL')) { failure = line; child.kill(); }
      if (line.includes('[menu-idle] PASS')) { passed = true; child.kill(); }
    }
  }
}
try {
  await Promise.all([drain(child.stdout), drain(child.stderr), child.exited]);
  if (failure || !passed || !ready || !captureStarted) throw new Error(failure || `Native menu check incomplete: ready=${ready}, captureStarted=${captureStarted}, passed=${passed}`);
} finally {
  clearTimeout(deadline);
  if (child.exitCode === null) child.kill();
  await child.exited;
  await writeFile(path.join(directory, 'host.log'), log);
}
