// Leak test: the enclosed room with its doorway sealed, so nothing can carry light in.
globalThis.__probeGiConfig = { scene: 'complex', seal: true };
await import('../demo.js');
