import { describe, expect, test } from 'bun:test';

const root = new URL('../', import.meta.url);
const cpp = await Bun.file(new URL('dynamic-benchmark.cpp', root)).text();
const build = await Bun.file(new URL('build.sh', root)).text();
const threadPool = await Bun.file(new URL('steam-audio-wasm-thread-pool.cpp', root)).text();
const readme = await Bun.file(new URL('README.md', root)).text();
const evidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-wasm-dynamic-fox-laptop-2026-07-18.json', import.meta.url)).json();
const manySourceEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-wasm-many-sources-fox-laptop-2026-07-18.json', import.meta.url)).json();

describe('fully dynamic Steam Audio WASM prototype', () => {
  test('uses runtime geometry and explicitly disables baked reflection data', () => {
    expect(cpp).toContain('input.baked = IPL_FALSE');
    expect(cpp).toContain('iplInstancedMeshUpdateTransform');
    expect(cpp).toContain('iplSceneCommit(gScene)');
    expect(cpp).toContain('iplSimulatorRunReflections');
    expect(cpp).toContain("sourceCount > 128");
    expect(cpp).toContain("rays > gMaxRays");
  });

  test('feeds generated reflections through Steam Audio DSP', () => {
    expect(cpp).toContain('iplSourceGetOutputs');
    expect(cpp).toContain('iplReflectionEffectApply');
    expect(cpp).toContain('iplBinauralEffectApply');
  });

  test('rebuilds the pinned library without nested browser workers', () => {
    expect(build).toContain('steam-audio-wasm-thread-pool.cpp');
    expect(build).toContain('steam-audio-threadless');
    expect(build).toContain("grep -q '4\\.0\\.23'");
    expect(threadPool).toContain('#if defined(IPL_OS_WASM)');
    expect(threadPool).toContain('processNextJob(0, mCancel)');
  });

  test('keeps published headline measurements tied to raw evidence', () => {
    const aggregate = evidence.dynamic.aggregate as Array<{ name: string; simulationWorstP99Ms: number }>;
    const low = aggregate.find(value => value.name === 'parametric-low');
    const medium = aggregate.find(value => value.name === 'parametric-medium');
    expect(low?.simulationWorstP99Ms).toBeCloseTo(1.705, 3);
    expect(medium?.simulationWorstP99Ms).toBeCloseTo(10.955, 3);
    expect(readme).toContain('| Parametric low | 1 | 1,024 × 2 | 1.02 ms | 1.70 ms |');
    expect(readme).toContain('| Parametric medium | 1 | 4,096 × 4 | 7.82 ms | 10.95 ms |');
  });

  test('ties the selected many-source configuration to unlocked raw evidence', () => {
    const aggregate = manySourceEvidence.aggregate as Array<{
      name: string; simulationWorstP99Ms: number; combinedAudioQuantumMeanMs: number;
    }>;
    const selected = aggregate.find(value => value.name === 'p64-512x2');
    expect(manySourceEvidence.host.session).toContain('unlocked');
    expect(selected?.simulationWorstP99Ms).toBeCloseTo(15.25, 2);
    expect(selected?.combinedAudioQuantumMeanMs).toBeCloseTo(1.2145, 3);
    expect(manySourceEvidence.selected.recommended.audibleDirectHrtfSources).toBe(128);
    expect(manySourceEvidence.selected.recommended.independentlyReflectedSources).toBe(64);
    expect(manySourceEvidence.selected.recommended.projectedReflection64PlusHrtf128QuantumMs)
      .toBeCloseTo(1.3642, 3);
    expect(readme).toContain('**64 priority sources**');
  });
});
