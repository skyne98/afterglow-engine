#!/usr/bin/env bun
import { readFile } from 'node:fs/promises';
import { dirname, join, normalize, relative, resolve } from 'node:path';
import type { WebArtifactManifest } from './check-web-contracts.ts';

const root = resolve(import.meta.dir, '..');
const www = join(root, 'crates/afterglow-web/www');
const manifest = JSON.parse(await readFile(join(www, 'web-artifacts.json'), 'utf8')) as WebArtifactManifest;
const visual = new Set(manifest.artifacts.filter((entry) => entry.role === 'visual-demo').map((entry) => entry.source));
export function importBoundaryErrors(file: string, source: string, wwwRoot: string, visualSources: ReadonlySet<string>): string[] {
  const found: string[] = [], engineRoot = join(wwwRoot, 'engine');
  for (const match of source.matchAll(/(?:from\s*|import\s*(?:\(\s*)?)['"]([^'"]+)['"]/g)) {
    const specifier = match[1];
    if (!specifier || !specifier.startsWith('.')) continue;
    const target = normalize(resolve(dirname(join(wwwRoot, file)), specifier));
    if (file.startsWith('engine/') && !file.endsWith('.test.ts') && relative(engineRoot, target).startsWith('..'))
      found.push(`${file}: engine production source may not import outside engine: ${specifier}`);
    if (visualSources.has(file) && (specifier.includes('/support/') || target.includes('/tests/') || specifier.includes('.test.')))
      found.push(`${file}: visual production source may not import test/support code: ${specifier}`);
    if (!file.endsWith('.test.ts') && (specifier.includes('.test.') || target.includes('/tests/')))
      found.push(`${file}: production source may not import tests: ${specifier}`);
  }
  return found;
}

if (import.meta.main) {
  const errors: string[] = [], glob = new Bun.Glob('**/*.ts');
  for await (const file of glob.scan({ cwd: www, onlyFiles: true })) {
    if (file.startsWith('node_modules/')) continue;
    errors.push(...importBoundaryErrors(file, await readFile(join(www, file), 'utf8'), www, visual));
  }
  if (errors.length) {
    for (const error of errors) console.error(`import-boundary: ${error}`);
    process.exit(1);
  }
  console.log('engine/demo/test import boundaries passed');
}
