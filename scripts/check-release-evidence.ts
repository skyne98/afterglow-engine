#!/usr/bin/env bun
import { access, readFile } from 'node:fs/promises';
import { join, normalize, resolve } from 'node:path';
import type { WebArtifactManifest } from './check-web-contracts.ts';

export type ReleaseTarget = 'web' | 'native';
interface Dimensions {
  logicalWidth: number;
  logicalHeight: number;
  physicalWidth: number;
  physicalHeight: number;
  surfaceWidth: number;
  surfaceHeight: number;
  canvasWidth: number;
  canvasHeight: number;
  feedbackWidth: number;
  feedbackHeight: number;
  devicePixelRatio: number;
}
interface VisualEvidence {
  demo: string;
  target: ReleaseTarget;
  artifact: string;
  sha256: string;
  capturedAt: string;
  commit: string;
  host: string;
  adapter: string;
  driver: string;
  dimensions: Dimensions;
  readiness: {
    gameReady: boolean;
    fatalDiagnostics: number;
    pendingBootstrap: number;
    stages: string[];
  };
  pixels: {
    screenshot: string;
    screenshotSha256: string;
    nonBackgroundFraction: number;
    luminanceStdDev: number;
    referenceDiff: number;
    maxReferenceDiff: number;
  };
  frames: { samples: number; p99Ms: number; maxMs: number };
  resources: { heapBytes: number; gpuBytes: number };
  queues: { overflows: number; pendingAtEnd: number };
}
interface SoakEvidence {
  scenario: string;
  target: ReleaseTarget;
  artifact: string;
  sha256: string;
  capturedAt: string;
  durationSeconds: number;
  errors: number;
  queueOverflows: number;
  pendingAtEnd: number;
  heapPlateau: boolean;
  gpuPlateau: boolean;
  timingPlateau: boolean;
}
interface ReleaseEvidence { version: number; visual: VisualEvidence[]; soaks: SoakEvidence[] }

const targets: readonly ReleaseTarget[] = ['web', 'native'];
const soakScenarios = [
  'dungeon-streaming',
  'mutable-painting',
  'rigged-model',
  'combined',
  'capacity-thrash',
] as const;

function safeRelative(path: unknown): path is string {
  if (typeof path !== 'string' || path.length === 0 || path.startsWith('/') || path.includes('\\')) return false;
  const clean = normalize(path).replaceAll('\\', '/');
  return clean === path && clean !== '..' && !clean.startsWith('../');
}

async function exists(path: string): Promise<boolean> {
  try { await access(path); return true; } catch { return false; }
}

