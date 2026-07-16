#!/usr/bin/env bun
/** Build every browser JavaScript artifact from authored TypeScript sources. */
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { validateWebContracts, type WebArtifactManifest } from './check-web-contracts.ts';

const root = resolve(import.meta.dir, '..');
const www = join(root, 'crates/afterglow-web/www');
const check = process.argv.includes('--check');

const contractErrors = await validateWebContracts(root);
if (contractErrors.length !== 0) {
  for (const error of contractErrors) console.error(`web-contract: ${error}`);
  process.exit(1);
}
const manifest = JSON.parse(
  await readFile(join(www, 'web-artifacts.json'), 'utf8'),
) as WebArtifactManifest;
// Authored module specifiers point to TypeScript. Bun resolves and bundles
// them; emitted JavaScript exists only as a deployment artifact.
const targets = manifest.artifacts;
const generated = new Set(targets.map((artifact) => artifact.output));
const vendor = new Set([
  'three.core.js', 'three.js', 'three.module.js', 'three.webgpu.js',
  'three.webgpu.min.js',
]);

async function listFiles(directory: string, pattern: string): Promise<string[]> {
  const glob = new Bun.Glob(pattern);
  const files: string[] = [];
  for await (const file of glob.scan({ cwd: directory, onlyFiles: true })) files.push(file);
  return files;
}

for (const file of await listFiles(www, '**/*.js')) {
  if (file.startsWith('node_modules/')) continue;
  if (!generated.has(file) && !vendor.has(file)) {
    console.error(`hand-authored JavaScript is forbidden: ${relative(root, join(www, file))}`);
    process.exit(1);
  }
}

for (const file of await listFiles(www, '**/*.ts')) {
  if (file.startsWith('node_modules/')) continue;
  const source = await readFile(join(www, file), 'utf8');
  if (/(?:from\s*|import\s*\()\s*['"]\.\.?\/[^'"]+\.js['"]/.test(source)) {
    console.error(`TypeScript must not import generated local JavaScript: ${relative(root, join(www, file))}`);
    process.exit(1);
  }
}

const temporary = await mkdtemp(join(tmpdir(), 'afterglow-web-build-'));
try {
  let drift = false;
  for (const { source, output } of targets) {
    const built = join(temporary, output);
    const proc = Bun.spawn([
      process.execPath, 'build', join(www, source), '--outfile', built,
      '--target', 'browser',
    ], { stdout: 'inherit', stderr: 'inherit' });
    if (await proc.exited !== 0) process.exit(1);
    const destination = join(www, output);
    if (check) {
      let actual: Uint8Array;
      try { actual = await readFile(destination); }
      catch { actual = new Uint8Array(); }
      const expected = await readFile(built);
      if (!Buffer.from(actual).equals(expected)) {
        console.error(`generated artifact is stale: ${relative(root, destination)}`);
        drift = true;
      }
    } else {
      await Bun.write(destination, Bun.file(built));
    }
  }
  if (drift) process.exit(1);
} finally {
  await rm(temporary, { recursive: true, force: true });
}

console.log(check ? 'web TypeScript artifacts are current' : 'built web artifacts from TypeScript');
