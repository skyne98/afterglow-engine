#!/usr/bin/env bun
/** Build the disposable browser deployment tree from organized web sources. */
import { cp, mkdir, mkdtemp, readFile, rename, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import {
  countBundledThreeCoreCopies,
  validateWebContracts,
  type WebArtifactManifest,
} from './check-web-contracts.ts';

const root = resolve(import.meta.dir, '..');
const web = join(root, 'crates/afterglow-web/web');
const sourceRoot = web;
const dist = join(root, 'crates/afterglow-web/www');
const check = process.argv.includes('--check');

const contractErrors = await validateWebContracts(root);
if (contractErrors.length !== 0) {
  for (const error of contractErrors) console.error(`web-contract: ${error}`);
  process.exit(1);
}
const manifest = JSON.parse(
  await readFile(join(web, 'contracts/web-artifacts.json'), 'utf8'),
) as WebArtifactManifest;

async function listFiles(directory: string, pattern = '**/*'): Promise<string[]> {
  const glob = new Bun.Glob(pattern);
  const files: string[] = [];
  for await (const file of glob.scan({ cwd: directory, onlyFiles: true })) files.push(file);
  return files.sort();
}

for (const file of await listFiles(join(web, 'src'), '**/*.js')) {
  console.error(`hand-authored JavaScript is forbidden: ${relative(root, join(web, 'src', file))}`);
  process.exit(1);
}
for (const file of await listFiles(join(web, 'src'), '**/*.ts')) {
  const source = await readFile(join(web, 'src', file), 'utf8');
  if (/(?:from\s*|import\s*\()\s*['"]\.\.?\/[^'"]+\.js['"]/.test(source)) {
    console.error(`TypeScript must not import generated local JavaScript: ${relative(root, join(web, 'src', file))}`);
    process.exit(1);
  }
}

async function compareTrees(expected: string, actual: string): Promise<boolean> {
  const expectedFiles = await listFiles(expected);
  let actualFiles: string[] = [];
  try { actualFiles = await listFiles(actual); } catch {}
  let drift = expectedFiles.length !== actualFiles.length ||
    expectedFiles.some((file, index) => file !== actualFiles[index]);
  if (!drift) {
    for (const file of expectedFiles) {
      const left = await readFile(join(expected, file));
      const right = await readFile(join(actual, file));
      if (!Buffer.from(left).equals(right)) { drift = true; break; }
    }
  }
  return drift;
}

const temporaryRoot = await mkdtemp(join(tmpdir(), 'afterglow-web-build-'));
const staged = join(temporaryRoot, 'www');
try {
  await mkdir(staged, { recursive: true });
  await cp(join(web, 'public'), staged, { recursive: true });
  await cp(join(web, 'assets'), staged, { recursive: true });
  for (const file of await listFiles(staged)) {
    if (/\.(?:[cm]?js|ts|tsx)$/.test(file) || /(?:^|\/)(?:package(?:-lock)?\.json|bun\.lock)$/.test(file)) {
      console.error(`public/assets input may not contain source or package state: ${file}`);
      process.exit(1);
    }
  }
  for (const { source, output, role } of manifest.artifacts) {
    const built = join(staged, output);
    await mkdir(dirname(built), { recursive: true });
    const proc = Bun.spawn([
      process.execPath, 'build', join(sourceRoot, source), '--outfile', built,
      '--target', 'browser',
    ], { stdout: 'inherit', stderr: 'inherit' });
    if (await proc.exited !== 0) process.exit(1);
    const builtSource = await readFile(built, 'utf8');
    const copies = countBundledThreeCoreCopies(builtSource);
    if (copies > 1 || (role === 'visual-demo' && copies !== 1)) {
      console.error(`${source}: ${role === 'visual-demo' ? 'visual bundle requires exactly one' : 'bundle contains'} ${copies} Three.js core identities`);
      process.exit(1);
    }
  }

  if (check) {
    if (await compareTrees(staged, dist)) {
      console.error(`generated web deployment is stale: ${relative(root, dist)}`);
      process.exit(1);
    }
  } else {
    await rm(dist, { recursive: true, force: true });
    await rename(staged, dist);
  }
} finally {
  await rm(temporaryRoot, { recursive: true, force: true });
}

console.log(check ? 'web deployment is current' : 'built disposable web deployment');