export async function validateReleaseEvidence(root: string, now = new Date()): Promise<string[]> {
  const errors: string[] = [];
  const www = join(root, 'crates/afterglow-web/www');
  const contracts = join(root, 'crates/afterglow-web/web/contracts');
  const benchmarks = join(root, 'docs/benchmarks');
  const evidencePath = join(benchmarks, 'release-evidence.json');
  let evidence: ReleaseEvidence;
  let manifest: WebArtifactManifest;
  try { evidence = JSON.parse(await readFile(evidencePath, 'utf8')) as ReleaseEvidence; }
  catch { return ['release evidence is missing or malformed: docs/benchmarks/release-evidence.json']; }
  try { manifest = JSON.parse(await readFile(join(contracts, 'web-artifacts.json'), 'utf8')) as WebArtifactManifest; }
  catch { return ['web artifact manifest is unavailable']; }
  if (evidence.version !== 2 || !Array.isArray(evidence.visual) || !Array.isArray(evidence.soaks))
    return ['release evidence has an unsupported schema; current schema is version 2'];

  const maximumAgeMs = 30 * 24 * 60 * 60 * 1000;
  const hashes = new Map<string, string>();
  async function fileHash(base: string, path: string): Promise<string> {
    const key = `${base}\0${path}`;
    const cached = hashes.get(key); if (cached) return cached;
    const hash = new Bun.CryptoHasher('sha256').update(await readFile(join(base, path))).digest('hex');
    hashes.set(key, hash); return hash;
  }
  async function artifactHash(path: string): Promise<string> { return fileHash(www, path); }
  function current(date: string, label: string): void {
    const stamp = Date.parse(date);
    if (!Number.isFinite(stamp) || stamp > now.getTime() + 60_000 || now.getTime() - stamp > maximumAgeMs)
      errors.push(`${label}: evidence must be a valid timestamp from the last 30 days`);
  }
  function positiveInteger(value: number): boolean { return Number.isInteger(value) && value > 0; }
  function nonNegativeInteger(value: number): boolean { return Number.isInteger(value) && value >= 0; }

  const visualDemos = manifest.artifacts.filter((entry) => entry.role === 'visual-demo');
  const visualKeys = new Set<string>();
  for (const record of evidence.visual) {
    const key = `${record.demo}\0${record.target}`;
    if (visualKeys.has(key)) errors.push(`${record.demo}/${record.target}: duplicate visual evidence`);
    visualKeys.add(key);
  }
  for (const demo of visualDemos) for (const target of targets) {
    const label = `${demo.source}/${target}`;
    const record = evidence.visual.find((entry) => entry.demo === demo.source && entry.target === target);
    if (!record) { errors.push(`${label}: visual evidence is missing`); continue; }
    if (!record.host || !record.adapter || !record.driver || !/^[0-9a-f]{7,40}$/i.test(record.commit))
      errors.push(`${label}: host, hardware identity, or commit is invalid`);
    if (record.artifact !== demo.output) errors.push(`${label}: evidence artifact is ${record.artifact}, expected ${demo.output}`);
    else if (record.sha256 !== await artifactHash(demo.output)) errors.push(`${label}: visual artifact hash is stale`);
    current(record.capturedAt, label);

    const size = record.dimensions;
    if (!size || !positiveInteger(size.logicalWidth) || !positiveInteger(size.logicalHeight) ||
        !positiveInteger(size.physicalWidth) || !positiveInteger(size.physicalHeight) ||
        !positiveInteger(size.surfaceWidth) || !positiveInteger(size.surfaceHeight) ||
        !positiveInteger(size.canvasWidth) || !positiveInteger(size.canvasHeight) ||
        !nonNegativeInteger(size.feedbackWidth) || !nonNegativeInteger(size.feedbackHeight) ||
        !Number.isFinite(size.devicePixelRatio) || size.devicePixelRatio <= 0)
      errors.push(`${label}: dimensions are invalid`);
    else {
      if (size.physicalWidth !== size.surfaceWidth || size.physicalHeight !== size.surfaceHeight ||
          size.physicalWidth !== size.canvasWidth || size.physicalHeight !== size.canvasHeight)
        errors.push(`${label}: physical window, surface, and canvas dimensions disagree`);
      if (Math.abs(size.logicalWidth * size.devicePixelRatio - size.physicalWidth) > 1 ||
          Math.abs(size.logicalHeight * size.devicePixelRatio - size.physicalHeight) > 1)
        errors.push(`${label}: logical size and device pixel ratio disagree with physical size`);
    }

    if (!record.readiness?.gameReady || record.readiness.fatalDiagnostics !== 0 ||
        record.readiness.pendingBootstrap !== 0 || !Array.isArray(record.readiness.stages) ||
        !record.readiness.stages.includes('GameReady'))
      errors.push(`${label}: GameReady/readiness evidence is invalid`);
    const pixels = record.pixels;
    if (!pixels || !safeRelative(pixels.screenshot) || !/^[0-9a-f]{64}$/i.test(pixels.screenshotSha256) ||
        !Number.isFinite(pixels.nonBackgroundFraction) || pixels.nonBackgroundFraction < 0.01 ||
        !Number.isFinite(pixels.luminanceStdDev) || pixels.luminanceStdDev <= 0.01 ||
        !Number.isFinite(pixels.referenceDiff) || !Number.isFinite(pixels.maxReferenceDiff) ||
        pixels.maxReferenceDiff <= 0 || pixels.maxReferenceDiff > 0.2 ||
        pixels.referenceDiff < 0 || pixels.referenceDiff > pixels.maxReferenceDiff) {
      errors.push(`${label}: semantic/reference pixel evidence is invalid`);
    } else if (!(await exists(join(benchmarks, pixels.screenshot)))) {
      errors.push(`${label}: screenshot is missing: ${pixels.screenshot}`);
    } else if (pixels.screenshotSha256 !== await fileHash(benchmarks, pixels.screenshot)) {
      errors.push(`${label}: screenshot hash is stale`);
    }
    if (!record.frames || !positiveInteger(record.frames.samples) || record.frames.samples < 120 ||
        !Number.isFinite(record.frames.p99Ms) || record.frames.p99Ms <= 0 ||
        !Number.isFinite(record.frames.maxMs) || record.frames.maxMs < record.frames.p99Ms)
      errors.push(`${label}: frame evidence is invalid`);
    if (!record.resources || !nonNegativeInteger(record.resources.heapBytes) || !nonNegativeInteger(record.resources.gpuBytes))
      errors.push(`${label}: resource totals are invalid`);
    if (!record.queues || record.queues.overflows !== 0 || record.queues.pendingAtEnd !== 0)
      errors.push(`${label}: queues overflowed or retained pending work`);
  }
  for (const record of evidence.visual) {
    if (!visualDemos.some((demo) => demo.source === record.demo) || !targets.includes(record.target))
      errors.push(`${record.demo}/${record.target}: stale or invalid visual evidence entry`);
  }

  const artifactByOutput = new Map(manifest.artifacts.map((entry) => [entry.output, entry]));
  const soakKeys = new Set<string>();
  for (const record of evidence.soaks) {
    const key = `${record.scenario}\0${record.target}`;
    if (soakKeys.has(key)) errors.push(`${record.scenario}/${record.target}: duplicate soak evidence`);
    soakKeys.add(key);
  }
  for (const scenario of soakScenarios) for (const target of targets) {
    const label = `${scenario}/${target}`;
    const record = evidence.soaks.find((entry) => entry.scenario === scenario && entry.target === target);
    if (!record) { errors.push(`${label}: soak evidence is missing`); continue; }
    const artifact = artifactByOutput.get(record.artifact);
    if (!artifact || artifact.role !== 'visual-demo') errors.push(`${label}: soak artifact is not a visual demo`);
    else if (record.sha256 !== await artifactHash(record.artifact)) errors.push(`${label}: soak artifact hash is stale`);
    if (record.durationSeconds < 1_800) errors.push(`${label}: soak must run at least 1800 seconds`);
    if (record.errors !== 0 || record.queueOverflows !== 0 || record.pendingAtEnd !== 0)
      errors.push(`${label}: soak ended with errors, overflow, or pending work`);
    if (!record.heapPlateau || !record.gpuPlateau || !record.timingPlateau)
      errors.push(`${label}: heap, GPU, and timing plateaus are required`);
    current(record.capturedAt, label);
  }
  for (const record of evidence.soaks) {
    if (!soakScenarios.includes(record.scenario as typeof soakScenarios[number]) || !targets.includes(record.target))
      errors.push(`${record.scenario}/${record.target}: stale or invalid soak evidence entry`);
  }
  return errors;
}

if (import.meta.main) {
  const root = resolve(import.meta.dir, '..');
  const errors = await validateReleaseEvidence(root);
  if (errors.length) {
    for (const error of errors) console.error(`release-gate: ${error}`);
    process.exit(1);
  }
  console.log('current visual and soak release evidence v2 passed');
}
