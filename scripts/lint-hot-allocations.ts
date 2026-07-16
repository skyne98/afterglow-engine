#!/usr/bin/env bun
/** Conservative allocation lint for explicitly sealed engine hot regions. */
import { readFile } from 'node:fs/promises';
import { join, relative, resolve } from 'node:path';
import ts from '../crates/afterglow-web/www/node_modules/typescript/lib/typescript.js';

const root = resolve(import.meta.dir, '..');
const www = join(root, 'crates/afterglow-web/www');
const engine = join(www, 'engine');
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
const effectBanned: Array<[RegExp, string]> = [
  ...banned,
  [/\b(?:requestAnimationFrame|queueMicrotask|setTimeout|setInterval)\s*\(/, 'scheduling/browser callback allocation'],
  [/\bconsole\.(?:log|info|warn|error|debug)\s*\(/, 'dynamic console diagnostic'],
  [/\.(?:innerHTML|textContent)\s*=/, 'dynamic DOM diagnostic'],
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
// JSDoc effects close the marker-omission loophole for registered frame clients.
// Every authored function declaring @alloc-effect none is scanned as a whole,
// including demo entrypoints outside engine/.
let effectFunctions = 0;
const configPath = join(www, 'tsconfig.harsh.json');
const loadedConfig = ts.readConfigFile(configPath, ts.sys.readFile);
if (loadedConfig.error) throw new Error(ts.flattenDiagnosticMessageText(loadedConfig.error.messageText, '\n'));
const parsedConfig = ts.parseJsonConfigFileContent(loadedConfig.config, ts.sys, www, undefined, configPath);
const effectProgram = ts.createProgram(parsedConfig.fileNames, parsedConfig.options);
const effectChecker = effectProgram.getTypeChecker();

function declaredEffect(call: ts.CallExpression): string | null {
  const target = ts.isPropertyAccessExpression(call.expression) ? call.expression.name : call.expression;
  let symbol = effectChecker.getSymbolAtLocation(target);
  if (!symbol) return null;
  if ((symbol.flags & ts.SymbolFlags.Alias) !== 0) symbol = effectChecker.getAliasedSymbol(symbol);
  const declaration = symbol.valueDeclaration ?? symbol.declarations?.[0];
  if (!declaration) return null;
  const declarationFile = declaration.getSourceFile().fileName;
  if (!declarationFile.startsWith(www) || declarationFile.includes('/node_modules/')) return null;
  const file = relative(engine, declarationFile).replaceAll('\\', '/');
  let qualified = symbol.getName();
  if ((ts.isMethodDeclaration(declaration) || ts.isMethodSignature(declaration)) &&
      declaration.parent && ts.isClassDeclaration(declaration.parent) && declaration.parent.name)
    qualified = `${declaration.parent.name.text}.${symbol.getName()}`;
  if (effects.none[qualified]) return 'none';
  if (effects.budgetedBoundaries[qualified]) return 'budgeted';
  if (effects.bootstrapOnly[qualified]) return 'bootstrap';
  const leading = declaration.getSourceFile().text.slice(declaration.getFullStart(), declaration.getStart());
  const explicit = leading.match(/@alloc-effect\s+(none|pooled|budgeted|bootstrap|gameFacing|diagnostic)\b/)?.[1];
  if (explicit) return explicit;
  return effects.moduleEffects[file] ?? 'unknown';
}

const authoredGlob = new Bun.Glob('**/*.ts');
for await (const path of authoredGlob.scan({ cwd: www, onlyFiles: true })) {
  if (path.startsWith('node_modules/') || path.endsWith('.test.ts') || path.endsWith('.d.ts')) continue;
  const absolutePath = join(www, path);
  const sourceText = await readFile(absolutePath, 'utf8');
  const source = effectProgram.getSourceFile(absolutePath) ??
    ts.createSourceFile(path, sourceText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const visit = (node: ts.Node): void => {
    if (ts.isFunctionDeclaration(node) || ts.isMethodDeclaration(node) ||
        ts.isFunctionExpression(node) || ts.isArrowFunction(node)) {
      const leading = sourceText.slice(node.getFullStart(), node.getStart(source));
      if (/@alloc-effect\s+none\b/.test(leading)) {
        effectFunctions++;
        const start = source.getLineAndCharacterOfPosition(node.getStart(source)).line;
        const lines = node.getText(source).split('\n');
        for (let index = 0; index < lines.length; index++) {
          const line = lines[index];
          const permit = line.match(/@alloc-allowed\s+reason=\S+\s+issue=DME-\d+\s+expires=(\d{4}-\d{2}-\d{2})/);
          if (permit?.[1] !== undefined && permit[1] >= new Date().toISOString().slice(0, 10)) continue;
          for (const [pattern, description] of effectBanned) if (pattern.test(line)) {
            console.error(`${relative(root, join(www, path))}:${start + index + 1}: @alloc-effect none: ${description}`);
            failures++;
          }
        }
        const inspectCalls = (child: ts.Node): void => {
          if (ts.isCallExpression(child)) {
            const effect = declaredEffect(child);
            if (effect !== null && effect !== 'none' && effect !== 'pooled') {
              const position = source.getLineAndCharacterOfPosition(child.getStart(source));
              const line = sourceText.split(/\r?\n/)[position.line] ?? '';
              const permit = line.match(/@alloc-allowed\s+reason=\S+\s+issue=DME-\d+\s+expires=(\d{4}-\d{2}-\d{2})/);
              const permitted = permit?.[1] !== undefined && permit[1] >= new Date().toISOString().slice(0, 10);
              if (!permitted) {
                console.error(`${relative(root, absolutePath)}:${position.line + 1}: @alloc-effect none calls authored ${effect} function ${child.expression.getText(source)}`);
                failures++;
              }
            }
          }
          ts.forEachChild(child, inspectCalls);
        };
        if (node.body) inspectCalls(node.body);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
}
if (effectFunctions === 0) {
  console.error('no @alloc-effect none functions found');
  failures++;
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
console.log(`allocation lint passed for ${regions} sealed hot regions and ${effectFunctions} effect-declared functions`);
