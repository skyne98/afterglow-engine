// Reference for the cascade-* runs: the same pose and light with the real blended result.
globalThis.__probeGiConfig = {
  scene: 'complex', staticLight: true,
  cameras: Array.from({ length: 3 }, () => ({ p: [0.0, 1.6, -3.0], yaw: Math.PI * 0.5, pitch: 0.0 })),
};
await import('../demo.js');
