#!/usr/bin/env bun
/** Harsh authored-web inventory and conformance validation. */
import { access, readFile, readdir } from 'node:fs/promises';
import { dirname, join, normalize, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export type ArtifactRole = 'runtime' | 'worker' | 'visual-demo' | 'diagnostic' | 'legacy-bridge';
export interface WebArtifact {
  source: string;
  output: string;
  role: ArtifactRole;
  pages?: string[];
  architectureChecked?: boolean;
}
export interface WebArtifactManifest { version: number; artifacts: WebArtifact[] }
export interface EngineConformance {
  version: number;
  releaseStatus: 'converging' | 'conformant';
  visualEntrypoints: Record<string, 'legacy' | 'canonical'>;
}

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultRoot = resolve(scriptDir, '..');
const validRoles = new Set<ArtifactRole>(['runtime', 'worker', 'visual-demo', 'diagnostic', 'legacy-bridge']);
const artifactKeys = new Set(['source', 'output', 'role', 'pages', 'architectureChecked']);

function isSafeRelative(path: unknown, extension: string): path is string {
  if (typeof path !== 'string' || !path.endsWith(extension) || path.startsWith('/') || path.includes('\\')) return false;
  const clean = normalize(path).replaceAll('\\', '/');
  return clean === path && clean !== '..' && !clean.startsWith('../');
}

async function exists(path: string): Promise<boolean> {
  try { await access(path); return true; } catch { return false; }
}

async function list(directory: string, suffix: string): Promise<string[]> {
  return (await readdir(directory, { withFileTypes: true }))
    .filter((entry) => entry.isFile() && entry.name.endsWith(suffix))
    .map((entry) => entry.name)
    .sort();
}

async function listRecursive(directory: string, suffix: string, prefix = ''): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) files.push(...await listRecursive(join(directory, entry.name), suffix, path));
    else if (entry.isFile() && entry.name.endsWith(suffix)) files.push(path);
  }
  return files.sort();
}

export function countBundledThreeCoreCopies(source: string): number {
  return source.match(/node_modules\/three\/build\/three\.core\.js/g)?.length ?? 0;
}

