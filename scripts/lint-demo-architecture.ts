#!/usr/bin/env bun
/** Ratcheting architecture lint for first-party visual demos and launchers. */
import ts from '../crates/afterglow-web/www/node_modules/typescript/lib/typescript.js';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { basename, join, relative, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import type { EngineConformance, WebArtifactManifest } from './check-web-contracts.ts';

export interface ArchitectureFinding {
  id: string;
  rule: string;
  file: string;
  line: number;
  column: number;
  excerpt: string;
}
interface RawFinding extends Omit<ArchitectureFinding, 'id'> { identity: string }
interface ArchitectureBaseline { version: number; findings: ArchitectureFinding[] }
const baselineVersion = 2;

const root = resolve(import.meta.dir, '..');
const www = join(root, 'crates/afterglow-web/www');
const baselinePath = join(www, 'demo-architecture-baseline.json');
const manifest = JSON.parse(await readFile(join(www, 'web-artifacts.json'), 'utf8')) as WebArtifactManifest;
const conformance = JSON.parse(await readFile(join(www, 'engine-conformance.json'), 'utf8')) as EngineConformance;
const descriptions: Record<string, string> = {
  'AG-DEMO-001': 'demo owns requestAnimationFrame/setAnimationLoop instead of EngineRuntime',
  'AG-DEMO-002': 'demo constructs engine lifecycle/infrastructure directly',
  'AG-DEMO-003': 'demo orchestrates low-level VT feedback directly',
  'AG-DEMO-004': 'global window engine/Three namespace bridge',
  'AG-DEMO-005': 'direct renderer backend/private Three API access',
  'AG-DEMO-006': 'unbounded demo error/waiter/key collection',
  'AG-DEMO-007': 'demo assembles BIG/range/page-provider infrastructure',
  'AG-DEMO-008': 'demo owns glTF texture/material replacement bookkeeping',
  'AG-DEMO-009': 'demo assembles POM/surface-detail shader graph',
  'AG-DEMO-010': 'registered frame callback lacks @alloc-effect none',
  'AG-DEMO-011': 'CEF launcher embeds an inline JavaScript redirect',
  'AG-DEMO-012': 'inline authored browser script',
  'AG-DEMO-013': 'demo owns raw event-listener lifecycle',
  'AG-DEMO-014': 'demo writes diagnostic UI directly',
  'AG-DEMO-015': 'demo erases type safety with any',
  'AG-DEMO-016': 'demo imports engine implementation modules instead of the public barrel',
  'AG-DEMO-017': 'demo assembles a raw RPC worker transport instead of using a typed worker factory',
};
const infrastructureConstructors = new Set([
  'WebGPURenderer', 'RendererSeal', 'EngineMemory', 'FrameBudget', 'RenderAdapter',
  'AssetStore', 'AsyncWorker', 'Worker', 'RingBuf', 'BoundedTranscoderPool',
  'VirtualTextureStore', 'VirtualTextureFeedbackPass',
]);
const bigCalls = new Set([
  'createFetchRangeLoader', 'parseBigHeader', 'createRawAssetLoader',
  'createPageDataProvider', 'readBigPrefix', 'selectGpuTextureFormat',
]);
const pomNames = new Set([
  'POM_UV_WGSL', 'POM_SELF_SHADOW_WGSL', 'buildPomUvNode', 'buildPomSelfShadowNode',
  'virtualTexturePbrSampleAtLevel', 'virtualTextureSampleAtLevel',
  'virtualTextureDisplacedFallbackUv', 'PomLightingModel',
]);
const replacementNames = /^(?:secondLayouts|materialByName|originalMaterials|originalTextures|sourceMaterials|sourceTextures|textureProps|feedbackRecords|model[12]Records)$/i;
const unboundedNames = /^(?:errors|waiters|frameWaiters|keys|pressedKeys)$/i;

function compact(text: string): string {
  return text.replace(/\s+/g, ' ').trim().slice(0, 180);
}
function nodeName(node: ts.Node): string {
  if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) return node.text;
  if (ts.isPropertyAccessExpression(node)) return node.name.text;
  if (ts.isElementAccessExpression(node) && node.argumentExpression && ts.isStringLiteral(node.argumentExpression))
    return node.argumentExpression.text;
  return '';
}
function hasNoneEffect(source: ts.SourceFile, callback: ts.Expression): boolean {
  if (!ts.isIdentifier(callback)) return false;
  let declaration: ts.Node | undefined;
  const find = (node: ts.Node): void => {
    if (declaration) return;
    if (ts.isFunctionDeclaration(node) && node.name?.text === callback.text) declaration = node;
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === callback.text)
      declaration = node;
    ts.forEachChild(node, find);
  };
  find(source);
  if (!declaration) return false;
  const full = source.text.slice(declaration.getFullStart(), declaration.getStart(source));
  return /@alloc-effect\s+none\b/.test(full);
}
function rawFinding(source: ts.SourceFile, file: string, rule: string, node: ts.Node, identity?: string): RawFinding {
  const position = source.getLineAndCharacterOfPosition(node.getStart(source));
  const excerpt = compact(node.getText(source));
  return {
    rule, file, line: position.line + 1, column: position.character + 1, excerpt,
    identity: `${file}\0${rule}\0${identity ?? excerpt}`,
  };
}

