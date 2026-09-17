// Timing split: probe updates off, render only.
globalThis.__probeGiConfig = { capture: false, probeUpdates: false };
await import('../demo.js');
