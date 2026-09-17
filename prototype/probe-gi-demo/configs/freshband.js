// Forces a reseeded band every frame while the light animates, reproducing the scroll
// state a flying camera creates. Used to test that the band is not visible as a step.
globalThis.__probeGiConfig = { forceFreshBand: true, cameras: [
  { p: [3.74, 2.34, 3.12], yaw: Math.PI * 1.25, pitch: -0.28 },
  { p: [3.74, 2.34, 3.12], yaw: Math.PI * 1.25, pitch: -0.28 },
  { p: [3.74, 2.34, 3.12], yaw: Math.PI * 1.25, pitch: -0.28 },
] };
await import('../demo.js');