export function scanTypeScript(sourceText: string, file: string): RawFinding[] {
  const source = ts.createSourceFile(file, sourceText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const findings: RawFinding[] = [];
  const add = (rule: string, node: ts.Node, identity?: string): void => {
    findings.push(rawFinding(source, file, rule, node, identity));
  };
  const visit = (node: ts.Node): void => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier)) {
      const specifier = node.moduleSpecifier.text;
      const publicBarrel = /^(?:\.\/|\.\.\/)engine\/(?:index|[a-z0-9-]+-api)\.ts$/.test(specifier);
      if (!publicBarrel && (specifier.startsWith('./engine/') || specifier.startsWith('../engine/')))
        add('AG-DEMO-016', node, specifier);
      if (specifier.endsWith('.client.ts')) add('AG-DEMO-017', node, specifier);
      if (specifier.endsWith('/surface-detail.ts') || specifier.endsWith('/virtual-texture-material.ts'))
        add('AG-DEMO-009', node, specifier);
    }
    if (node.kind === ts.SyntaxKind.AnyKeyword) add('AG-DEMO-015', node, 'explicit-any');
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
      if (unboundedNames.test(node.name.text) && node.initializer &&
          (ts.isArrayLiteralExpression(node.initializer) ||
           (ts.isNewExpression(node.initializer) && nodeName(node.initializer.expression) === 'Set')))
        add('AG-DEMO-006', node, node.name.text);
      if (replacementNames.test(node.name.text)) add('AG-DEMO-008', node, node.name.text);
    }
    if (ts.isNewExpression(node)) {
      const name = nodeName(node.expression);
      if (infrastructureConstructors.has(name)) add('AG-DEMO-002', node, name);
      if (name === 'VirtualTextureFeedbackPass') add('AG-DEMO-003', node, name);
      if (name.endsWith('Client')) add('AG-DEMO-017', node, name);
    }
    if (ts.isCallExpression(node)) {
      const name = nodeName(node.expression);
      if (name === 'requestAnimationFrame' || name === 'setAnimationLoop') {
        add('AG-DEMO-001', node, `${name}:${compact(node.arguments[0]?.getText(source) ?? '')}`);
        const callback = node.arguments[0];
        if (!callback || !hasNoneEffect(source, callback))
          add('AG-DEMO-010', node, `${name}:${compact(callback?.getText(source) ?? '')}`);
      }
      if (name === 'start' && ts.isPropertyAccessExpression(node.expression) &&
          /runtime/i.test(node.expression.expression.getText(source))) {
        const client = node.arguments[0];
        let callback: ts.Expression | undefined;
        if (client && ts.isObjectLiteralExpression(client)) {
          for (const property of client.properties) {
            if (ts.isPropertyAssignment(property) && property.name.getText(source) === 'update')
              callback = property.initializer;
          }
        } else if (client && ts.isIdentifier(client)) {
          for (const statement of source.statements) {
            if (!ts.isVariableStatement(statement)) continue;
            for (const declaration of statement.declarationList.declarations) {
              if (!ts.isIdentifier(declaration.name) || declaration.name.text !== client.text ||
                  !declaration.initializer || !ts.isObjectLiteralExpression(declaration.initializer)) continue;
              for (const property of declaration.initializer.properties) {
                if (ts.isPropertyAssignment(property) && property.name.getText(source) === 'update')
                  callback = property.initializer;
              }
            }
          }
        }
        if (!callback || !hasNoneEffect(source, callback))
          add('AG-DEMO-010', node, `EngineRuntime.start:${compact(callback?.getText(source) ?? '')}`);
      }
      if (name === 'addEventListener' || name === 'removeEventListener') add('AG-DEMO-013', node, compact(node.getText(source)));
      if (name === 'submit' || name === 'consume' || name === 'mergeFeedbackMaps') {
        const target = ts.isPropertyAccessExpression(node.expression) ? node.expression.expression.getText(source) : '';
        if (/feedback/i.test(target)) add('AG-DEMO-003', node, `${target}.${name}`);
      }
      const callTarget = node.expression.getText(source);
      if (callTarget === 'Rpc.create' || /Client\.(?:spawn|spawnThreaded)$/.test(callTarget))
        add('AG-DEMO-017', node, callTarget);
      if (bigCalls.has(name)) add('AG-DEMO-007', node, name);
      if (pomNames.has(name)) add('AG-DEMO-009', node, name);
    }
    if (ts.isPropertyAccessExpression(node)) {
      const text = node.getText(source);
      if (/^window\.(?:THREE|Afterglow\w*|bitecs\w*)\b/.test(text)) add('AG-DEMO-004', node, text);
      if (node.name.text === 'backend' || node.name.text.startsWith('_') ||
          node.name.text === 'afterglowAdapterInfo' || node.name.text === 'createShaderModule')
        add('AG-DEMO-005', node, text);
      if (node.name.text === 'innerHTML' || node.name.text === 'textContent') add('AG-DEMO-014', node, text);
    }
    if (ts.isIdentifier(node) && pomNames.has(node.text)) add('AG-DEMO-009', node, node.text);
    ts.forEachChild(node, visit);
  };
  visit(source);
  return findings;
}

