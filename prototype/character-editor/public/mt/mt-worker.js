postMessage({ step: 'start' });
try {
  const mod = await import('./omtprobe.js');
  postMessage({ step: 'imported', hasDefault: !!mod.default });
  const instance = await mod.default({ locateFile: (p) => './' + p });
  postMessage({ step: 'created', hasRunOmp: typeof instance._run_omp });
  const n = instance._run_omp();
  postMessage({ step: 'done', threads: n, xoi: crossOriginIsolated, cores: navigator.hardwareConcurrency });
} catch (e) {
  postMessage({ step: 'error', error: e.message, stack: e.stack });
}
