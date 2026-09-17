// Diagnostic: only probes inside the room's AABB contribute to the cage.
globalThis.__probeGiConfig = { scene: 'complex', seal: true, debug: 6 };
await import('../demo.js');
