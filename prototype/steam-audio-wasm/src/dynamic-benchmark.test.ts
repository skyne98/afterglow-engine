import { describe, expect, test } from 'bun:test';

const root = new URL('../', import.meta.url);
const cpp = await Bun.file(new URL('dynamic-benchmark.cpp', root)).text();
const directCpp = await Bun.file(new URL('benchmark.cpp', root)).text();
const tracer = await Bun.file(new URL('../../../crates/afterglow-obvhs-tracer/src/lib.rs', import.meta.url)).text();
const build = await Bun.file(new URL('build.sh', root)).text();
const nativeBuild = await Bun.file(new URL('build-native.sh', root)).text();
const nativeRunner = await Bun.file(new URL('native-dynamic-benchmark.cpp', root)).text();
const bistroBuild = await Bun.file(new URL('build-native-bistro.sh', root)).text();
const bistroCooker = await Bun.file(new URL('cook-bistro-acoustic.cpp', root)).text();
const bistroRunner = await Bun.file(new URL('native-bistro-geometry-benchmark.cpp', root)).text();
const bistroWasm = await Bun.file(new URL('bistro-benchmark.cpp', root)).text();
const bistroWorker = await Bun.file(new URL('src/bistro-worker.ts', root)).text();
const bistroMain = await Bun.file(new URL('src/bistro-main.ts', root)).text();
const readme = await Bun.file(new URL('README.md', root)).text();
const audioWorker = await Bun.file(new URL('../../../crates/afterglow-audio-worker/src/lib.rs', import.meta.url)).text();
const audioHost = await Bun.file(new URL('../../../crates/afterglow-web/web/src/workers/audio-worker.ts', import.meta.url)).text();
const audioClient = await Bun.file(new URL('../../../crates/afterglow-web/web/src/workers/engineaudioservice.client.ts', import.meta.url)).text();
const audioWorklet = await Bun.file(new URL('../../../crates/afterglow-web/web/src/engine/audio/audio-worklet.ts', import.meta.url)).text();
const wasmAudioWorklet = await Bun.file(new URL('audio-worklet-gate.cpp', root)).text();
const evidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-wasm-dynamic-fox-laptop-2026-07-18.json', import.meta.url)).json();
const manySourceEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-wasm-many-sources-fox-laptop-2026-07-18.json', import.meta.url)).json();
const nativeEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-native-many-sources-fox-laptop-2026-07-18.json', import.meta.url)).json();
const bistroEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-native-bistro-fox-laptop-2026-07-18.json', import.meta.url)).json();
const bistroPackageEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-native-bistro-full-package-fox-laptop-2026-07-18.json', import.meta.url)).json();
const bistroEmbreeEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-native-bistro-embree-fox-laptop-2026-07-18.json', import.meta.url)).json();
const obvhsEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-wasm-obvhs-fox-laptop-2026-07-18.json', import.meta.url)).json();
const threadedObvhsEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-wasm-obvhs-simd-pthreads-fox-laptop-2026-07-18.json', import.meta.url)).json();
const bistroWasmEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-wasm-bistro-full-package-fox-laptop-2026-07-19.json', import.meta.url)).json();
const workletEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-wasm-audio-worklet-gate-fox-workstation-2026-07-19.json', import.meta.url)).json();
const unifiedWorkerEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-unified-worker-web-profile-fox-workstation-2026-07-19.json', import.meta.url)).json();
const realAssetEvidence = await Bun.file(new URL('../../../docs/benchmarks/steam-audio-real-assets-fox-workstation-2026-07-18.json', import.meta.url)).json();

