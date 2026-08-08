import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { countBundledThreeCoreCopies, validateWebContracts } from './check-web-contracts.ts';
import { validateConvergenceDeletions } from './check-convergence-deletions.ts';
import { scanTypeScript } from './lint-demo-architecture.ts';
import { importBoundaryErrors } from './lint-import-boundaries.ts';

const repositoryRoot = new URL('../', import.meta.url);
const agents = await Bun.file(new URL('AGENTS.md', repositoryRoot)).text();
const audioPlan = await Bun.file(new URL(
  'docs/implementation/spatial-audio-integration-plan.md', repositoryRoot,
)).text();

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

  test('accepts canonical source architecture while the release converges', async () => {
    expect(await validateWebContracts(await fixture({ releaseStatus: 'converging' }))).toEqual([]);
  });

  test('rejects false conformant release claims', async () => {
    const errors = await validateWebContracts(await fixture({
      releaseStatus: 'conformant', conformance: { 'src/demo.ts': 'legacy' },
    }));
    expect(errors.some((error) => error.includes('conformant release has legacy demos'))).toBe(true);
  });

  test('rejects obsolete migration status', async () => {
    const errors = await validateWebContracts(await fixture({ releaseStatus: 'migration' }));
    expect(errors.some((error) => error.includes('invalid releaseStatus migration'))).toBe(true);
  });

  test('rejects stale conformance entries', async () => {
    const errors = await validateWebContracts(await fixture({
      releaseStatus: 'converging', conformance: { 'src/demo.ts': 'legacy', 'gone.ts': 'legacy' },
    }));
    expect(errors.some((error) => error.includes('stale/non-visual entrypoint gone.ts'))).toBe(true);
  });
});

