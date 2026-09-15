import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

// This diagnostic moves only a verified child window to an empty workspace.
const mode = process.argv[3];
if (process.argv.length < 3 || process.argv.length > 4 || (mode !== undefined && mode !== '--control' && mode !== '--strace')) throw new Error('Expected an output directory and optional --control or --strace');
const directory = path.resolve(process.argv[2]!);
await mkdir(directory, { recursive: true });
const events: unknown[] = [];
function niri(...args: string[]) {
  const result = Bun.spawnSync(['niri', 'msg', ...args]);
  if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  return result.stdout.toString();
}
const wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
const driver = Bun.spawn(['bun', 'scripts/test-native-paint-capture.ts', directory, ...(mode ? [mode] : [])], { stdout: 'pipe', stderr: 'inherit', detached: true });
let visibility: Promise<void> | undefined;
let failure: unknown;
function stop() {
  try { process.kill(-driver.pid, 'SIGKILL'); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
}
const deadline = setTimeout(stop, 190_000);
async function changeVisibility() {
  await wait(2500);
  const children = new Set((await readFile(`/proc/${driver.pid}/task/${driver.pid}/children`, 'utf8')).trim().split(/\s+/).map(Number));
  const windows = JSON.parse(niri('--json', 'windows')) as { id: number; pid: number; workspace_id: number }[];
  // With strace, the host is one process below the direct child.
  if (mode === '--strace') {
    for (const pid of [...children]) {
      const nested = (await readFile(`/proc/${pid}/task/${pid}/children`, 'utf8')).trim();
      if (nested) for (const child of nested.split(/\s+/)) children.add(Number(child));
    }
  }
  const owned = windows.filter(window => children.has(window.pid));
  if (owned.length !== 1) throw new Error('Expected one verified child window');
  const window = owned[0]!;
  const workspaces = JSON.parse(niri('--json', 'workspaces')) as { id: number; idx: number; output: string; is_active: boolean; active_window_id: number | null }[];
  const original = workspaces.find(workspace => workspace.id === window.workspace_id);
  const empty = workspaces.find(workspace => workspace.output === original?.output && !workspace.is_active && workspace.active_window_id === null);
  if (!original?.is_active || !empty || !Number.isSafeInteger(window.id)) throw new Error('No safe empty workspace');
  events.push({ action: 'hide', observer_ms: performance.now(), window: window.id, pid: window.pid, original_workspace: original.id, hidden_workspace: empty.id });
  try {
    niri('action', 'move-window-to-workspace', '--window-id', String(window.id), '--focus', 'false', String(empty.idx));
    await wait(4500);
  } finally {
    const current = JSON.parse(niri('--json', 'windows')) as typeof windows;
    if (current.some(item => item.id === window.id && item.pid === window.pid)) {
      const spaces = JSON.parse(niri('--json', 'workspaces')) as typeof workspaces;
      const destination = spaces.find(item => item.id === original.id);
      if (!destination) throw new Error('Original workspace is absent');
      niri('action', 'move-window-to-workspace', '--window-id', String(window.id), '--focus', 'false', String(destination.idx));
      events.push({ action: 'restore', observer_ms: performance.now(), window: window.id });
    }
  }
}
try {
  let tail = '';
  for await (const chunk of driver.stdout) {
    const text = new TextDecoder().decode(chunk);
    process.stdout.write(text);
    tail += text;
    let end: number;
    while ((end = tail.indexOf('\n')) >= 0) {
      const line = tail.slice(0, end); tail = tail.slice(end + 1);
      if (line.includes('[paint-capture-ready]')) {
        if (visibility) throw new Error('Repeated readiness message');
        visibility = changeVisibility().catch(error => { failure = error; });
      }
    }
  }
  await visibility;
  if (failure) throw failure;
  if (!visibility || await driver.exited !== 0) throw new Error('Visibility capture failed');
} finally {
  await visibility;
  clearTimeout(deadline);
  stop();
  await writeFile(path.join(directory, 'visibility.json'), JSON.stringify({ mode: mode ?? 'capture', events, error: failure ? String(failure) : null }, null, 2));
}