function finalize(raw: RawFinding[]): ArchitectureFinding[] {
  const counts = new Map<string, number>();
  return raw.sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line || a.column - b.column || a.rule.localeCompare(b.rule))
    .map(({ identity, ...finding }) => {
      const occurrence = counts.get(identity) ?? 0;
      counts.set(identity, occurrence + 1);
      const id = createHash('sha256').update(`${identity}\0${occurrence}`).digest('hex').slice(0, 20);
      return { id, ...finding };
    });
}

async function scanRustLaunchers(): Promise<RawFinding[]> {
  const findings: RawFinding[] = [];
  const glob = new Bun.Glob('*.rs');
  const directory = join(root, 'crates/afterglow-cef/examples');
  for await (const name of glob.scan({ cwd: directory, onlyFiles: true })) {
    const text = await readFile(join(directory, name), 'utf8');
    for (const match of text.matchAll(/\.index_html\s*\(\s*b?"[^"\n]*<script[\s\S]*?<\/script>[^"\n]*"\s*\)/g)) {
      const before = text.slice(0, match.index);
      const line = before.split('\n').length;
      const excerpt = compact(match[0]);
      findings.push({
        rule: 'AG-DEMO-011', file: `crates/afterglow-cef/examples/${name}`,
        line, column: 1, excerpt, identity: `crates/afterglow-cef/examples/${name}\0AG-DEMO-011\0${excerpt}`,
      });
    }
  }
  return findings;
}

export async function scanArchitecture(): Promise<ArchitectureFinding[]> {
  const raw: RawFinding[] = [];
  for (const artifact of manifest.artifacts) {
    if (artifact.architectureChecked !== true) continue;
    raw.push(...scanTypeScript(await readFile(join(www, artifact.source), 'utf8'), artifact.source));
  }
  raw.push(...await scanRustLaunchers());
  return finalize(raw);
}

