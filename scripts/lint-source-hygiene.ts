#!/usr/bin/env bun
/** Zero-growth ratchet for TypeScript escape hatches and deferred-code markers. */
import ts from '../crates/afterglow-web/web/node_modules/typescript/lib/typescript.js';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { join, relative, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

interface Finding { id: string; rule: string; file: string; line: number; column: number; excerpt: string }
interface Baseline { version: number; findings: Finding[] }
const baselineVersion = 3;
const root = resolve(import.meta.dir, '..');
const sourceRoot = join(root, 'crates/afterglow-web/web/src');
const baselinePath = join(root, 'crates/afterglow-web/web/contracts/source-hygiene-baseline.json');
const descriptions: Record<string, string> = {
  'AG-TS-001': 'explicit any defeats static type checking',
  'AG-TS-002': 'TypeScript/linter suppression directive',
  'AG-TS-003': 'dynamic code evaluation',
  'AG-TS-004': 'empty catch silently swallows failure',
  'AG-TS-005': 'deferred TODO/FIXME/HACK marker',
  'AG-TS-006': 'non-null/definite-assignment assertion bypasses checking',
  'AG-TS-007': 'allocation permit lacks an issue and expiry',
  'AG-TS-008': 'architecture suppression is forbidden outside the temporary ratchet',
  'AG-TS-009': 'double type assertion bypasses structural type checking',
  'AG-TS-010': 'unsafe-cast permit is malformed or expired',
};
function compact(text: string): string { return text.replace(/\s+/g, ' ').trim().slice(0, 180); }
function compareRef(): string | null {
  const index = process.argv.indexOf('--compare-ref');
  return index < 0 ? null : process.argv[index + 1] ?? null;
}
function atRef(ref: string): Baseline | null {
  const path = 'crates/afterglow-web/web/contracts/source-hygiene-baseline.json';
  const result = spawnSync('git', ['show', `${ref}:${path}`], { cwd: root, encoding: 'utf8' });
  if (result.status !== 0) return null;
  try {
    const baseline = JSON.parse(result.stdout) as Baseline;
    return baseline.version === baselineVersion ? baseline : null;
  } catch { return null; }
}

export async function scanSourceHygiene(): Promise<Finding[]> {
  const raw: Array<Omit<Finding, 'id'> & { identity: string }> = [];
  const glob = new Bun.Glob('**/*.ts');
  for await (const file of glob.scan({ cwd: sourceRoot, onlyFiles: true })) {
    if (file.startsWith('node_modules/')) continue;
    const text = await readFile(join(sourceRoot, file), 'utf8');
    const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
    const add = (rule: string, node: ts.Node, identity?: string): void => {
      const position = source.getLineAndCharacterOfPosition(node.getStart(source));
      const excerpt = compact(node.getText(source));
      raw.push({ rule, file, line: position.line + 1, column: position.character + 1, excerpt,
        identity: `${file}\0${rule}\0${identity ?? excerpt}` });
    };
    const visit = (node: ts.Node): void => {
      if (node.kind === ts.SyntaxKind.AnyKeyword) add('AG-TS-001', node, 'explicit-any');
      if (ts.isNonNullExpression(node) ||
          ((ts.isPropertyDeclaration(node) || ts.isPropertySignature(node)) && node.exclamationToken))
        add('AG-TS-006', node, compact(node.getText(source)));
      if (ts.isAsExpression(node) && ts.isAsExpression(node.expression)) {
        const position = source.getLineAndCharacterOfPosition(node.getStart(source));
        const line = text.split(/\r?\n/)[position.line] ?? '';
        const permit = line.match(/@unsafe-cast\s+reason=\S+\s+issue=DME-\d+\s+expires=(\d{4}-\d{2}-\d{2})/);
        const validPermit = permit?.[1] !== undefined && permit[1] >= new Date().toISOString().slice(0, 10);
        if (!validPermit) add('AG-TS-009', node, compact(node.getText(source)));
      }
      if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'eval')
        add('AG-TS-003', node, 'eval');
      if (ts.isNewExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'Function')
        add('AG-TS-003', node, 'Function');
      if (ts.isCatchClause(node) && node.block.statements.length === 0) add('AG-TS-004', node, 'empty-catch');
      ts.forEachChild(node, visit);
    };
    visit(source);
    const lines = text.split(/\r?\n/);
    for (let index = 0; index < lines.length; index++) {
      const line = lines[index];
      const lexical = (rule: string, marker: string): void => {
        const column = line.indexOf(marker);
        if (column < 0) return;
        const excerpt = compact(line);
        raw.push({ rule, file, line: index + 1, column: column + 1, excerpt,
          identity: `${file}\0${rule}\0${marker}\0${excerpt}` });
      };
      for (const marker of ['@ts-ignore', '@ts-nocheck', '@ts-expect-error', 'eslint-disable', 'biome-ignore'])
        lexical('AG-TS-002', marker);
      for (const marker of ['TODO', 'FIXME', 'HACK']) lexical('AG-TS-005', marker);
      if (line.includes('@alloc-allowed') &&
          !/@alloc-allowed\s+reason=\S+\s+issue=DME-\d+\s+expires=\d{4}-\d{2}-\d{2}/.test(line))
        lexical('AG-TS-007', '@alloc-allowed');
      lexical('AG-TS-008', '@architecture-allow');
      if (line.includes('@unsafe-cast')) {
        const permit = line.match(/@unsafe-cast\s+reason=\S+\s+issue=DME-\d+\s+expires=(\d{4}-\d{2}-\d{2})/);
        if (permit?.[1] === undefined || permit[1] < new Date().toISOString().slice(0, 10))
          lexical('AG-TS-010', '@unsafe-cast');
      }
    }
  }
  raw.sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line || a.column - b.column || a.rule.localeCompare(b.rule));
  const occurrences = new Map<string, number>();
  return raw.map(({ identity, ...finding }) => {
    const occurrence = occurrences.get(identity) ?? 0;
    occurrences.set(identity, occurrence + 1);
    const id = createHash('sha256').update(`${identity}\0${occurrence}`).digest('hex').slice(0, 20);
    return { id, ...finding };
  });
}

