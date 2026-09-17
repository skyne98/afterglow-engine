// Validation entry: the enclosed room with the camera orbiting inside it, so the
// probe field scrolls every frame and relocation runs against walls continuously.
globalThis.__probeGiConfig = { scene: 'complex', motion: 'orbit' };
await import('../demo.js');
