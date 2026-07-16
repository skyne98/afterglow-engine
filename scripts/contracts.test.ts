import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { countBundledThreeCoreCopies, validateWebContracts } from './check-web-contracts.ts';
import { scanTypeScript } from './lint-demo-architecture.ts';

const temporary: string[] = [];
afterEach(async () => {
  while (temporary.length !== 0) await rm(temporary.pop()!, { recursive: true, force: true });
});

async function fixture(options: {
  html?: string;
  conformance?: Record<string, string>;
  releaseStatus?: string;
  extraJs?: string;
  legacyBridge?: boolean;
} = {}): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'afterglow-contract-'));
  temporary.push(root);
  const www = join(root, 'crates/afterglow-web/www');
  await mkdir(join(www, 'engine'), { recursive: true });
  await writeFile(join(www, 'demo.ts'), 'export {};\n');
  await writeFile(join(www, 'demo.js'), 'export {};\n');
  await writeFile(join(www, 'demo.html'), options.html ??
    `${options.legacyBridge ? '<script src="bridge.js"></script>' : ''}<script type="module" src="./demo.js"></script>\n`);
  if (options.extraJs) await writeFile(join(www, options.extraJs), 'bad();\n');
  if (options.legacyBridge) {
    await writeFile(join(www, 'bridge.ts'), 'export {};\n');
    await writeFile(join(www, 'bridge.js'), 'export {};\n');
  }
  const artifacts: Array<Record<string, unknown>> = [{
    source: 'demo.ts', output: 'demo.js', role: 'visual-demo',
    pages: ['demo.html'], architectureChecked: true,
  }];
  if (options.legacyBridge)
    artifacts.push({ source: 'bridge.ts', output: 'bridge.js', role: 'legacy-bridge', architectureChecked: true });
  await writeFile(join(www, 'web-artifacts.json'), JSON.stringify({ version: 1, artifacts }));
  await writeFile(join(www, 'engine-conformance.json'), JSON.stringify({
    version: 1,
    releaseStatus: options.releaseStatus ?? 'conformant',
    visualEntrypoints: options.conformance ?? { 'demo.ts': 'canonical' },
  }));
  return root;
}

describe('web artifact/conformance contract', () => {
  test('accepts one fully declared canonical page', async () => {
    expect(await validateWebContracts(await fixture())).toEqual([]);
  });

  test('rejects inline authored scripts', async () => {
    const errors = await validateWebContracts(await fixture({ html: '<script>bad()</script>' }));
    expect(errors.some((error) => error.includes('inline authored script'))).toBe(true);
    expect(errors.some((error) => error.includes('does not load its owned output'))).toBe(true);
  });

  test('rejects unclassified generated or authored JavaScript', async () => {
    const errors = await validateWebContracts(await fixture({ extraJs: 'escape.js' }));
    expect(errors.some((error) => error.includes('escape.js') && error.includes('not classified'))).toBe(true);
  });

  test('rejects a legacy bridge on a canonical page', async () => {
    const errors = await validateWebContracts(await fixture({ legacyBridge: true }));
    expect(errors.some((error) => error.includes('canonical visual demo may not load legacy bridge'))).toBe(true);
    expect(errors.some((error) => error.includes('conformant release may not retain legacy-bridge'))).toBe(true);
  });

  test('rejects false conformant release claims', async () => {
    const errors = await validateWebContracts(await fixture({
      releaseStatus: 'conformant', conformance: { 'demo.ts': 'legacy' },
    }));
    expect(errors.some((error) => error.includes('conformant release has legacy demos'))).toBe(true);
  });

  test('rejects stale conformance entries', async () => {
    const errors = await validateWebContracts(await fixture({
      releaseStatus: 'migration', conformance: { 'demo.ts': 'legacy', 'gone.ts': 'legacy' },
    }));
    expect(errors.some((error) => error.includes('stale/non-visual entrypoint gone.ts'))).toBe(true);
  });
});

describe('bundle identity contract', () => {
  test('detects duplicate bundled Three.js cores', () => {
    const marker = '// node_modules/three/build/three.core.js';
    expect(countBundledThreeCoreCopies(`${marker}\ncode`)).toBe(1);
    expect(countBundledThreeCoreCopies(`${marker}\n${marker}`)).toBe(2);
  });
});

describe('demo architecture scanner', () => {
  test('finds lifecycle, globals, any, internals, allocations, and untyped frame callbacks', () => {
    const findings = scanTypeScript(`
      import { Thing } from './engine/private.ts';
      const errors = [];
      function frame(value: any) { window.AfterglowMemory = value; renderer.backend._secret(); }
      addEventListener('error', frame);
      requestAnimationFrame(frame);
      new WebGPURenderer();
    `, 'bad.ts');
    const rules = new Set(findings.map((finding) => finding.rule));
    for (const rule of [
      'AG-DEMO-001', 'AG-DEMO-002', 'AG-DEMO-004', 'AG-DEMO-005',
      'AG-DEMO-006', 'AG-DEMO-010', 'AG-DEMO-013', 'AG-DEMO-015', 'AG-DEMO-016',
    ]) expect(rules.has(rule)).toBe(true);
  });

  test('allows explicit subsystem API barrels but rejects implementation modules', () => {
    const findings = scanTypeScript(`
      import { ModelPrimitives } from './engine/model-api.ts';
      import { VirtualGltfBinding } from './engine/virtual-texturing-api.ts';
    `, 'barrels.ts');
    expect(findings.some((finding) => finding.rule === 'AG-DEMO-016')).toBe(false);
  });

  test('rejects an EngineRuntime client whose update is not allocation-effect declared', () => {
    const findings = scanTypeScript(`
      function updateFrame(): void {}
      const frameClient = { update: updateFrame };
      runtime.start(frameClient);
    `, 'runtime-client.ts');
    expect(findings.some((finding) => finding.rule === 'AG-DEMO-010')).toBe(true);
  });

  test('recognizes an explicitly allocation-free frame callback', () => {
    const findings = scanTypeScript(`
      /** @alloc-effect none */
      function frame(): void {}
      requestAnimationFrame(frame);
    `, 'frame.ts');
    expect(findings.some((finding) => finding.rule === 'AG-DEMO-001')).toBe(true);
    expect(findings.some((finding) => finding.rule === 'AG-DEMO-010')).toBe(false);
  });
});
