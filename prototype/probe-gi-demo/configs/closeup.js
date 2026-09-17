// Sweep of near views over the cube's shadowed side (+X/+Z faces face the light early
// on, so the -X/-Z faces receive probe GI only, where cage quantization is visible).
globalThis.__probeGiConfig = { cameras: [
  { p: [-1.2, 1.4, -1.2], yaw: 0.785, pitch: -0.23 },
  { p: [-2.0, 1.5, -2.0], yaw: 0.785, pitch: -0.20 },
  { p: [-3.2, 1.9, -3.2], yaw: 0.785, pitch: -0.20 },
] };
await import('../demo.js');
