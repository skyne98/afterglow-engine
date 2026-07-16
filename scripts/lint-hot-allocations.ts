#!/usr/bin/env bun
/** Conservative allocation lint for explicitly sealed engine hot regions. */
import { readFile } from 'node:fs/promises';
import { join, relative, resolve } from 'node:path';

const root = resolve(import.meta.dir, '..');
const engine = join(root, 'crates/afterglow-web/www/engine');
const effectsPath = join(root, 'crates/afterglow-web/www/engine-allocation-effects.json');
const effects = JSON.parse(await readFile(effectsPath, 'utf8')) as {
  version: number;
  moduleEffects: Record<string, string>;
  none: Record<string, string>;
  budgetedBoundaries: Record<string, string>;
  bootstrapOnly: Record<string, string>;
};
if (effects.version !== 1) throw new Error(`unsupported allocation-effect manifest version ${effects.version}`);
const begin = /\/\/\s*@hot-no-alloc-begin\s+(\S+)/;
const end = /\/\/\s*@hot-no-alloc-end\s+(\S+)/;
const banned: Array<[RegExp, string]> = [
  [/\bnew\s+(?!Error\b|RangeError\b)/, '`new` allocation'],
  [/\b(?:async|await)\b/, 'async/microtask allocation'],
  [/\bPromise\b|\.(?:then|catch|finally)\s*\(/, 'promise allocation'],
  [/=>/, 'closure allocation'],
  [/`[^`]*\$\{/, 'dynamic template string'],
  [/\.(?:map|filter|reduce|flatMap|slice|splice|concat)\s*\(/, 'allocating array operation'],
  [/\bArray\.from\s*\(|\bObject\.(?:keys|values|entries|assign)\s*\(/, 'materializing collection operation'],
  [/\.subarray\s*\(/, 'new typed-array view'],
  [/\.{3}[A-Za-z_$\[{]/, 'spread/rest allocation'],
  [/\breturn\s*\{/, 'object literal return'],
];

const glob = new Bun.Glob('**/*.ts');
let failures = 0;
let regions = 0;
const names = new Set<string>();
const sourceFiles = new Set<string>();
const regionFiles = new Map<string, string>();
const boundaryCalls = Object.keys(effects.budgetedBoundaries).map(name => ({
  name,
  method: name.slice(name.lastIndexOf('.') + 1),
}));
for await (const path of glob.scan({ cwd: engine, onlyFiles: true })) {
  if (!path.endsWith('.test.ts')) sourceFiles.add(path);
  const lines = (await readFile(join(engine, path), 'utf8')).split('\n');
  let active: { name: string; line: number } | null = null;
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index];
    const open = line.match(begin);
    const close = line.match(end);
    if (open) {
      if (active) {
        console.error(`${relative(root, join(engine, path))}:${index + 1}: nested hot region`);
        failures++;
      }
      active = { name: open[1], line: index + 1 };
      if (names.has(open[1])) {
        console.error(`${relative(root, join(engine, path))}:${index + 1}: duplicate hot region ${open[1]}`);
        failures++;
      }
      names.add(open[1]);
      regionFiles.set(open[1], path);
      regions++; 
      continue;
    }
    if (close) {
      if (!active || close[1] !== active.name) {
        console.error(`${relative(root, join(engine, path))}:${index + 1}: mismatched hot-region end ${close[1]}`);
        failures++;
      }
      active = null;
      continue;
    }
    if (!active || line.includes('@alloc-allowed')) continue;
    for (const boundary of boundaryCalls) {
      if (new RegExp(`(?:\\.|\\b)${boundary.method}\\s*\\(`).test(line)) {
        console.error(`${relative(root, join(engine, path))}:${index + 1}: ${active.name}: calls budgeted boundary ${boundary.name} without @alloc-allowed reason`);
        failures++;
      }
    }
    for (const [pattern, description] of banned) {
      if (pattern.test(line)) {
        console.error(`${relative(root, join(engine, path))}:${index + 1}: ${active.name}: ${description}`);
        failures++;
      }
    }
  }
  if (active) {
    console.error(`${relative(root, join(engine, path))}:${active.line}: unclosed hot region ${active.name}`);
    failures++;
  }
}
if (regions === 0) {
  console.error('no @hot-no-alloc regions found');
  failures++;
}
for (const [name, path] of regionFiles) {
  const declaredFile = effects.none[name];
  if (!declaredFile) {
    console.error(`${path}: hot region ${name} has no @alloc-effect none manifest entry`);
    failures++;
  } else if (declaredFile !== path) {
    console.error(`${path}: hot region ${name} is declared in ${declaredFile}`);
    failures++;
  }
}
for (const [name, path] of Object.entries(effects.none)) {
  if (!names.has(name)) {
    console.error(`${path}: stale @alloc-effect none manifest entry ${name}`);
    failures++;
  }
}
const validEffects = new Set(['none', 'pooled', 'budgeted', 'bootstrap', 'gameFacing', 'diagnostic']);
for (const path of sourceFiles) {
  const effect = effects.moduleEffects[path];
  if (!effect) {
    console.error(`${path}: engine module has no allocation-effect classification`);
    failures++;
  } else if (!validEffects.has(effect)) {
    console.error(`${path}: unknown allocation effect ${effect}`);
    failures++;
  }
}
for (const path of Object.keys(effects.moduleEffects)) {
  if (!sourceFiles.has(path)) {
    console.error(`${path}: stale allocation-effect module entry`);
    failures++;
  }
}
if (failures) process.exit(1);
console.log(`allocation lint passed for ${regions} sealed hot regions`);
