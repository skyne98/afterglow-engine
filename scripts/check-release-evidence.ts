#!/usr/bin/env bun
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import type { WebArtifactManifest } from './check-web-contracts.ts';

interface GpuEvidence {
  demo: string;
  artifact: string;
  sha256: string;
  capturedAt: string;
  adapter: string;
  driver: string;
  ok: boolean;
}
interface SoakEvidence {
  mode: 'stable' | 'traverse' | 'thrash';
  artifact: string;
  sha256: string;
  capturedAt: string;
  durationSeconds: number;
  errors: number;
  queueOverflows: number;
  pendingAtEnd: number;
}
interface ReleaseEvidence { version: number; gpu: GpuEvidence[]; dungeonSoaks: SoakEvidence[] }

export async function validateReleaseEvidence(root: string, now = new Date()): Promise<string[]> {
  const errors: string[] = [];
  const www = join(root, 'crates/afterglow-web/www');
  const contracts = join(root, 'crates/afterglow-web/web/contracts');
  const evidencePath = join(root, 'docs/benchmarks/release-evidence.json');
  let evidence: ReleaseEvidence;
  let manifest: WebArtifactManifest;
  try { evidence = JSON.parse(await readFile(evidencePath, 'utf8')) as ReleaseEvidence; }
  catch { return ['release evidence is missing or malformed: docs/benchmarks/release-evidence.json']; }
  try { manifest = JSON.parse(await readFile(join(contracts, 'web-artifacts.json'), 'utf8')) as WebArtifactManifest; }
  catch { return ['web artifact manifest is unavailable']; }
  if (evidence.version !== 1 || !Array.isArray(evidence.gpu) || !Array.isArray(evidence.dungeonSoaks))
    return ['release evidence has an unsupported schema'];
  const maximumAgeMs = 30 * 24 * 60 * 60 * 1000;
  const hashes = new Map<string, string>();
  async function artifactHash(path: string): Promise<string> {
    const cached = hashes.get(path); if (cached) return cached;
    const hash = new Bun.CryptoHasher('sha256').update(await readFile(join(www, path))).digest('hex');
    hashes.set(path, hash); return hash;
  }
  function current(date: string, label: string): void {
    const stamp = Date.parse(date);
    if (!Number.isFinite(stamp) || stamp > now.getTime() + 60_000 || now.getTime() - stamp > maximumAgeMs)
      errors.push(`${label}: evidence must be a valid timestamp from the last 30 days`);
  }
  for (const demo of manifest.artifacts.filter((entry) => entry.role === 'visual-demo')) {
    const record = evidence.gpu.find((entry) => entry.demo === demo.source);
    if (!record) { errors.push(`${demo.source}: real-GPU evidence is missing`); continue; }
    if (!record.ok || !record.adapter || !record.driver) errors.push(`${demo.source}: GPU result or hardware identity is invalid`);
    if (record.artifact !== demo.output) errors.push(`${demo.source}: evidence artifact is ${record.artifact}, expected ${demo.output}`);
    else if (record.sha256 !== await artifactHash(demo.output)) errors.push(`${demo.source}: GPU evidence artifact hash is stale`);
    current(record.capturedAt, demo.source);
  }
  const dungeon = manifest.artifacts.find((entry) => entry.source === 'dungeon.ts');
  if (!dungeon) errors.push('dungeon.ts visual artifact is missing');
  else for (const mode of ['stable', 'traverse', 'thrash'] as const) {
    const record = evidence.dungeonSoaks.find((entry) => entry.mode === mode);
    if (!record) { errors.push(`Dungeon ${mode}: soak evidence is missing`); continue; }
    if (record.durationSeconds < 600) errors.push(`Dungeon ${mode}: soak must run at least 600 seconds`);
    if (record.errors !== 0 || record.queueOverflows !== 0 || record.pendingAtEnd !== 0)
      errors.push(`Dungeon ${mode}: soak ended with errors, overflow, or pending work`);
    if (record.artifact !== dungeon.output || record.sha256 !== await artifactHash(dungeon.output))
      errors.push(`Dungeon ${mode}: soak artifact hash is stale`);
    current(record.capturedAt, `Dungeon ${mode}`);
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
  console.log('current real-GPU and soak release evidence passed');
}
