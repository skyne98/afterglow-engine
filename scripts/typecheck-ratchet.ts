#!/usr/bin/env bun
/** Strict TypeScript checker with a merge-base-monotonic legacy error baseline. */
import ts from '../crates/afterglow-web/www/node_modules/typescript/lib/typescript.js';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { join, relative, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

interface TypeFinding {
  id: string;
  file: string;
  line: number;
  column: number;
  code: number;
  message: string;
  source: string;
}
interface TypeBaseline { version: number; config: string; findings: TypeFinding[] }
const root = resolve(import.meta.dir, '..');
const www = join(root, 'crates/afterglow-web/www');
const configName = 'tsconfig.harsh.json';
const configPath = join(www, configName);
const baselinePath = join(www, 'typescript-error-baseline.json');

function compact(text: string): string { return text.replace(/\s+/g, ' ').trim(); }
function parseCompareRef(): string | null {
  const index = process.argv.indexOf('--compare-ref');
  return index < 0 ? null : process.argv[index + 1] ?? null;
}
function baselineAtRef(ref: string): TypeBaseline | null {
  const path = 'crates/afterglow-web/www/typescript-error-baseline.json';
  const result = spawnSync('git', ['show', `${ref}:${path}`], { cwd: root, encoding: 'utf8' });
  if (result.status !== 0) return null;
  try { return JSON.parse(result.stdout) as TypeBaseline; } catch { return null; }
}

export function collectTypeFindings(): TypeFinding[] {
  const loaded = ts.readConfigFile(configPath, ts.sys.readFile);
  if (loaded.error) throw new Error(ts.flattenDiagnosticMessageText(loaded.error.messageText, '\n'));
  const parsed = ts.parseJsonConfigFileContent(loaded.config, ts.sys, www, undefined, configPath);
  if (parsed.errors.length !== 0)
    throw new Error(parsed.errors.map((error) => ts.flattenDiagnosticMessageText(error.messageText, '\n')).join('\n'));
  const program = ts.createProgram(parsed.fileNames, parsed.options);
  const diagnostics = ts.getPreEmitDiagnostics(program)
    .filter((diagnostic) => !diagnostic.file?.fileName.includes('/node_modules/'));
  const raw = diagnostics.map((diagnostic) => {
    const file = diagnostic.file ? relative(www, diagnostic.file.fileName).replaceAll('\\', '/') : '<global>';
    const position = diagnostic.file && diagnostic.start !== undefined
      ? diagnostic.file.getLineAndCharacterOfPosition(diagnostic.start)
      : { line: 0, character: 0 };
    const source = diagnostic.file ? compact(diagnostic.file.text.split(/\r?\n/)[position.line] ?? '') : '';
    const message = compact(ts.flattenDiagnosticMessageText(diagnostic.messageText, ' '));
    return { file, line: position.line + 1, column: position.character + 1, code: diagnostic.code, message, source };
  }).sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line || a.column - b.column || a.code - b.code || a.message.localeCompare(b.message));
  const occurrences = new Map<string, number>();
  return raw.map((finding) => {
    const identity = `${finding.file}\0${finding.code}\0${finding.message}\0${finding.source}`;
    const occurrence = occurrences.get(identity) ?? 0;
    occurrences.set(identity, occurrence + 1);
    const id = createHash('sha256').update(`${identity}\0${occurrence}`).digest('hex').slice(0, 20);
    return { id, ...finding };
  });
}

if (import.meta.main) {
  const findings = collectTypeFindings();
  if (process.argv.includes('--json')) {
    console.log(JSON.stringify({ version: 1, config: configName, findings }, null, 2));
    process.exit(0);
  }
  if (process.argv.includes('--write-baseline')) {
    const baseline: TypeBaseline = { version: 1, config: configName, findings };
    await writeFile(baselinePath, `${JSON.stringify(baseline, null, 2)}\n`);
    console.log(`wrote ${findings.length} frozen strict TypeScript error(s) to ${relative(root, baselinePath)}`);
    process.exit(0);
  }

  let baseline: TypeBaseline;
  try { baseline = JSON.parse(await readFile(baselinePath, 'utf8')) as TypeBaseline; }
  catch { console.error('strict TypeScript baseline is missing; bootstrap it explicitly with --write-baseline'); process.exit(1); }
  if (baseline.version !== 1 || baseline.config !== configName || !Array.isArray(baseline.findings)) {
    console.error('strict TypeScript baseline has an unsupported or malformed schema');
    process.exit(1);
  }
  const accepted = new Set(baseline.findings.map((finding) => finding.id));
  const current = new Set(findings.map((finding) => finding.id));
  let failures = 0;
  for (const finding of findings) if (!accepted.has(finding.id)) {
    console.error(`${finding.file}:${finding.line}:${finding.column}: TS${finding.code}: NEW strict error: ${finding.message}`);
    failures++;
  }
  for (const finding of baseline.findings) if (!current.has(finding.id)) {
    console.error(`${finding.file}:${finding.line}:${finding.column}: TS${finding.code}: stale strict-error baseline entry; remove it`);
    failures++;
  }

  const compareRef = parseCompareRef();
  if (compareRef) {
    const old = baselineAtRef(compareRef);
    if (old) {
      const oldIds = new Set(old.findings.map((finding) => finding.id));
      for (const finding of baseline.findings) if (!oldIds.has(finding.id)) {
        console.error(`${finding.file}:${finding.line}:${finding.column}: strict-error baseline addition forbidden relative to ${compareRef}: TS${finding.code}`);
        failures++;
      }
    } else {
      console.warn(`strict TypeScript ratchet bootstrap: ${compareRef} has no readable baseline`);
    }
  }

  if (failures !== 0) {
    console.error(`strict TypeScript ratchet failed with ${failures} error(s)`);
    process.exit(1);
  }
  console.log(`strict TypeScript ratchet passed (${findings.length} frozen error(s), zero new)`);
}
