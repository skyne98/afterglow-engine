import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { validateReleaseEvidence, type ReleaseTarget } from './check-release-evidence.ts';

const roots: string[] = [];
afterEach(async () => {
  while (roots.length) {
    const root = roots.pop();
    if (root) await rm(root, { recursive: true, force: true });
  }
});

async function fixture(): Promise<{ root: string; artifactHash: string; screenshotHash: string }> {
  const root = await mkdtemp(join(tmpdir(), 'afterglow-release-'));
  roots.push(root);
  const www = join(root, 'crates/afterglow-web/www');
  const contracts = join(root, 'crates/afterglow-web/web/contracts');
  const benchmarks = join(root, 'docs/benchmarks');
  await mkdir(www, { recursive: true });
  await mkdir(contracts, { recursive: true });
  await mkdir(join(benchmarks, 'screenshots'), { recursive: true });
  await writeFile(join(www, 'dungeon.js'), 'artifact');
  await writeFile(join(benchmarks, 'screenshots/dungeon.png'), 'pixels');
  await writeFile(join(contracts, 'web-artifacts.json'), JSON.stringify({ version: 1, artifacts: [
    { source: 'dungeon.ts', output: 'dungeon.js', role: 'visual-demo', pages: ['dungeon.html'], architectureChecked: true },
  ] }));
  const artifactHash = new Bun.CryptoHasher('sha256').update('artifact').digest('hex');
  const screenshotHash = new Bun.CryptoHasher('sha256').update('pixels').digest('hex');
  return { root, artifactHash, screenshotHash };
}

function validEvidence(artifactHash: string, screenshotHash: string): object {
  const capturedAt = '2026-07-18T12:00:00.000Z';
  const visual = (['web', 'native'] as const).map((target: ReleaseTarget) => ({
    demo: 'dungeon.ts', target, artifact: 'dungeon.js', sha256: artifactHash,
    capturedAt, commit: '0123456789abcdef0123456789abcdef01234567', host: 'afterglow-test',
    adapter: 'amd rdna2', driver: 'RADV',
    dimensions: {
      logicalWidth: 640, logicalHeight: 360, physicalWidth: 1280, physicalHeight: 720,
      surfaceWidth: 1280, surfaceHeight: 720, canvasWidth: 1280, canvasHeight: 720,
      feedbackWidth: 160, feedbackHeight: 90, devicePixelRatio: 2,
    },
    readiness: {
      gameReady: true, fatalDiagnostics: 0, pendingBootstrap: 0,
      stages: ['Bootstrap', 'Warmup', 'GameplaySealed', 'GameReady'],
    },
    pixels: {
      screenshot: 'screenshots/dungeon.png', screenshotSha256: screenshotHash,
      nonBackgroundFraction: 0.5, luminanceStdDev: 0.2, referenceDiff: 0.01,
      maxReferenceDiff: 0.05,
    },
    frames: { samples: 300, p99Ms: 16.7, maxMs: 20 },
    resources: { heapBytes: 1024, gpuBytes: 2048 },
    queues: { overflows: 0, pendingAtEnd: 0 },
  }));
  const scenarios = ['dungeon-streaming', 'mutable-painting', 'rigged-model', 'combined', 'capacity-thrash'];
  const soaks = scenarios.flatMap((scenario) => (['web', 'native'] as const).map((target) => ({
    scenario, target, artifact: 'dungeon.js', sha256: artifactHash, capturedAt,
    durationSeconds: 1800, errors: 0, queueOverflows: 0, pendingAtEnd: 0,
    heapPlateau: true, gpuPlateau: true, timingPlateau: true,
  })));
  return { version: 2, visual, soaks };
}

describe('release evidence gate', () => {
  test('requires an evidence document', async () => {
    const { root } = await fixture();
    expect(await validateReleaseEvidence(root)).toEqual([
      'release evidence is missing or malformed: docs/benchmarks/release-evidence.json',
    ]);
  });

  test('accepts complete current visual and soak evidence', async () => {
    const { root, artifactHash, screenshotHash } = await fixture();
    await writeFile(join(root, 'docs/benchmarks/release-evidence.json'),
      JSON.stringify(validEvidence(artifactHash, screenshotHash)));
    expect(await validateReleaseEvidence(root, new Date('2026-07-19T00:00:00.000Z'))).toEqual([]);
  });

  test('rejects the unaudited version-one ok boolean schema', async () => {
    const { root } = await fixture();
    await writeFile(join(root, 'docs/benchmarks/release-evidence.json'), JSON.stringify({
      version: 1, gpu: [{ ok: true }], dungeonSoaks: [],
    }));
    expect(await validateReleaseEvidence(root)).toEqual([
      'release evidence has an unsupported schema; current schema is version 2',
    ]);
  });

  test('rejects black pixels, size disagreement, and incomplete soaks', async () => {
    const { root, artifactHash, screenshotHash } = await fixture();
    const evidence = validEvidence(artifactHash, screenshotHash) as {
      visual: Array<{ dimensions: { canvasWidth: number }; pixels: { nonBackgroundFraction: number } }>;
      soaks: Array<{ durationSeconds: number; heapPlateau: boolean }>;
    };
    evidence.visual[0]!.dimensions.canvasWidth = 1;
    evidence.visual[0]!.pixels.nonBackgroundFraction = 0;
    evidence.soaks[0]!.durationSeconds = 60;
    evidence.soaks[0]!.heapPlateau = false;
    await writeFile(join(root, 'docs/benchmarks/release-evidence.json'), JSON.stringify(evidence));
    const errors = await validateReleaseEvidence(root, new Date('2026-07-19T00:00:00.000Z'));
    expect(errors.some((error) => error.includes('dimensions disagree'))).toBe(true);
    expect(errors.some((error) => error.includes('pixel evidence is invalid'))).toBe(true);
    expect(errors.some((error) => error.includes('at least 1800 seconds'))).toBe(true);
    expect(errors.some((error) => error.includes('plateaus are required'))).toBe(true);
  });
});
