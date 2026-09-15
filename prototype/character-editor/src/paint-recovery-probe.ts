import './paint-main.ts';

const host = globalThis as typeof globalThis & { __documentLoaded?: boolean };
const page = window as typeof window & {
  probe(y: number): number;
  __probeResult?: { id: number; native?: boolean; colored: number };
};
const wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
function element<T extends HTMLElement>(id: string): T {
  const value = document.getElementById(id);
  if (!value) throw new Error(`Missing recovery control: ${id}`);
  return value as T;
}
function press(id: string) { element<HTMLButtonElement>(id).click(); }
async function until(condition: () => boolean, label: string) {
  const end = performance.now() + 30_000;
  while (!condition()) {
    if (performance.now() >= end) throw new Error(`Recovery deadline: ${label}`);
    await wait(25);
  }
}
async function pixels() {
  const id = page.probe(0.5);
  await until(() => page.__probeResult?.id === id, 'pixels');
  if (!page.__probeResult?.native) throw new Error('Recovery check needs native paint');
  const canvas = element<HTMLCanvasElement>('paint');
  const data = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
  let hash = 2166136261;
  for (const byte of data) hash = Math.imul(hash ^ byte, 16777619) >>> 0;
  return { width: canvas.width, height: canvas.height, hash, colored: page.__probeResult.colored,
    layers: Array.from(document.querySelectorAll('.layer-name')).map(node => node.textContent) };
}
function same(a: unknown, b: unknown) {
  if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`Recovery mismatch: ${JSON.stringify({ a, b })}`);
}
async function run() {
  await until(() => host.__documentLoaded === true, 'native input startup');
  const prompt = element('paintRecovery');
  const hadRecovery = prompt.style.display !== 'none';
  if (hadRecovery) {
    const points: Record<string, { x: number; y: number }> = {};
    for (const decision of ['restore', 'discard']) {
      const button = element<HTMLButtonElement>(`${decision}PaintBtn`);
      const r = button.getBoundingClientRect();
      if (button.disabled || r.width <= 0 || r.height <= 0) throw new Error('Recovery button is not available');
      const x = r.left + r.width / 2, y = r.top + r.height / 2;
      if (!button.contains(document.elementFromPoint(x, y))) throw new Error('Recovery button hit test failed');
      points[decision] = { x: Math.round(x * devicePixelRatio), y: Math.round(y * devicePixelRatio) };
    }
    console.log('[paint-recovery-click]', JSON.stringify(points));
    await until(() => prompt.style.display === 'none', 'physical recovery button');
    const restored = await pixels();
    if (restored.colored > 0) {
      if (restored.width !== 512 || restored.height !== 384 || restored.layers.length !== 3)
        throw new Error('Recovered dimensions or composition are incorrect');
      press('undoBtn');
      const undone = await pixels();
      if (undone.colored !== 0) throw new Error('Recovered undo did not remove the stroke');
      press('redoBtn');
      same(await pixels(), restored);
      console.log('[paint-recovery-result]', JSON.stringify({ phase: 'restore', snapshot: restored }));
    } else {
      if (restored.layers.length !== 1) throw new Error('Discard retained old layers');
      press('undoBtn');
      same(await pixels(), restored);
      press('redoBtn');
      same(await pixels(), restored);
      console.log('[paint-recovery-result]', JSON.stringify({ phase: 'discard', snapshot: restored }));
    }
    return;
  }
  element<HTMLInputElement>('documentWidth').value = '512';
  element<HTMLInputElement>('documentHeight').value = '384';
  press('newDocumentBtn');
  await pixels();
  press('addLayerBtn');
  await pixels();
  press('addGroupBtn');
  await pixels();
  const parent = element<HTMLSelectElement>('layerParent');
  parent.value = '0';
  parent.dispatchEvent(new Event('change', { bubbles: true }));
  await pixels();
  press('strokeBtn');
  const painted = await pixels();
  if (painted.colored === 0 || painted.layers.length !== 3) throw new Error('Recovery seed did not paint the composition');
  console.log('[paint-recovery-result]', JSON.stringify({ phase: 'seed', snapshot: painted }));
}
// Native input starts only after module evaluation completes.
void run().catch(error => console.error('[paint-recovery-failed]', error instanceof Error ? error.stack : String(error)));
