// Diagnostic: exclude one class of outside probes from the cage (see render.wgsl.js).
globalThis.__probeGiConfig = { scene: 'complex', seal: true, debug: 8 };
await import('../demo.js');