function parseExternalScripts(html: string, page: string, errors: string[]): string[] {
  const outputs: string[] = [];
  const tags = html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi);
  for (const match of tags) {
    const attributes = match[1];
    const body = match[2].trim();
    const src = attributes.match(/\bsrc\s*=\s*(["'])(.*?)\1/i)?.[2];
    if (!src) {
      if (body.length !== 0) errors.push(`${page}: inline authored script is forbidden`);
      continue;
    }
    if (body.length !== 0) errors.push(`${page}: script with src must have an empty body`);
    if (/^(?:https?:|data:|blob:|\/\/)/i.test(src)) {
      errors.push(`${page}: remote/dynamic script source is forbidden: ${src}`);
      continue;
    }
    const clean = src.split(/[?#]/, 1)[0].replace(/^\.\//, '').replace(/^\//, '');
    outputs.push(clean);
  }
  return outputs;
}

export async function validateWebContracts(root = defaultRoot): Promise<string[]> {
  const errors: string[] = [];
  const web = join(root, 'crates/afterglow-web/web');
  const publicDirectory = join(web, 'public');
  const contracts = join(web, 'contracts');
  const manifestPath = join(contracts, 'web-artifacts.json');
  const conformancePath = join(contracts, 'engine-conformance.json');
  let manifest: WebArtifactManifest;
  let conformance: EngineConformance;
  try { manifest = JSON.parse(await readFile(manifestPath, 'utf8')); }
  catch (error) { return [`cannot read web-artifacts.json: ${String(error)}`]; }
  try { conformance = JSON.parse(await readFile(conformancePath, 'utf8')); }
  catch (error) { return [`cannot read engine-conformance.json: ${String(error)}`]; }

  if (manifest.version !== 1) errors.push(`web-artifacts.json: unsupported version ${String(manifest.version)}`);
  if (!Array.isArray(manifest.artifacts) || manifest.artifacts.length === 0)
    errors.push('web-artifacts.json: artifacts must be a non-empty array');
  if (conformance.version !== 1) errors.push(`engine-conformance.json: unsupported version ${String(conformance.version)}`);
  if (conformance.releaseStatus !== 'converging' && conformance.releaseStatus !== 'conformant')
    errors.push(`engine-conformance.json: invalid releaseStatus ${String(conformance.releaseStatus)}`);
  if (!conformance.visualEntrypoints || Array.isArray(conformance.visualEntrypoints))
    errors.push('engine-conformance.json: visualEntrypoints must be an object');

  const sources = new Set<string>();
  const outputs = new Set<string>();
  const declaredPages = new Set<string>();
  const visualSources = new Set<string>();
  const outputOwners = new Map<string, WebArtifact>();
  for (const [index, artifact] of (manifest.artifacts ?? []).entries()) {
    const where = `web-artifacts.json: artifacts[${index}]`;
    if (!artifact || typeof artifact !== 'object') { errors.push(`${where}: must be an object`); continue; }
    for (const key of Object.keys(artifact)) if (!artifactKeys.has(key)) errors.push(`${where}: unknown key ${key}`);
    if (!isSafeRelative(artifact.source, '.ts')) errors.push(`${where}: unsafe/non-TypeScript source ${String(artifact.source)}`);
    if (!isSafeRelative(artifact.output, '.js')) errors.push(`${where}: unsafe/non-JavaScript output ${String(artifact.output)}`);
    if (!validRoles.has(artifact.role)) errors.push(`${where}: invalid role ${String(artifact.role)}`);
    if (sources.has(artifact.source)) errors.push(`${where}: duplicate source ${artifact.source}`);
    if (outputs.has(artifact.output)) errors.push(`${where}: duplicate output ${artifact.output}`);
    sources.add(artifact.source); outputs.add(artifact.output); outputOwners.set(artifact.output, artifact);
    if (!(await exists(join(web, artifact.source)))) errors.push(`${where}: source does not exist: ${artifact.source}`);
    if (artifact.pages !== undefined && !Array.isArray(artifact.pages)) errors.push(`${where}: pages must be an array`);
    for (const page of artifact.pages ?? []) {
      if (!isSafeRelative(page, '.html') || page.includes('/')) errors.push(`${where}: unsafe page ${String(page)}`);
      if (declaredPages.has(page)) errors.push(`${where}: page has multiple entry owners: ${page}`);
      declaredPages.add(page);
    }
    if (artifact.role === 'visual-demo') {
      visualSources.add(artifact.source);
      if (artifact.architectureChecked !== true) errors.push(`${where}: visual demo must set architectureChecked=true`);
      if (artifact.pages?.length !== 1) errors.push(`${where}: visual demo must own exactly one page`);
    }
    if (artifact.architectureChecked !== undefined && typeof artifact.architectureChecked !== 'boolean')
      errors.push(`${where}: architectureChecked must be boolean`);
  }

  const actualPages = await list(publicDirectory, '.html');
  for (const page of actualPages) if (!declaredPages.has(page)) errors.push(`${page}: HTML page is missing from web-artifacts.json`);
  for (const page of declaredPages) if (!actualPages.includes(page)) errors.push(`${page}: declared HTML page does not exist`);

  for (const page of actualPages) {
    const scripts = parseExternalScripts(await readFile(join(publicDirectory, page), 'utf8'), page, errors);
    for (const output of scripts) {
      const artifact = outputOwners.get(output);
      if (!artifact) { errors.push(`${page}: script output is not declared: ${output}`); continue; }
      const pageOwned = artifact.pages?.includes(page) ?? false;
      const shared = artifact.role === 'runtime' || artifact.role === 'legacy-bridge';
      if (!pageOwned && !shared) errors.push(`${page}: may not load ${artifact.role} artifact ${output}`);
      const owner = manifest.artifacts.find((candidate) => candidate.pages?.includes(page));
      if (artifact.role === 'legacy-bridge' && owner?.role === 'visual-demo' &&
          conformance.visualEntrypoints[owner.source] === 'canonical')
        errors.push(`${page}: canonical visual demo may not load legacy bridge ${output}`);
    }
    const owner = manifest.artifacts.find((artifact) => artifact.pages?.includes(page));
    if (owner && !scripts.includes(owner.output)) errors.push(`${page}: does not load its owned output ${owner.output}`);
  }

  for (const file of await listRecursive(join(web, 'src'), '.js'))
    errors.push(`${file}: hand-authored JavaScript is forbidden in web/src`);

  const states = conformance.visualEntrypoints ?? {};
  for (const source of visualSources) {
    if (!(source in states)) errors.push(`engine-conformance.json: missing visual entrypoint ${source}`);
    else if (states[source] !== 'legacy' && states[source] !== 'canonical')
      errors.push(`engine-conformance.json: invalid state for ${source}: ${String(states[source])}`);
  }
  for (const source of Object.keys(states)) if (!visualSources.has(source))
    errors.push(`engine-conformance.json: stale/non-visual entrypoint ${source}`);

  const legacy = Object.entries(states).filter(([, state]) => state === 'legacy').map(([source]) => source);
  const baselineExists = await exists(join(contracts, 'demo-architecture-baseline.json'));
  if (conformance.releaseStatus === 'conformant') {
    if (legacy.length !== 0) errors.push(`engine-conformance.json: conformant release has legacy demos: ${legacy.join(', ')}`);
    if (baselineExists) errors.push('engine-conformance.json: conformant release may not retain demo-architecture-baseline.json');
    if (manifest.artifacts.some((artifact) => artifact.role === 'legacy-bridge'))
      errors.push('engine-conformance.json: conformant release may not retain legacy-bridge artifacts');
  }

  return errors;
}

if (import.meta.main) {
  const errors = await validateWebContracts();
  if (errors.length !== 0) {
    for (const error of errors) console.error(`web-contract: ${error}`);
    console.error(`web contract failed with ${errors.length} error(s)`);
    process.exit(1);
  }
  console.log('web artifact and engine conformance contracts passed');
}
