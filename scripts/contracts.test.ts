import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { countBundledThreeCoreCopies, validateWebContracts } from './check-web-contracts.ts';
import { scanTypeScript } from './lint-demo-architecture.ts';
import { importBoundaryErrors } from './lint-import-boundaries.ts';

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
  const web = join(root, 'crates/afterglow-web/web');
  const source = join(web, 'src');
  const publicDirectory = join(web, 'public');
  const contracts = join(web, 'contracts');
  await mkdir(source, { recursive: true });
  await mkdir(publicDirectory, { recursive: true });
  await mkdir(contracts, { recursive: true });
  await writeFile(join(source, 'demo.ts'), 'export {};\n');
  await writeFile(join(publicDirectory, 'demo.html'), options.html ??
    `${options.legacyBridge ? '<script src="bridge.js"></script>' : ''}<script type="module" src="./demo.js"></script>\n`);
  if (options.extraJs) await writeFile(join(source, options.extraJs), 'bad();\n');
  if (options.legacyBridge) await writeFile(join(source, 'bridge.ts'), 'export {};\n');
  const artifacts: Array<Record<string, unknown>> = [{
    source: 'src/demo.ts', output: 'demo.js', role: 'visual-demo',
    pages: ['demo.html'], architectureChecked: true,
  }];
  if (options.legacyBridge)
    artifacts.push({ source: 'src/bridge.ts', output: 'bridge.js', role: 'legacy-bridge', architectureChecked: true });
  await writeFile(join(contracts, 'web-artifacts.json'), JSON.stringify({ version: 1, artifacts }));
  await writeFile(join(contracts, 'engine-conformance.json'), JSON.stringify({
    version: 1,
    releaseStatus: options.releaseStatus ?? 'conformant',
    visualEntrypoints: options.conformance ?? { 'src/demo.ts': 'canonical' },
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
    expect(errors.some((error) => error.includes('escape.js') && error.includes('hand-authored JavaScript'))).toBe(true);
  });

  test('rejects a legacy bridge on a canonical page', async () => {
    const errors = await validateWebContracts(await fixture({ legacyBridge: true }));
    expect(errors.some((error) => error.includes('canonical visual demo may not load legacy bridge'))).toBe(true);
    expect(errors.some((error) => error.includes('conformant release may not retain legacy-bridge'))).toBe(true);
  });

  test('rejects false conformant release claims', async () => {
    const errors = await validateWebContracts(await fixture({
      releaseStatus: 'conformant', conformance: { 'src/demo.ts': 'legacy' },
    }));
    expect(errors.some((error) => error.includes('conformant release has legacy demos'))).toBe(true);
  });

  test('rejects stale conformance entries', async () => {
    const errors = await validateWebContracts(await fixture({
      releaseStatus: 'migration', conformance: { 'src/demo.ts': 'legacy', 'gone.ts': 'legacy' },
    }));
    expect(errors.some((error) => error.includes('stale/non-visual entrypoint gone.ts'))).toBe(true);
  });
});

describe('import boundary contract', () => {
  const root = '/project/www';
  test('rejects engine imports outside engine', () => {
    expect(importBoundaryErrors('engine/runtime.ts', "import '../dungeon.ts'", root, new Set())).toHaveLength(1);
  });
  test('rejects visual imports from tests/support', () => {
    expect(importBoundaryErrors('demo.ts', "import './tests/helper.ts'", root, new Set(['demo.ts']))).toHaveLength(2);
  });
  test('allows engine-local, generated worker clients, and public visual imports', () => {
    expect(importBoundaryErrors('engine/runtime.ts', "import './frame.ts'", root, new Set())).toEqual([]);
    expect(importBoundaryErrors('engine/runtime.ts', "import '../workers/meshopt.client.ts'", root, new Set())).toEqual([]);
    expect(importBoundaryErrors('demo.ts', "import './engine/index.ts'", root, new Set(['demo.ts']))).toEqual([]);
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
      import { Thing } from '../../engine/core/private.ts';
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

  test('rejects renderer extension hooks and raw RPC worker assembly', () => {
    const findings = scanTypeScript(`
      import { MeshoptClient } from './meshopt.client.ts';
      const info = host.renderer.afterglowAdapterInfo;
      device.createShaderModule = compile;
      const rpc = await Rpc.create(options);
      const texture = new TextureClient(rpc);
      const meshopt = await MeshoptClient.spawnThreaded();
    `, 'hacks.ts');
    const rules = new Set(findings.map((finding) => finding.rule));
    expect(rules.has('AG-DEMO-005')).toBe(true);
    expect(rules.has('AG-DEMO-017')).toBe(true);
  });

  test('allows subsystem index barrels but rejects implementation modules', () => {
    const findings = scanTypeScript(`
      import { ModelPrimitives } from '../../engine/presentation/index.ts';
      import { VirtualGltfBinding } from '../../engine/virtual-texturing/index.ts';
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
