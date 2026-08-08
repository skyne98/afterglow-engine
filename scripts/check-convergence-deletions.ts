#!/usr/bin/env bun
/** Deletion-first convergence ledger: removed paths/symbols may never return. */
import { lstat, readFile, readdir } from 'node:fs/promises';
import { dirname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export interface DeletionItem {
  id: string;
  status: 'pending' | 'removed';
  description: string;
  roots: string[];
  literals: string[];
  forbiddenPaths?: string[];
}
export interface DeletionLedger { version: number; items: DeletionItem[] }

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultRoot = resolve(scriptDir, '..');
const itemKeys = new Set(['id', 'status', 'description', 'roots', 'literals', 'forbiddenPaths']);
const sourceExtensions = ['.ts', '.rs', '.mjs', '.json'];

function safeRelative(path: unknown): path is string {
  if (typeof path !== 'string' || path.length === 0 || path.startsWith('/') || path.includes('\\')) return false;
  const clean = normalize(path).replaceAll('\\', '/');
  return clean === path && clean !== '..' && !clean.startsWith('../');
}

async function exists(path: string): Promise<boolean> {
  try { await lstat(path); return true; } catch { return false; }
}

async function sourceFiles(path: string): Promise<string[]> {
  let info;
  try { info = await lstat(path); } catch { return []; }
  if (info.isFile()) return sourceExtensions.some((extension) => path.endsWith(extension)) ? [path] : [];
  if (!info.isDirectory()) return [];
  const files: string[] = [];
  for (const entry of await readdir(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) files.push(...await sourceFiles(child));
    else if (entry.isFile() && sourceExtensions.some((extension) => entry.name.endsWith(extension))) files.push(child);
  }
  return files;
}

export async function validateConvergenceDeletions(root = defaultRoot): Promise<string[]> {
  const errors: string[] = [];
  let ledger: DeletionLedger;
  const path = join(root, 'crates/afterglow-web/web/contracts/convergence-deletions.json');
  try { ledger = JSON.parse(await readFile(path, 'utf8')) as DeletionLedger; }
  catch (error) { return [`cannot read convergence-deletions.json: ${String(error)}`]; }
  if (ledger.version !== 1 || !Array.isArray(ledger.items))
    return ['convergence-deletions.json has an unsupported or malformed schema'];
  const ids = new Set<string>();
  for (const [index, item] of ledger.items.entries()) {
    const where = `convergence-deletions.json: items[${index}]`;
    if (!item || typeof item !== 'object') { errors.push(`${where}: must be an object`); continue; }
    for (const key of Object.keys(item)) if (!itemKeys.has(key)) errors.push(`${where}: unknown key ${key}`);
    if (!/^CUE-DEL-\d{3}$/.test(item.id)) errors.push(`${where}: invalid id ${String(item.id)}`);
    else if (ids.has(item.id)) errors.push(`${where}: duplicate id ${item.id}`);
    else ids.add(item.id);
    if (item.status !== 'pending' && item.status !== 'removed') errors.push(`${where}: invalid status ${String(item.status)}`);
    if (typeof item.description !== 'string' || item.description.trim().length === 0)
      errors.push(`${where}: description must be non-empty`);
    if (!Array.isArray(item.roots) || item.roots.length === 0 || item.roots.some((rootPath) => !safeRelative(rootPath)))
      errors.push(`${where}: roots must contain safe relative paths`);
    if (!Array.isArray(item.literals) || item.literals.length === 0 ||
        item.literals.some((literal) => typeof literal !== 'string' || literal.length === 0))
      errors.push(`${where}: literals must be non-empty strings`);
    if (item.forbiddenPaths !== undefined && (!Array.isArray(item.forbiddenPaths) ||
        item.forbiddenPaths.some((forbidden) => !safeRelative(forbidden))))
      errors.push(`${where}: forbiddenPaths must contain safe relative paths`);
    if (errors.some((error) => error.startsWith(where))) continue;

    let matches = 0;
    for (const rootPath of item.roots) {
      for (const file of await sourceFiles(join(root, rootPath))) {
        const source = await readFile(file, 'utf8');
        for (const literal of item.literals) if (source.includes(literal)) matches++;
      }
    }
    for (const forbidden of item.forbiddenPaths ?? []) {
      if (await exists(join(root, forbidden))) {
        errors.push(`${item.id}: removed path was reintroduced: ${forbidden}`);
        matches++;
      }
    }
    if (item.status === 'removed' && matches !== 0)
      errors.push(`${item.id}: removed symbol/path was reintroduced (${item.description})`);
    if (item.status === 'pending' && matches === 0)
      errors.push(`${item.id}: pending deletion is gone; mark it removed to ratchet the contract`);
  }
  return errors;
}

if (import.meta.main) {
  const errors = await validateConvergenceDeletions();
  if (errors.length !== 0) {
    for (const error of errors) console.error(`deletion-ledger: ${error}`);
    console.error(`convergence deletion ledger failed with ${errors.length} error(s)`);
    process.exit(1);
  }
  console.log('convergence deletion ledger passed');
}
