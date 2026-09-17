// Scroll-only temporal test: static camera, static light, and the SCROLL ORIGIN drifting at
// 4 m/s (CFG.scrollDrift), so a wrapped band + the one-frame cascade-origin lag happen with
// zero parallax. Consecutive captures then differ by the probe-field change alone, which is
// the only way a one-frame scroll event shows up in a still image.
globalThis.__probeGiConfig = {
  scene: 'complex',
  cameras: Array.from({ length: 3 }, () => ({ p: [0.0, 1.6, -3.0], yaw: Math.PI * 0.5, pitch: 0.0 })),
  staticLight: true,
  scrollDrift: 4.0,
  fieldTrace: true,
  captureFrames: Array.from({ length: 25 }, (_, i) => 40 + i),
};
await import('../demo.js');
