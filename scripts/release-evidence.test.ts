import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { validateReleaseEvidence } from './check-release-evidence.ts';

const roots: string[] = [];
afterEach(async () => { while (roots.length) { const root = roots.pop(); if (root) await rm(root, { recursive: true, force: true }); } });

async function fixture(): Promise<{ root: string; hash: string }> {
  const root = await mkdtemp(join(tmpdir(), 'afterglow-release-')); roots.push(root);
  const www = join(root, 'crates/afterglow-web/www');
  await mkdir(www, { recursive: true }); await mkdir(join(root, 'docs/benchmarks'), { recursive: true });
  await writeFile(join(www, 'dungeon.js'), 'artifact');
  await writeFile(join(www, 'web-artifacts.json'), JSON.stringify({ version: 1, artifacts: [
    { source: 'dungeon.ts', output: 'dungeon.js', role: 'visual-demo', pages: ['dungeon.html'], architectureChecked: true },
  ] }));
  const hash = new Bun.CryptoHasher('sha256').update('artifact').digest('hex');
  return { root, hash };
}

describe('release evidence gate', () => {
  test('requires an evidence document', async () => {
    const { root } = await fixture();
    expect(await validateReleaseEvidence(root)).toEqual([
      'release evidence is missing or malformed: docs/benchmarks/release-evidence.json',
    ]);
  });
  test('accepts current matching GPU and soak evidence', async () => {
    const { root, hash } = await fixture(), capturedAt = '2026-07-18T12:00:00.000Z';
    await writeFile(join(root, 'docs/benchmarks/release-evidence.json'), JSON.stringify({
      version: 1,
      gpu: [{ demo: 'dungeon.ts', artifact: 'dungeon.js', sha256: hash, capturedAt, adapter: 'amd rdna2', driver: 'RADV', ok: true }],
      dungeonSoaks: ['stable', 'traverse', 'thrash'].map((mode) => ({
        mode, artifact: 'dungeon.js', sha256: hash, capturedAt,
        durationSeconds: 600, errors: 0, queueOverflows: 0, pendingAtEnd: 0,
      })),
    }));
    expect(await validateReleaseEvidence(root, new Date('2026-07-19T00:00:00.000Z'))).toEqual([]);
  });
  test('rejects stale hashes and failed soak state', async () => {
    const { root } = await fixture(), capturedAt = '2026-07-18T12:00:00.000Z';
    await writeFile(join(root, 'docs/benchmarks/release-evidence.json'), JSON.stringify({
      version: 1,
      gpu: [{ demo: 'dungeon.ts', artifact: 'dungeon.js', sha256: 'bad', capturedAt, adapter: 'amd', driver: 'radv', ok: true }],
      dungeonSoaks: ['stable', 'traverse', 'thrash'].map((mode) => ({ mode, artifact: 'dungeon.js', sha256: 'bad', capturedAt, durationSeconds: 60, errors: 1, queueOverflows: 1, pendingAtEnd: 1 })),
    }));
    const errors = await validateReleaseEvidence(root, new Date('2026-07-19T00:00:00.000Z'));
    expect(errors.some((error) => error.includes('artifact hash is stale'))).toBe(true);
    expect(errors.some((error) => error.includes('at least 600 seconds'))).toBe(true);
    expect(errors.some((error) => error.includes('ended with errors'))).toBe(true);
  });
});