describe('convergence deletion ledger', () => {
  async function ledgerFixture(status: 'pending' | 'removed', source: string): Promise<string> {
    const root = await mkdtemp(join(tmpdir(), 'afterglow-deletions-'));
    temporary.push(root);
    const contracts = join(root, 'crates/afterglow-web/web/contracts');
    const engine = join(root, 'crates/afterglow-web/web/src/engine');
    await mkdir(contracts, { recursive: true });
    await mkdir(engine, { recursive: true });
    await writeFile(join(engine, 'legacy.ts'), source);
    await writeFile(join(contracts, 'convergence-deletions.json'), JSON.stringify({
      version: 1,
      items: [{
        id: 'CUE-DEL-001', status, description: 'legacy symbol',
        roots: ['crates/afterglow-web/web/src/engine'], literals: ['legacyPath'],
      }],
    }));
    return root;
  }

  test('keeps pending debt visible', async () => {
    expect(await validateConvergenceDeletions(await ledgerFixture('pending', 'legacyPath();'))).toEqual([]);
  });

  test('rejects reintroduction after removal', async () => {
    const errors = await validateConvergenceDeletions(await ledgerFixture('removed', 'legacyPath();'));
    expect(errors.some((error) => error.includes('removed symbol/path was reintroduced'))).toBe(true);
  });

  test('requires an absent pending item to ratchet to removed', async () => {
    const errors = await validateConvergenceDeletions(await ledgerFixture('pending', 'export {};'));
    expect(errors.some((error) => error.includes('mark it removed'))).toBe(true);
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

describe('runtime readiness boundary contract', () => {
  test('keeps strict engine readiness separate from compatibility first-present readiness', async () => {
    const runtime = await Bun.file(new URL(
      'crates/afterglow-web/web/src/engine/core/runtime.ts', repositoryRoot,
    )).text();
    const shell = await Bun.file(new URL(
      'crates/afterglow-shell/src/main.rs', repositoryRoot,
    )).text();
    expect(runtime).toContain('RuntimeReadinessStage.GameReady');
    expect(runtime).toContain('op_afterglow_game_ready');
    expect(shell).toContain('fn op_afterglow_game_ready');
    expect(shell).toContain('self.compatibility_mode && !self.ready.load');
    expect(shell).toContain('--compat-three requires an HTML example path');
    expect(shell).not.toContain('if self.official_example && !self.ready.load');
  });

  test('keeps native capture diagnostic-only and copies the final composited surface', async () => {
    const shell = await Bun.file(new URL(
      'crates/afterglow-shell/src/main.rs', repositoryRoot,
    )).text();
    const canvas = await Bun.file(new URL(
      'crates/afterglow-shell/vendor/deno_webgpu/canvas.rs', repositoryRoot,
    )).text();
    expect(shell).toContain('AFTERGLOW_CAPTURE_PATH');
    expect(shell).toContain('Afterglow diagnostic surface capture');
    expect(shell).toContain('copy_texture_to_buffer');
    expect(shell).toContain('AFTERGLOW_CAPTURE_READY_FRAMES');
    expect(canvas).toContain('usage: usage | wgpu_types::TextureUsages::COPY_SRC');
  });

  test('ships one diagnostic protocol and no production demo globals', async () => {
    const protocol = await Bun.file(new URL(
      'crates/afterglow-web/web/src/engine/diagnostics/visual-protocol.ts', repositoryRoot,
    )).text();
    expect(protocol).toContain("'__afterglowDiagnosticV1'");
    for (const path of [
      'dungeon/main.ts', 'rigged-vt/main.ts', 'vt/main.ts', 'engine/main.ts', 'lod/main.ts',
    ]) {
      const source = await Bun.file(new URL(
        `crates/afterglow-web/web/src/demos/${path}`, repositoryRoot,
      )).text();
      expect(source).not.toContain('__afterglow');
      expect(source).not.toContain('publishDevHarness');
      expect(source).not.toContain('FrameStepHarness');
    }
  });
});

describe('native asset/worker boundary contract', () => {
  test('keeps concrete service composition out of the generic shell bridge', async () => {
    const bridge = await Bun.file(new URL(
      'crates/afterglow-shell/src/rpc_bridge.rs', repositoryRoot,
    )).text();
    const production = bridge.split('#[cfg(test)]')[0]!;
    expect(production).not.toContain('use afterglow_texture');
    expect(production).not.toContain('use afterglow_meshopt');
    expect(production).not.toContain('op_afterglow_arena_view');
    expect(production).toContain('register_named_async');
  });

  test('keeps public-web OPFS payloads inside the shared-ring Worker', async () => {
    const platform = await Bun.file(new URL(
      'crates/afterglow-web/web/src/engine/streaming/platform-persistent-blob-store.ts', repositoryRoot,
    )).text();
    const backend = await Bun.file(new URL(
      'crates/afterglow-web/web/src/engine/streaming/web-persistent-blob-backend.ts', repositoryRoot,
    )).text();
    const worker = await Bun.file(new URL(
      'crates/afterglow-web/web/src/workers/storage-worker.ts', repositoryRoot,
    )).text();
    expect(platform).toContain('WebPersistentBlobBackend.open');
    expect(platform).not.toContain('OpfsPersistentBlobBackend');
    expect(backend).toContain("workerJsUrl: 'storage-worker.js'");
    expect(backend).toContain('BlobStorageClient.spawnThreaded');
    expect(worker).toContain("self.postMessage('wake')");
    expect(worker).not.toMatch(/postMessage\([^)]*(bytes|responseData|args)/);
  });

  test('publishes worker manifests instead of TypeScript worker ids', async () => {
    const workers = await Bun.file(new URL(
      'crates/afterglow-web/web/src/engine/assets/platform-workers.ts', repositoryRoot,
    )).text();
    expect(workers).toContain("nativeWorkerIds('texture')");
    expect(workers).toContain("nativeWorkerIds('meshopt')");
    const storage = await Bun.file(new URL(
      'crates/afterglow-web/web/src/engine/streaming/native-persistent-blob-backend.ts', repositoryRoot,
    )).text();
    expect(storage).toContain("op_afterglow_worker_ids('storage')");
    expect(storage).toContain('NativeRpcTransport');
    expect(workers).not.toContain('NATIVE_TEXTURE_WORKER_FIRST');
    expect(workers).not.toContain('NATIVE_MESHOPT_WORKER');
  });

  test('keeps native physical-core topology in bootstrap instead of demos', async () => {
    const shell = await Bun.file(new URL(
      'crates/afterglow-shell/src/main.rs', repositoryRoot,
    )).text();
    expect(shell).toContain('num_cpus::get_physical()');
    expect(shell).toContain('NATIVE_TEXTURE_WORKER_CAP: usize = 16');
    expect(shell).toContain('builder::register_async_worker');
    expect(shell).toContain('"storage"');
    expect(shell).toContain('BlobStorageWorker::default()');
    expect(shell).not.toContain('registry.register_named_async');
    for (const path of [
      'crates/afterglow-web/web/src/demos/dungeon/main.ts',
      'crates/afterglow-web/web/src/demos/rigged-vt/main.ts',
    ]) {
      const demo = await Bun.file(new URL(path, repositoryRoot)).text();
      expect(demo).not.toContain('navigator.hardwareConcurrency');
      expect(demo).not.toMatch(/\bworkerCount\s*:/);
      expect(demo).not.toContain('createVirtualTextureStore');
      expect(demo).toContain('createVirtualTextureSystem');
    }
  });

  test('uses the decomposed asset API with no legacy session wrapper', async () => {
    const barrel = await Bun.file(new URL(
      'crates/afterglow-web/web/src/engine/assets/index.ts', repositoryRoot,
    )).text();
    expect(barrel).toContain('EngineAssets');
    expect(barrel).toContain('BigContainer');
    expect(barrel).not.toContain('BigAssetSession');
    expect(await Bun.file(new URL(
      'crates/afterglow-web/web/src/engine/assets/big-asset-session.ts', repositoryRoot,
    )).exists()).toBe(false);
    expect(await Bun.file(new URL(
      'crates/afterglow-web/web/src/engine/assets/big-parser.ts', repositoryRoot,
    )).exists()).toBe(false);
  });

  test('keeps VT mechanisms decomposed with one container index and ownership path', async () => {
    const root = 'crates/afterglow-web/web/src/engine/';
    const engineAssets = await Bun.file(new URL(
      `${root}assets/engine-assets.ts`, repositoryRoot,
    )).text();
    const provider = await Bun.file(new URL(
      `${root}assets/vt-page-provider.ts`, repositoryRoot,
    )).text();
    const sortedReader = await Bun.file(new URL(
      `${root}assets/source-sorted-page-reader.ts`, repositoryRoot,
    )).text();
    const store = await Bun.file(new URL(
      `${root}virtual-texturing/virtual-texture.ts`, repositoryRoot,
    )).text();
    expect(engineAssets).not.toMatch(/\n\s*get (source|containerPath|header|rawAssets)\(/);
    expect(engineAssets).not.toContain('poll(): void {}');
    expect(provider).toContain('new VtPageDirectory(header)');
    expect(sortedReader).toContain('new VtPageDirectory(header)');
    expect(store).not.toContain('private loader:');
    expect(provider.split('\n').length).toBeLessThan(250);
    expect(store.split('\n').length).toBeLessThan(1_700);
  });

  test('shares bounded handles across mutable textures and deformation-aware model LODs', async () => {
    const root = 'crates/afterglow-web/web/src/engine/';
    const vtBarrel = await Bun.file(new URL(
      `${root}virtual-texturing/index.ts`, repositoryRoot,
    )).text();
    const modelBarrel = await Bun.file(new URL(
      `${root}presentation/index.ts`, repositoryRoot,
    )).text();
    const modelLod = await Bun.file(new URL(
      `${root}presentation/model-lod.ts`, repositoryRoot,
    )).text();
    const textureNodes = await Bun.file(new URL(
      `${root}virtual-texturing/virtual-texture-nodes.ts`, repositoryRoot,
    )).text();
    const assetStore = await Bun.file(new URL(
      `${root}assets/asset-store.ts`, repositoryRoot,
    )).text();
    expect(vtBarrel).toContain('VirtualTextureSystem');
    expect(vtBarrel).toContain('MemoryVirtualTextureSource');
    expect(vtBarrel).toContain('virtualTextureNode');
    expect(modelBarrel).toContain('ModelSystem');
    expect(modelBarrel).toContain('ModelLodBinding');
    expect(modelLod).toContain('simplifyWithAttributes');
    expect(modelLod).toContain("geometry.morphAttributes");
    expect(textureNodes).toContain('readonly rawValue');
    expect(textureNodes).toContain('sRGBTransferEOTF');
    expect(vtBarrel).toContain('MemoryTexturePersistenceStatus');
    const geometryArena = await Bun.file(new URL(
      `${root}presentation/geometry-arena.ts`, repositoryRoot,
    )).text();
    expect(geometryArena).toContain('class GeometryArena');
    expect(geometryArena).toContain('activeGpuBytes');
    expect(assetStore).not.toContain('loadModel(path:');
    expect(await Bun.file(new URL(
      `${root}presentation/static-lod.ts`, repositoryRoot,
    )).exists()).toBe(false);
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
