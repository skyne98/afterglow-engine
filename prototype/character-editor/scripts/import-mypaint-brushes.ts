import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

const projectRoot = path.resolve(import.meta.dir, '..');
const sourceRoot = process.argv[2] ?? process.env.MYPAINT_BRUSHES_DIR;
const outputRoot = path.join(projectRoot, 'public', 'mypaint', 'brushes');
const manifestPath = path.join(projectRoot, 'public', 'mypaint', 'brushes.json');

if (!sourceRoot) {
  throw new Error('Give the MyPaint brush directory as an argument or MYPAINT_BRUSHES_DIR.');
}

const source = path.resolve(sourceRoot);
const allowedExtensions = new Set(['.myb', '.png', '.txt', '.conf']);

function safeSegment(segment: string): string {
  return segment.replace(/[^A-Za-z0-9._-]/g, (character) =>
    `_x${character.codePointAt(0)!.toString(16)}_`);
}

function safeRelative(relative: string): string {
  return relative.split('/').map(safeSegment).join('/');
}

async function collectFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    const filePath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...await collectFiles(filePath));
    } else if (allowedExtensions.has(path.extname(entry.name).toLowerCase())) {
      files.push(filePath);
    }
  }
  return files;
}

await rm(outputRoot, { recursive: true, force: true });
await mkdir(outputRoot, { recursive: true });

const sourceFiles = await collectFiles(source);
for (const sourceFile of sourceFiles) {
  const relative = path.relative(source, sourceFile).split(path.sep).join('/');
  const target = path.join(outputRoot, safeRelative(relative));
  await mkdir(path.dirname(target), { recursive: true });
  await cp(sourceFile, target);
}

const brushFiles = sourceFiles
  .filter((filePath) => filePath.endsWith('.myb'))
  .map((filePath) => path.relative(source, filePath).split(path.sep).join('/'))
  .sort((a, b) => a.localeCompare(b));

const brushes = [];
for (const brushPath of brushFiles) {
  const sourceFile = path.join(source, brushPath);
  const data = JSON.parse(await readFile(sourceFile, 'utf8')) as {
    description?: string;
    group?: string;
    parent_brush_name?: string;
  };
  const stem = brushPath.slice(0, -'.myb'.length);
  const previewPath = `${stem}_prev.png`;
  const group = data.group || path.posix.dirname(brushPath).split('/')[0];
  brushes.push({
    id: stem,
    name: data.description || path.posix.basename(stem),
    group,
    brush: safeRelative(brushPath),
    preview: safeRelative(previewPath),
    parentBrushName: data.parent_brush_name || stem,
  });
}

await writeFile(manifestPath, `${JSON.stringify({
  source: 'mypaint-brushes 2.0.2',
  count: brushes.length,
  brushes,
}, null, 2)}\n`);

console.log(`Imported ${brushes.length} MyPaint brushes and their previews.`);
