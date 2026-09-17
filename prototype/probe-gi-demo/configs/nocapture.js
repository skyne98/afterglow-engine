// Soak without the automated capture set: isolates capture cost from render cost.
globalThis.__probeGiConfig = { capture: false };
await import('../demo.js');
