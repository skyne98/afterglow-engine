// Diagnostic: indirect irradiance only (the probe contribution, tonemapped for display).
globalThis.__probeGiConfig = { scene: 'complex', seal: true, debug: 2 };
await import('../demo.js');
