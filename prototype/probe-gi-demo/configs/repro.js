// Repro of the reported artifact: the camera pose read off the reporter's HUD
// (pos 3.74 2.34 3.12). Blocks appear on the cube and as a rectangular region on the
// ground when zoomed in.
globalThis.__probeGiConfig = { cameras: [
  { p: [3.74, 2.34, 3.12], yaw: -2.266060, pitch: -0.268482 },
  { p: [3.74, 2.34, 3.12], yaw: -2.266060, pitch: -0.268482 },
  { p: [3.74, 2.34, 3.12], yaw: -2.266060, pitch: -0.268482 },
] };
await import('../demo.js');
