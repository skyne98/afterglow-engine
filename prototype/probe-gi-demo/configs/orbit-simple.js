// Moving camera in the simple scene: every 1 m the field scrolls and a band of probes is
// reseeded, which is the case a static capture never exercises.
globalThis.__probeGiConfig = { motion: 'orbit' };
await import('../demo.js');