function parseCompareRef(): string | null {
  const index = process.argv.indexOf('--compare-ref');
  return index < 0 ? null : process.argv[index + 1] ?? null;
}
function baselineAtRef(ref: string): ArchitectureBaseline | null {
  const path = 'crates/afterglow-web/www/demo-architecture-baseline.json';
  const result = spawnSync('git', ['show', `${ref}:${path}`], { cwd: root, encoding: 'utf8' });
  if (result.status !== 0) return null;
  try {
    const baseline = JSON.parse(result.stdout) as ArchitectureBaseline;
    return baseline.version === baselineVersion ? baseline : null;
  } catch { return null; }
}

if (import.meta.main) {
  const findings = await scanArchitecture();
  if (process.argv.includes('--json')) {
    console.log(JSON.stringify({ version: baselineVersion, findings }, null, 2));
    process.exit(0);
  }
  if (process.argv.includes('--write-baseline')) {
    const baseline: ArchitectureBaseline = { version: baselineVersion, findings };
    await writeFile(baselinePath, `${JSON.stringify(baseline, null, 2)}\n`);
    console.log(`wrote ${findings.length} frozen architecture violation(s) to ${relative(root, baselinePath)}`);
    process.exit(0);
  }

  let baseline: ArchitectureBaseline;
  try { baseline = JSON.parse(await readFile(baselinePath, 'utf8')) as ArchitectureBaseline; }
  catch {
    if (conformance.releaseStatus === 'conformant') baseline = { version: baselineVersion, findings: [] };
    else { console.error('architecture baseline is missing; bootstrap it explicitly with --write-baseline'); process.exit(1); }
  }
  if (baseline.version !== baselineVersion || !Array.isArray(baseline.findings)) {
    console.error('architecture baseline has an unsupported or malformed schema');
    process.exit(1);
  }
  const accepted = new Map(baseline.findings.map((finding) => [finding.id, finding]));
  const current = new Map(findings.map((finding) => [finding.id, finding]));
  let failures = 0;
  for (const [source, state] of Object.entries(conformance.visualEntrypoints)) {
    if (state === 'legacy' && !findings.some((finding) => finding.file === source)) {
      console.error(`${source}: legacy status is forbidden without a frozen architecture violation; mark it canonical`);
      failures++;
    }
  }
  for (const finding of findings) {
    const state = conformance.visualEntrypoints[finding.file];
    if (state === 'canonical') {
      console.error(`${finding.file}:${finding.line}:${finding.column}: ${finding.rule}: canonical demo violation: ${descriptions[finding.rule]}`);
      failures++;
    } else if (!accepted.has(finding.id)) {
      console.error(`${finding.file}:${finding.line}:${finding.column}: ${finding.rule}: NEW violation: ${descriptions[finding.rule]} :: ${finding.excerpt}`);
      failures++;
    }
  }
  for (const finding of baseline.findings) if (!current.has(finding.id)) {
    console.error(`${finding.file}:${finding.line}:${finding.column}: stale baseline violation ${finding.rule}; remove it from the baseline`);
    failures++;
  }

  const compareRef = parseCompareRef();
  if (compareRef) {
    const old = baselineAtRef(compareRef);
    if (old) {
      const oldIds = new Set(old.findings.map((finding) => finding.id));
      for (const finding of baseline.findings) if (!oldIds.has(finding.id)) {
        console.error(`${finding.file}:${finding.line}:${finding.column}: baseline addition forbidden relative to ${compareRef}: ${finding.rule}`);
        failures++;
      }
    } else {
      console.warn(`architecture ratchet bootstrap: ${compareRef} has no readable baseline`);
    }
  }

  if (failures !== 0) {
    console.error(`demo architecture lint failed with ${failures} error(s)`);
    process.exit(1);
  }
  console.log(`demo architecture ratchet passed (${findings.length} frozen violation(s), zero new)`);
}
