import { expect, test } from 'bun:test';
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const driver = fileURLToPath(new URL('./test-native-paint-capture.ts', import.meta.url));
const available = process.platform === 'linux' && Bun.which('strace') !== null;

for (const failure of [false, true]) {
  test.skipIf(!available)(`native capture closes its tracer after ${failure ? 'failure' : 'result'}`, async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'afterglow-capture-cleanup-'));
    let child: ReturnType<typeof Bun.spawn> | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const bin = path.join(root, 'target/release');
      await mkdir(bin, { recursive: true });
      const marker = failure
        ? '[paint-capture-failed] deliberate test failure'
        : '[paint-capture-ready] {}\n[paint-capture-result] {}';
      // The fixture owns these PIDs. The tracee becomes sleep without another child.
      await writeFile(path.join(bin, 'afterglow-shell'), `#!/bin/sh\nprintf '%s\\n' "$PPID" > tracer.pid\nprintf '%s\\n' "$$" > tracee.pid\nprintf '%s\\n' '${marker}'\nexec sleep 60\n`);
      await writeFile(path.join(bin, 'afterglow-collector'), '#!/bin/sh\nexit 0\n');
      await chmod(path.join(bin, 'afterglow-shell'), 0o700);
      await chmod(path.join(bin, 'afterglow-collector'), 0o700);
      child = Bun.spawn([process.execPath, driver, path.join(root, 'evidence'), '--strace'], {
        cwd: root, stdout: 'pipe', stderr: 'pipe',
      });
      const stderr = new Response(child.stderr).text();
      const stdout = new Response(child.stdout).text();
      const exit = await Promise.race([
        child.exited,
        new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('Capture cleanup exceeded 10 seconds')), 10_000); }),
      ]);
      expect(exit).not.toBe(0);
      expect(await stderr).toContain(failure ? 'deliberate test failure' : 'Expected one completed capture');
      await stdout;
    } finally {
      clearTimeout(timer);
      child?.kill('SIGKILL');
      if (child) await child.exited;
      for (const name of ['tracer.pid', 'tracee.pid']) {
        try {
          const pid = Number((await readFile(path.join(root, name), 'utf8')).trim());
          if (Number.isSafeInteger(pid) && pid > 1) process.kill(pid, 'SIGKILL');
        } catch (error) {
          if (!['ESRCH', 'ENOENT'].includes((error as NodeJS.ErrnoException).code ?? '')) throw error;
        }
      }
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);
}