if (import.meta.main) {
  const findings = await scanSourceHygiene();
  if (process.argv.includes('--write-baseline')) {
    await writeFile(baselinePath, `${JSON.stringify({ version: baselineVersion, findings }, null, 2)}\n`);
    console.log(`wrote ${findings.length} frozen source-hygiene violation(s) to ${relative(root, baselinePath)}`);
    process.exit(0);
  }
  let baseline: Baseline;
  try { baseline = JSON.parse(await readFile(baselinePath, 'utf8')) as Baseline; }
  catch { console.error('source-hygiene baseline is missing; bootstrap it explicitly with --write-baseline'); process.exit(1); }
  if (baseline.version !== baselineVersion || !Array.isArray(baseline.findings)) {
    console.error('source-hygiene baseline has an unsupported or malformed schema'); process.exit(1);
  }
  const accepted = new Set(baseline.findings.map((finding) => finding.id));
  const current = new Set(findings.map((finding) => finding.id));
  let failures = 0;
  for (const finding of findings) if (!accepted.has(finding.id)) {
    console.error(`${finding.file}:${finding.line}:${finding.column}: ${finding.rule}: NEW: ${descriptions[finding.rule]} :: ${finding.excerpt}`);
    failures++;
  }
  for (const finding of baseline.findings) if (!current.has(finding.id)) {
    console.error(`${finding.file}:${finding.line}:${finding.column}: stale source-hygiene baseline entry ${finding.rule}; remove it`);
    failures++;
  }
  const ref = compareRef();
  if (ref) {
    const old = atRef(ref);
    if (old) {
      const oldIds = new Set(old.findings.map((finding) => finding.id));
      for (const finding of baseline.findings) if (!oldIds.has(finding.id)) {
        console.error(`${finding.file}:${finding.line}:${finding.column}: baseline addition forbidden relative to ${ref}: ${finding.rule}`);
        failures++;
      }
    } else console.warn(`source-hygiene ratchet bootstrap: ${ref} has no readable baseline`);
  }
  if (failures !== 0) { console.error(`source-hygiene ratchet failed with ${failures} error(s)`); process.exit(1); }
  console.log(`source-hygiene ratchet passed (${findings.length} frozen violation(s), zero new)`);
}