describe('fully dynamic Steam Audio WASM prototype', () => {
  test('uses one unified Rust RPC service for the Worker to AudioWorklet gate', () => {
    expect(audioWorker).toContain('#[rpc(worker = EngineAudioWorker)]');
    expect(audioWorker).toContain('afterglow_audio_pump');
    expect(audioWorker).toContain('afterglow_steam_audio_render_quantum');
    expect(build).toContain('libafterglow_audio_worker.a');
    expect(build).toContain('engine-audio-rpc.js');
    expect(build).toContain('CARGO_BUILD_JOBS="$build_jobs"');
    expect(cpp).toContain('IPL_REFLECTIONEFFECTTYPE_HYBRID');
    expect(cpp).toContain('kMaxEngineReflectionVoiceCount = 64');
    expect(cpp).toContain('gSpatialDirectVoiceLimit = static_cast<int>(reflectionVoices)');
    expect(cpp).toContain('afterglow_steam_audio_register_sound');
    expect(cpp).toContain('residentSample(*resident, control.cursor + sample');
    expect(cpp).toContain('afterglow_steam_audio_load_acoustic_scene');
    expect(cpp).toContain('afterglow_obvhs_create_indexed');
    expect(cpp).toContain('gActiveReflectionEffectLimit.store(\n        static_cast<int>(reflectionVoices)');
    expect(cpp).toContain('afterglow_steam_audio_set_active_reflection_voices');
    expect(audioHost).toContain('afterglow_wasm_serve_frame');
    expect(audioHost).toContain('afterglow_audio_pump');
    expect(audioHost).toContain('afterglow_audio_simulate_motion');
    expect(audioHost).toContain('nextSimulationMs = performance.now() + 1_000');
    expect(audioClient).toContain('async crossfadeTo(');
    expect(audioClient).toContain('async setVoiceVolume(');
    expect(audioClient).toContain('async loadWav(data: Uint8Array, looped: boolean)');
    expect(audioClient).toContain('async beginWavUpload(');
    expect(audioClient).toContain('async beginAcousticSceneUpload(');
    expect(audioWorker).toContain('VoiceScheduler');
    expect(audioWorker).toContain('fn crossfade_to(');
    expect(audioWorklet).not.toContain('postMessage');
  });
  test('runs deadline DSP in an Emscripten real-time Wasm AudioWorklet gate', () => {
    expect(build).toContain('-sAUDIO_WORKLET=1 -sWASM_WORKERS=1');
    expect(wasmAudioWorklet).toContain('emscripten_start_wasm_audio_worklet_thread_async');
    expect(wasmAudioWorklet).toContain('afterglow_steam_audio_render_quantum()');
    expect(wasmAudioWorklet).toContain('processAudio');
    expect(wasmAudioWorklet).toContain('runSimulationWorker');
    expect(wasmAudioWorklet).toContain('gSimulationThread = std::thread');
    expect(cpp).toContain('publishSimulationSnapshot');
    expect(cpp).toContain('consumeSimulationSnapshot');
    expect(cpp).toContain('gSnapshotMiddleIndex.exchange');
    expect(wasmAudioWorklet).toContain('updateIndex % 5');
    expect(wasmAudioWorklet).toContain('std::chrono::milliseconds(200)');
    const renderQuantum = cpp.slice(
      cpp.indexOf('int afterglow_steam_audio_render_quantum()'),
      cpp.indexOf('const float* afterglow_steam_audio_pcm_ptr()'),
    );
    expect(renderQuantum).toContain('consumeSimulationSnapshot()');
    expect(renderQuantum).not.toContain('gOutputs[');
    expect(renderQuantum).not.toContain('gSourceInputs[');
    expect(renderQuantum).not.toContain('gSharedInputs.listener');
    expect(wasmAudioWorklet).not.toMatch(/new |malloc|postMessage/);
  });

  test('ties the selected unified Worker profile to device and render evidence', () => {
    expect(unifiedWorkerEvidence.architecture.voiceCapacity).toBe(16);
    expect(unifiedWorkerEvidence.architecture.completeWorldPhysicalVoiceCapacity).toBe(4);
    expect(unifiedWorkerEvidence.architecture.transport).toContain('eight-quantum');
    expect(unifiedWorkerEvidence.run.callbacks).toBeGreaterThan(30_000);
    expect(unifiedWorkerEvidence.run.simulationUpdates).toBeGreaterThan(100);
    expect(unifiedWorkerEvidence.run.underruns).toBe(0);
    expect(unifiedWorkerEvidence.run.sequenceErrors).toBe(0);
    expect(unifiedWorkerEvidence.run.pumpOverBudget).toBe(0);
    expect(unifiedWorkerEvidence.voiceSchedulerValidation.activeWorldPhysicalVoices).toBe(4);
    expect(unifiedWorkerEvidence.voiceSchedulerValidation.completedVoiceFades).toBe(1);
    expect(unifiedWorkerEvidence.voiceSchedulerValidation.underruns).toBe(0);
    expect(unifiedWorkerEvidence.voiceSchedulerValidation.physicalMonitorWaveform.longestInternalZeroMs)
      .toBeLessThan(1);
    expect(unifiedWorkerEvidence.residentSoundValidation.loadedResidentSounds).toBe(1);
    expect(unifiedWorkerEvidence.residentSoundValidation.residentSoundBytes).toBe(38_400);
    expect(unifiedWorkerEvidence.residentSoundValidation.underruns).toBe(0);
    expect(unifiedWorkerEvidence.residentSoundValidation.physicalMonitorWaveform.longestInternalZeroMs)
      .toBeLessThan(1);
    expect(unifiedWorkerEvidence.conclusion.selectedProductionCandidate).toBe(true);
  });

  test('selects render-ahead depth using coupled real sounds and real environments', () => {
    expect(realAssetEvidence.assets.sounds.files).toHaveLength(5);
    expect(realAssetEvidence.assets.environments.scenes).toHaveLength(3);
    expect(realAssetEvidence.nativeRejectedFourQuanta.every((run: { underruns: number }) => run.underruns > 0)).toBe(true);
    expect(realAssetEvidence.nativeAcceptedEightQuanta.every((run: { underruns: number; sequenceErrors: number; longestZeroMs: number }) =>
      run.underruns === 0 && run.sequenceErrors === 0 && run.longestZeroMs === 0)).toBe(true);
    expect(realAssetEvidence.webRealSoundSet.underruns).toBe(0);
    expect(realAssetEvidence.conclusion.selectedNativeRenderAheadQuanta).toBe(8);
  });

  test('ties concurrent AudioWorklet admission to physical and render evidence', () => {
    const accepted = workletEvidence.runs.find(
      (run: { result: string }) => run.result === 'pass-concurrent-simulation-short-gate',
    );
    expect(accepted.activeReflectionVoices).toBe(16);
    expect(accepted.measuredCallbacks).toBeGreaterThan(20_000);
    expect(accepted.steadyCallbacksOver2_667Ms).toBe(0);
    expect(accepted.callbackErrors).toBe(0);
    expect(accepted.concurrentSimulation.errors).toBe(0);
    expect(accepted.steadyRafP99Ms).toBeLessThan(8.333);
    expect(accepted.physicalMonitorWaveform.longestInternalZeroMs).toBeLessThan(1);
  });

  test('uses the allocation-free obvhs custom scene without baked data', () => {
    expect(cpp).toContain('input.baked = IPL_FALSE');
    expect(cpp).toContain('IPL_SCENETYPE_CUSTOM');
    expect(cpp).toContain('afterglow_obvhs_batched_closest_hit');
    expect(cpp).toContain('afterglow_obvhs_set_door_y');
    expect(cpp).toContain('iplSimulatorRunReflections');
    expect(cpp).toContain("sourceCount > 128");
    expect(cpp).toContain("rays > gMaxRays");
    expect(build).toContain('libafterglow_obvhs_tracer.a');
    expect(build).toContain('target-feature=+simd128');
  });

  test('implements all custom callbacks with fixed-stack query traversal', () => {
    expect(directCpp).toContain('IPL_SCENETYPE_CUSTOM');
    expect(tracer).toContain('pub unsafe extern "C" fn afterglow_obvhs_closest_hit');
    expect(tracer).toContain('pub unsafe extern "C" fn afterglow_obvhs_any_hit');
    expect(tracer).toContain('pub unsafe extern "C" fn afterglow_obvhs_batched_closest_hit');
    expect(tracer).toContain('pub unsafe extern "C" fn afterglow_obvhs_batched_any_hit');
    expect(tracer).toContain('callbacks_allocate_nothing_after_build');
    expect(tracer).toContain('shared_triangle_edge_has_no_acoustic_crack');
    expect(tracer).toContain('core::arch::wasm32');
    expect(tracer).toContain('f32x4_le');
    expect(tracer).toContain('afterglow_obvhs_traversal_lanes');
  });

  test('records the scalar obvhs baseline and accepts SIMD pthreads', () => {
    expect(obvhsEvidence.runs).toHaveLength(5);
    expect(threadedObvhsEvidence.runs).toHaveLength(5);
    expect(threadedObvhsEvidence.aggregate.simulationWorstP99Ms).toBeLessThan(16.667);
    expect(threadedObvhsEvidence.aggregate.launchesOver16_667MsP99).toBe(0);
    expect(threadedObvhsEvidence.aggregate.simulationThreads).toBe(2);
    expect(threadedObvhsEvidence.aggregate.tracerLanes).toBe(4);
    expect(threadedObvhsEvidence.aggregate.allIrValid).toBe(true);
    expect(threadedObvhsEvidence.aggregate.allOutputEnergyNonzero).toBe(true);
    expect(threadedObvhsEvidence.nativeSameConfiguration.runs).toHaveLength(5);
    expect(threadedObvhsEvidence.nativeSameConfiguration.simulationWorstP99Ms)
      .toBeLessThan(threadedObvhsEvidence.aggregate.simulationWorstP99Ms);
    expect(threadedObvhsEvidence.conclusion.accepted).toBe(true);
  });

  test('feeds generated reflections through Steam Audio DSP', () => {
    expect(cpp).toContain('iplSourceGetOutputs');
    expect(cpp).toContain('iplReflectionEffectApply');
    expect(cpp).toContain('iplBinauralEffectApply');
  });

  test('rebuilds every dynamic archive for a fixed pthread pool', () => {
    expect(build).toContain('steam-audio-threaded');
    expect(build).toContain('-sPTHREAD_POOL_SIZE=2');
    expect(build).toContain('-sPTHREAD_POOL_SIZE_STRICT=2');
    expect(build).toContain('pthread-deps');
    expect(build).toContain('target-feature=+atomics,+bulk-memory,+mutable-globals,+simd128');
    expect(build).toContain("grep -q '4\\.0\\.23'");
    expect(build).toContain('375b1431b');
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

  test('runs the same dynamic workload in a real native worker', () => {
    expect(nativeBuild).toContain('steamaudio/lib/linux-x64/libphonon.so');
    expect(nativeBuild).toContain('-march=x86-64-v3');
    expect(nativeRunner).toContain('std::thread worker');
    expect(nativeRunner).toContain('dyn_set_simulation_threads');
    expect(nativeRunner).toContain('onlyScenario');
    expect(nativeRunner).toContain('"c64-512x2-o1"');
    expect(cpp).toContain('simulationSettings.numThreads = gSimulationThreads');
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

  test('cooks the official Bistro scene into runtime acoustic geometry', () => {
    expect(bistroBuild).toContain('https://developer.nvidia.com/bistro');
    expect(bistroBuild).toContain('BistroExterior.fbx');
    expect(bistroBuild).toContain('BistroInterior.fbx');
    expect(bistroBuild).toContain('BistroInterior_Wine.fbx');
    expect(bistroCooker).toContain('aiProcess_PreTransformVertices');
    expect(bistroCooker).toContain('acousticCategory');
    expect(bistroRunner).toContain('inputs[index].baked = IPL_FALSE');
    expect(bistroRunner).toContain('iplStaticMeshCreate');
    expect(bistroRunner).toContain('IPL_SCENETYPE_EMBREE');
    expect(bistroRunner).toContain('iplEmbreeDeviceCreate');
    expect(bistroRunner).toContain('std::thread worker');
  });

  test('ties the native worker policy to five-launch thread sweeps', () => {
    expect(nativeEvidence.threadSets).toHaveLength(3);
    expect(nativeEvidence.threadSets.map((value: { steamAudioSimulationThreads: number }) =>
      value.steamAudioSimulationThreads)).toEqual([1, 2, 4]);
    expect(nativeEvidence.threadSets.every((value: { runs: unknown[] }) => value.runs.length === 5)).toBe(true);
    expect(nativeEvidence.selected.balanced.simulationWorstP99Ms).toBeCloseTo(10.7421, 3);
    expect(nativeEvidence.selected.balanced.projectedReflection64PlusHrtf128QuantumMs)
      .toBeCloseTo(1.4334, 3);
    expect(nativeEvidence.selected.highSimulationQuality.simulationWorstP99Ms)
      .toBeCloseTo(12.5247, 3);
    expect(readme).toContain('16.48 → 9.27 → 5.53 ms');
  });

  test('records real million-triangle Bistro geometry scaling and attribution', () => {
    expect(bistroEvidence.asset.triangles).toBe(1_046_609);
    expect(bistroEvidence.asset.license).toBe('CC-BY 4.0');
    expect(bistroEvidence.threadSets).toHaveLength(3);
    expect(bistroEvidence.threadSets.every((value: { runs: unknown[] }) => value.runs.length === 5)).toBe(true);
    expect(bistroEvidence.conclusion.recommendedSimulationWorstP99Ms).toBeCloseTo(25.5619, 3);
    expect(bistroEvidence.conclusion.eightThread512WorstP99Ms).toBeCloseTo(17.5229, 3);
    expect(bistroEvidence.threadSets.every((set: { aggregate: { scenarios: Array<{ allIrValid: boolean }> } }) =>
      set.aggregate.scenarios.every(scenario => scenario.allIrValid))).toBe(true);
    expect(readme).toContain('Amazon Lumberyard Bistro, Open Research Content Archive (ORCA)');
  });

  test('covers every distinct scene in the full Bistro package', () => {
    expect(bistroPackageEvidence.assets.map((asset: { name: string }) => asset.name))
      .toEqual(['BistroExterior', 'BistroInterior', 'BistroInterior_Wine']);
    expect(bistroPackageEvidence.assets.map((asset: { triangles: number }) => asset.triangles))
      .toEqual([2_832_120, 1_046_609, 1_320_323]);
    expect(bistroPackageEvidence.sceneThreadSets).toHaveLength(6);
    expect(bistroPackageEvidence.sceneThreadSets.every((value: { runs: unknown[] }) => value.runs.length === 5)).toBe(true);
    const exteriorFour = bistroPackageEvidence.sceneThreadSets.find((value: {
      scene: string; steamAudioSimulationThreads: number;
    }) => value.scene === 'BistroExterior' && value.steamAudioSimulationThreads === 4);
    const exteriorEight = bistroPackageEvidence.sceneThreadSets.find((value: {
      scene: string; steamAudioSimulationThreads: number;
    }) => value.scene === 'BistroExterior' && value.steamAudioSimulationThreads === 8);
    expect(exteriorFour.aggregate.scenarios[0].simulationWorstP99Ms).toBeCloseTo(34.8201, 3);
    expect(exteriorEight.aggregate.scenarios[0].simulationWorstP99Ms).toBeCloseTo(24.2426, 3);
    expect(bistroPackageEvidence.package.note).toContain('intentionally not merged');
    expect(readme).toContain('## Full Amazon Lumberyard Bistro package');
  });

  test('runs the complete full-resolution Bistro package in browser WASM', () => {
    expect(tracer).toContain('afterglow_obvhs_create_indexed');
    expect(bistroWasm).toContain('afterglow_obvhs_create_indexed');
    expect(bistroWasm).toContain('settings.rayBatchSize = 64');
    expect(bistroWorker).toContain('writeFrame(rings, RING_BYTES, response)');
    expect(bistroMain).toContain('FAILED:');
    expect(build).toContain('-sINITIAL_MEMORY=1610612736');
    expect(bistroWasmEvidence.runs).toHaveLength(15);
    expect(bistroWasmEvidence.aggregates.map((value: { triangles: number }) => value.triangles))
      .toEqual([2_832_120, 1_046_609, 1_320_323]);
    expect(bistroWasmEvidence.acceptance.allIrValid).toBe(true);
    expect(bistroWasmEvidence.acceptance.allRuntimeTelemetry).toBe(true);
    expect(bistroWasmEvidence.acceptance.ray512Fits60HzAllScenes).toBe(false);
    expect(bistroWasmEvidence.acceptance.ray512Fits30HzAllScenes).toBe(true);
    expect(bistroWasmEvidence.acceptance.packageWorst512P99Ms).toBeCloseTo(27.88, 2);
  });

  test('uses Embree to keep every full Bistro scene inside 60 Hz', () => {
    expect(bistroEmbreeEvidence.method.rayTracer).toContain('IPL_SCENETYPE_EMBREE');
    expect(bistroEmbreeEvidence.sceneThreadSets).toHaveLength(9);
    expect(bistroEmbreeEvidence.sceneThreadSets.every((value: { runs: unknown[] }) => value.runs.length === 5)).toBe(true);
    expect(bistroEmbreeEvidence.sceneThreadSets.every((set: {
      aggregate: { scenarios: Array<{ simulationWorstP99Ms: number; allIrValid: boolean }> };
    }) => set.aggregate.scenarios.every(scenario =>
      scenario.simulationWorstP99Ms < 1000 / 60 && scenario.allIrValid))).toBe(true);
    expect(bistroEmbreeEvidence.conclusion.packageWorstTwoThread512P99Ms).toBeCloseTo(3.89501, 4);
    expect(bistroEmbreeEvidence.conclusion.packageWorstTwoThread1024P99Ms).toBeCloseTo(6.93063, 4);
    expect(bistroEmbreeEvidence.conclusion.exteriorBuildMeanMs).toBeCloseTo(519.603, 2);
    expect(readme).toContain('Embree improved mean simulation time by **18.8–22.9×**');
  });
});
