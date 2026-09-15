import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const source = new Bun.Transpiler({ loader: 'ts' }).transformSync(
  readFileSync(new URL('./paint-capture-probe.ts', import.meta.url), 'utf8')
    .replace("import './paint-main.ts';", ''),
);

for (const native of [true, false]) {
  test(`capture module completes before measurements (${native ? 'native' : 'web'})`, async () => {
    let probes = 0;
    const timers: (() => void)[] = [];
    const page: { probe(): number; __documentLoaded?: boolean } = { probe: () => ++probes };
    if (native) page.__documentLoaded = false;
    const evaluation = runInNewContext(`(async () => { ${source} })()`, {
      window: page, performance: { now: () => 0 }, console,
      setTimeout: (callback: () => void) => { timers.push(callback); return timers.length; },
      clearTimeout: () => {},
    }) as Promise<void>;
    let completed = false;
    void evaluation.then(() => { completed = true; });
    await Promise.resolve();
    await Promise.resolve();
    expect(completed).toBe(true);
    expect(probes).toBe(native ? 0 : 1);
    if (native) {
      page.__documentLoaded = true;
      timers.shift()!();
      await Promise.resolve();
      await Promise.resolve();
      expect(probes).toBe(1);
    }
  });
}
